//! 边缘 HTTP/WS（docs/19 §3）：路径布局、边缘鉴权（服务器级设备）、cookie 路由、
//! 会话反代（含 101 透传）、治理挂载、配对/舰队静态页与指标。
//!
//! 手机常驻连接纪律（§3.4）：1×WS（remote.mux 会话）+ 1×HTTP(H2)——本模块就是那条 HTTP
//! 与会话代理的落点；控制面 `/api/*` 是统一端点的 HTTP 投影（§4.5）。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Query, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::json;

use crate::db::DeviceRow;
use crate::governance::{Class, Guard, Reject};
use crate::hub::{self, Hub};
use crate::transport;
use crate::tunnel;

/// 请求体上限（与反代层 client_max_body_size 对齐，docs/19 §5.3）。
const MAX_BODY: usize = 16 * 1024 * 1024;

/// 转发前剥离的逐跳/伪造头（docs/19 §4.3）。
const STRIP_HEADERS: &[&str] = &[
    "host",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "transfer-encoding",
    "x-qx-device",
    "x-qx-edge",
];

pub struct EdgeState {
    pub hub: Arc<Hub>,
    /// Host 重写目标（docs/19 §4.3）：结构性无视入站 Host，一律写这个。
    pub public_host: String,
    /// 对外宣称的协议（反代层 https / 直连 http）。
    pub public_proto: &'static str,
    /// 登录/配对爆破限流（IP 键与用户名键共用；内存态，重启清零）。
    pub limiter: Arc<AttemptLimiter>,
}

/// 失败尝试限流：窗口内连续 N 次失败 → 锁定一段时间（登录与配对兑换共用）。
#[derive(Default)]
pub struct AttemptLimiter {
    inner: Mutex<std::collections::HashMap<String, AttemptState>>,
}

#[derive(Default, Clone, Copy)]
struct AttemptState {
    fails: u32,
    first_fail: Option<std::time::Instant>,
    locked_until: Option<std::time::Instant>,
}

const ATTEMPT_WINDOW: Duration = Duration::from_secs(15 * 60);
const ATTEMPT_MAX_FAILS: u32 = 5;
const ATTEMPT_LOCK: Duration = Duration::from_secs(15 * 60);

impl AttemptLimiter {
    /// 锁定中返回剩余秒数。
    fn check(&self, key: &str) -> Result<(), u64> {
        let map = self.inner.lock().expect("限流锁");
        if let Some(state) = map.get(key) {
            if let Some(until) = state.locked_until {
                let now = std::time::Instant::now();
                if now < until {
                    return Err((until - now).as_secs().max(1));
                }
            }
        }
        Ok(())
    }

    fn fail(&self, key: &str) {
        let mut map = self.inner.lock().expect("限流锁");
        let now = std::time::Instant::now();
        let state = map.entry(key.to_owned()).or_default();
        // 窗口过期则重新计数。
        if let Some(first) = state.first_fail {
            if now.duration_since(first) > ATTEMPT_WINDOW {
                state.fails = 0;
                state.first_fail = None;
                state.locked_until = None;
            }
        }
        state.fails += 1;
        state.first_fail.get_or_insert(now);
        if state.fails >= ATTEMPT_MAX_FAILS {
            state.locked_until = Some(now + ATTEMPT_LOCK);
            state.fails = 0;
            state.first_fail = None;
        }
    }

    fn pass(&self, key: &str) {
        self.inner.lock().expect("限流锁").remove(key);
    }
}

pub fn router(state: Arc<EdgeState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
        .route("/qx-ws", any(qx_ws))
        .route("/qx-pair", get(pair_page))
        .route("/api/pair/submit", post(pair_submit))
        // 手机凭据换 HttpOnly Cookie（会话入口；token 不落 URL/日志）。
        .route("/api/enter", post(device_enter))
        .route("/qx-fleet", get(fleet_page))
        .route("/api/fleet", get(fleet_api))
        // ---- 管理面（/console，预置用户登录，docs/19 v5.5）----
        // 壳与静态产物由内嵌资产提供（docs/20 §7.4）；/qx-pair 永久零构建。
        .route("/login", get(crate::assets::serve_index))
        .route("/console", get(crate::assets::serve_index))
        .route("/console/", get(crate::assets::serve_index))
        .route("/console/assets/{*path}", get(crate::assets::serve_asset))
        .route("/console/api/login", post(console_login))
        .route("/console/api/logout", post(console_logout))
        .route("/console/api/me", get(console_me))
        .route("/console/api/overview", get(console_overview))
        .route("/console/api/audit", get(console_audit))
        .route("/console/api/audit.csv", get(console_audit_csv))
        .route("/console/api/devices", get(console_devices))
        .route("/console/api/devices/pair", post(console_pair_issue))
        .route("/console/api/devices/pair/status", get(console_pair_status))
        .route(
            "/console/api/devices/revoke-all",
            post(console_device_revoke_all),
        )
        .route(
            "/console/api/devices/{id}/revoke",
            post(console_device_revoke),
        )
        .route(
            "/console/api/devices/{id}/restore",
            post(console_device_restore),
        )
        .route(
            "/console/api/devices/{id}/rotate",
            post(console_device_rotate),
        )
        .route("/console/api/fleet", get(console_fleet))
        .route("/i/{iid}", get(instance_entry))
        .route("/i/{iid}/", get(instance_entry))
        .route("/i/{iid}/{*rest}", get(instance_entry))
        .fallback(any(proxy))
        .with_state(state)
}

pub async fn serve(listener: tokio::net::TcpListener, state: Arc<EdgeState>) -> Result<(), String> {
    let flusher = state.clone();
    let service = router(state);
    axum::serve(
        listener,
        service.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        spawn_metrics_flusher(flusher);
        futures_util::future::pending::<()>().await
    })
    .await
    .map_err(|cause| cause.to_string())
}

/// 计量落库节奏（docs/20 §4.3）。热路径只往内存里累加，这里每分钟搬一次到
/// SQLite 小时桶——单连接互斥的库里，让每个反代请求写一行会直接变成瓶颈。
/// 进程退出时未落库的计量随内存一起丢：24h 图表少最后一分钟不影响判断。
fn spawn_metrics_flusher(state: Arc<EdgeState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.tick().await; // 第一次立即返回：不必先空等一分钟。
        loop {
            ticker.tick().await;
            let pending = state.hub.governance().drain_pending();
            if pending.is_empty() {
                continue;
            }
            let bucket = hour_bucket(crate::db::unix_now());
            let rows: Vec<crate::db::MetricRow> = pending
                .into_iter()
                .map(|item| crate::db::MetricRow {
                    bucket,
                    instance_id: item.instance,
                    device_id: item.device.unwrap_or_else(|| "-".to_owned()),
                    class: item.class.as_str().to_owned(),
                    bytes: item.bytes,
                    rejected: item.rejected,
                    shed: item.shed,
                })
                .collect();
            if let Err(cause) = state.hub.db().flush_metrics(&rows) {
                eprintln!("计量落库失败（{cause}）——本分钟计量丢失");
            }
        }
    });
}

// ---- 基础 ----

async fn healthz() -> &'static str {
    "ok"
}

async fn metrics(
    State(state): State<Arc<EdgeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if !peer.ip().is_loopback() {
        return (StatusCode::FORBIDDEN, "metrics 仅回环").into_response();
    }
    let mut text = String::new();
    for id in state.hub.online_ids() {
        text.push_str(&format!("qx_instance_online{{id=\"{id}\"}} 1\n"));
    }
    for (instance, class, admitted, rejected, shed, bytes) in
        state.hub.governance().snapshot_counters()
    {
        text.push_str(&format!(
            "qx_class_admitted{{instance=\"{instance}\",class=\"{klass}\"}} {admitted}\n\
             qx_class_rejected{{instance=\"{instance}\",class=\"{klass}\"}} {rejected}\n\
             qx_class_shed{{instance=\"{instance}\",class=\"{klass}\"}} {shed}\n\
             qx_class_bytes{{instance=\"{instance}\",class=\"{klass}\"}} {bytes}\n",
            klass = class.as_str(),
        ));
    }
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        text,
    )
        .into_response()
}

// ---- 统一端点（docs/19 §4.5）----

async fn qx_ws(State(state): State<Arc<EdgeState>>, upgrade: WebSocketUpgrade) -> Response {
    let hub = state.hub.clone();
    upgrade.on_upgrade(move |socket| async move {
        let channels = transport::from_axum_ws(socket);
        hub::run_instance_session(hub, channels).await;
    })
}

// ---- 配对（服务端签发，服务器级；docs/19 §4.4）----

#[derive(Deserialize)]
struct PairSubmit {
    code: String,
    #[serde(default)]
    name: Option<String>,
}

async fn pair_submit(
    State(state): State<Arc<EdgeState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(submit): Json<PairSubmit>,
) -> Response {
    // 爆破限流（每 IP 5 次失败/15 分钟 → 锁 15 分钟；码是一次性 48bit，正常用户碰不到）。
    let ip = client_ip(&headers, &addr);
    let key = format!("pair:{ip}");
    if let Err(retry_after) = state.limiter.check(&key) {
        state
            .hub
            .db()
            .audit("pair.locked", None, None, &format!("{{\"ip\":\"{ip}\"}}"));
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, HeaderValue::from(retry_after))],
            Json(json!({ "error": "尝试过于频繁" })),
        )
            .into_response();
    }
    let name = submit
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("手机");
    match state.hub.redeem_pair(&submit.code, name) {
        Ok((device_id, token)) => {
            state.limiter.pass(&key);
            state
                .hub
                .db()
                .audit("pair.submit_ok", None, Some(&device_id), "{}");
            Json(json!({ "deviceId": device_id, "token": token })).into_response()
        }
        Err(reason) => {
            state.limiter.fail(&key);
            state.hub.db().audit(
                "pair.submit_fail",
                None,
                None,
                &format!("{{\"ip\":\"{ip}\"}}"),
            );
            (StatusCode::FORBIDDEN, Json(json!({ "error": reason }))).into_response()
        }
    }
}

async fn pair_page(Query(params): Query<std::collections::HashMap<String, String>>) -> Response {
    let code = params.get("code").cloned().unwrap_or_default();
    axum::response::Html(pair_html(&code)).into_response()
}

/// 手机会话入口：POST {token, iid?} → 校验设备 token → 换 HttpOnly Cookie。
/// 配对页/舰队页在 JS 里调用——token 从不进 URL（避免泄漏进日志/Referer）。
async fn device_enter(
    State(state): State<Arc<EdgeState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let ip = client_ip(&headers, &addr);
    let key = format!("enter:{ip}");
    if let Err(retry_after) = state.limiter.check(&key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, HeaderValue::from(retry_after))],
            Json(json!({ "error": "尝试过于频繁" })),
        )
            .into_response();
    }
    let Some(token) = body.get("token").and_then(|value| value.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "token 必填" })),
        )
            .into_response();
    };
    let iid = body
        .get("iid")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    match state.hub.db().device_auth(token) {
        Some(device) => {
            state.limiter.pass(&key);
            state.hub.db().touch_device(&device.id);
            state
                .hub
                .db()
                .audit("device.enter", None, Some(&device.id), "{}");
            let secure = if state.public_proto == "https" {
                "; Secure"
            } else {
                ""
            };
            let mut cookies = vec![format!(
                "qx_token={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=15552000{secure}"
            )];
            if !iid.is_empty() {
                cookies.push(format!(
                    "qx_iid={iid}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000{secure}"
                ));
            }
            let mut response = Json(json!({ "ok": true, "deviceId": device.id })).into_response();
            for cookie in cookies {
                response.headers_mut().append(
                    header::SET_COOKIE,
                    HeaderValue::from_str(&cookie).expect("enter cookie 构造"),
                );
            }
            response
        }
        None => {
            state.limiter.fail(&key);
            state.hub.db().audit(
                "device.enter_fail",
                None,
                None,
                &format!("{{\"ip\":\"{ip}\"}}"),
            );
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "无效设备凭据" })),
            )
                .into_response()
        }
    }
}

// ---- 舰队（HTTP 投影 = fleet.get，docs/19 §4.5）----

fn device_auth(state: &EdgeState, headers: &HeaderMap, query: &str) -> Option<DeviceRow> {
    let token = cookie_value(headers, "qx_token")
        .or_else(|| query_param(query, "token").map(str::to_owned))
        .unwrap_or_default();
    if token.is_empty() {
        return None;
    }
    state.hub.db().device_auth(&token)
}

async fn fleet_api(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let query = query.unwrap_or_default();
    let Some(device) = device_auth(&state, &headers, &query) else {
        return (StatusCode::UNAUTHORIZED, "未配对").into_response();
    };
    state.hub.db().touch_device(&device.id);
    Json(json!({ "instances": state.hub.fleet() })).into_response()
}

async fn fleet_page() -> Response {
    axum::response::Html(FLEET_HTML).into_response()
}

// ---- 管理面（/console）：预置用户登录 → 会话 Cookie → 舰队/设备治理 ----

/// 从 Cookie 取会话并校验；有效返回 user_id。
fn session_auth(state: &EdgeState, headers: &HeaderMap) -> Option<String> {
    let token = cookie_value(headers, "qx_session")?;
    state.hub.db().validate_session(&token)
}

/// 客户端 IP：优先反代层的 X-Forwarded-For（我们的 openresty conf 只写单个值）。
fn client_ip(headers: &HeaderMap, addr: &SocketAddr) -> String {
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let source = forwarded.unwrap_or_else(|| addr.ip().to_string());
    source
        .split(',')
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_owned()
}

async fn console_login(
    State(state): State<Arc<EdgeState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    // 兼容 {username,password} 形状（显式取字段，避免 struct 里 serde 别名噪音）。
    let username = body
        .get("username")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_owned();
    let password = body
        .get("password")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    if username.is_empty() || password.is_empty() {
        return (StatusCode::BAD_REQUEST, "用户名与密码必填").into_response();
    }
    let ip = client_ip(&headers, &addr);
    let ip_key = format!("login-ip:{ip}");
    let user_key = format!("login-user:{username}");
    // IP 与用户名任一锁定都拒绝（防换单 IP 绕过 / 换用户名打同一用户）。
    for key in [&ip_key, &user_key] {
        if let Err(retry_after) = state.limiter.check(key) {
            state.hub.db().audit(
                "login.locked",
                None,
                None,
                &format!("{{\"key\":\"{key}\"}}"),
            );
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, HeaderValue::from(retry_after))],
                "尝试过于频繁，请稍后再试",
            )
                .into_response();
        }
    }
    match state.hub.db().verify_login(&username, &password) {
        Some(user_id) => {
            state.limiter.pass(&ip_key);
            state.limiter.pass(&user_key);
            state.hub.db().audit(
                "login.ok",
                None,
                None,
                &format!("{{\"name\":\"{username}\",\"ip\":\"{ip}\"}}"),
            );
            let token = state.hub.db().create_session(&user_id, &ip);
            let secure = if state.public_proto == "https" {
                "; Secure"
            } else {
                ""
            };
            let cookie = format!(
                "qx_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{secure}"
            );
            (
                StatusCode::OK,
                [(
                    header::SET_COOKIE,
                    HeaderValue::from_str(&cookie).expect("会话 cookie 构造"),
                )],
                Json(json!({ "ok": true, "username": username })),
            )
                .into_response()
        }
        None => {
            state.limiter.fail(&ip_key);
            state.limiter.fail(&user_key);
            state.hub.db().audit(
                "login.fail",
                None,
                None,
                &format!("{{\"name\":\"{username}\",\"ip\":\"{ip}\"}}"),
            );
            // 统一话术：不泄露账号存在性。
            (StatusCode::UNAUTHORIZED, "用户名或密码不正确").into_response()
        }
    }
}

async fn console_logout(State(state): State<Arc<EdgeState>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_value(&headers, "qx_session") {
        state.hub.db().delete_session(&token);
    }
    let clear = "qx_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0";
    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            HeaderValue::from_str(clear).expect("清 cookie 构造"),
        )],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

async fn console_me(State(state): State<Arc<EdgeState>>, headers: HeaderMap) -> Response {
    match session_auth(&state, &headers) {
        Some(_user_id) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        None => (StatusCode::UNAUTHORIZED, "未登录").into_response(),
    }
}

/// 小时桶起点（本地时区的自然小时，对齐 SQLite 里的 bucket 列）。
fn hour_bucket(at: i64) -> i64 {
    at.div_euclid(3600) * 3600
}

/// 设备页一次给全（docs/20 §4.2 的「一个事实只出现一次」）：会话、24h 流量、
/// 占配额、凭据都在这一个响应里——3 秒轮询下拆成多个端点没有理由。
async fn console_devices(State(state): State<Arc<EdgeState>>, headers: HeaderMap) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    let db = state.hub.db();
    let since_bucket = hour_bucket(crate::db::unix_now() - 24 * 3600);
    let total: i64 = db.device_bytes_24h(since_bucket).values().sum();
    let fleet: std::collections::HashMap<String, String> = state
        .hub
        .fleet()
        .into_iter()
        .map(|item| (item.id, item.name))
        .collect();
    let live = state.hub.governance().live_sessions();

    let devices: Vec<serde_json::Value> = db
        .device_rows()
        .into_iter()
        .map(|row| {
            // 桶形：24 项按「距今第几小时」摆位，缺桶补 0（不跳格）。
            let shape = dense_24(&db.device_traffic_24h(&row.id, since_bucket), since_bucket);
            let bytes24h: i64 = shape.iter().sum();
            let sessions: Vec<serde_json::Value> = live
                .iter()
                .filter(|session| session.device.as_deref() == Some(row.id.as_str()))
                .map(|session| {
                    json!({
                        "instanceId": session.instance,
                        "instanceName": fleet.get(&session.instance).cloned().unwrap_or_else(|| session.instance.clone()),
                        "class": session.class.as_str(),
                        "seconds": session.seconds,
                        "bytesPerSec": session.bytes.checked_div(session.seconds.max(1)).unwrap_or(0),
                    })
                })
                .collect();
            json!({
                "id": row.id,
                "name": row.name,
                "createdAt": row.created_at,
                "lastSeenAt": row.last_seen_at,
                "revoked": row.revoked,
                "lastIp": row.last_ip,
                "bytes24h": bytes24h,
                "share": if total > 0 { bytes24h as f64 / total as f64 } else { 0.0 },
                "shape": shape,
                "sessions": sessions,
                "credential": {
                    "hint": row.credential_hint.unwrap_or_else(|| "—".to_owned()),
                    "issuedAt": row.credential_issued_at,
                    "expiresAt": row.credential_expires_at,
                },
            })
        })
        .collect();
    Json(json!({ "devices": devices })).into_response()
}

/// 把稀疏的 (bucket, bytes) 摊成连续 24 根柱子（从旧到新）。
fn dense_24(rows: &[(i64, i64)], since_bucket: i64) -> Vec<i64> {
    let mut map: std::collections::HashMap<i64, i64> = rows.iter().copied().collect();
    (0..24)
        .map(|offset| map.remove(&(since_bucket + offset * 3600)).unwrap_or(0))
        .collect()
}

async fn console_pair_issue(State(state): State<Arc<EdgeState>>, headers: HeaderMap) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    match state.hub.db().create_pair_code("console") {
        Ok((code, expires_at)) => {
            // 审计由 create_pair_code 自己写（detail 带 via=console），路由不再补一条。
            let pair_url = format!(
                "{}://{}/qx-pair?code={code}",
                state.public_proto, state.public_host
            );
            Json(json!({ "code": code, "expiresAt": expires_at, "pairUrl": pair_url }))
                .into_response()
        }
        Err(cause) => (StatusCode::INTERNAL_SERVER_ERROR, cause).into_response(),
    }
}

/// 配对弹层的状态机（docs/20 §4.4）：pending → used → expired 全部由库里那
/// 一条码记录推导，前端不自己猜——重开页面也不会判错。
async fn console_pair_status(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    let Some(code) = params.get("code").map(String::as_str) else {
        return (StatusCode::BAD_REQUEST, "code 必填").into_response();
    };
    use crate::db::PairState;
    let body = match state.hub.db().pair_state(code) {
        PairState::Pending { expires_at } => json!({
            "status": "pending",
            "deviceName": serde_json::Value::Null,
            "expiresAt": expires_at,
        }),
        PairState::Used { device_name } => json!({
            "status": "used",
            "deviceName": device_name,
            "expiresAt": crate::db::unix_now(),
        }),
        PairState::Expired => json!({
            "status": "expired",
            "deviceName": serde_json::Value::Null,
            "expiresAt": crate::db::unix_now(),
        }),
        PairState::Unknown => {
            return (StatusCode::NOT_FOUND, "配对码不存在").into_response();
        }
    };
    Json(body).into_response()
}

/// 吊销单台设备。只翻标志位，因此「撤销」在实现上成立（docs/20 §4.4）。
async fn console_device_revoke(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    match state.hub.db().revoke_device(&id, "console") {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(cause) => (StatusCode::NOT_FOUND, cause).into_response(),
    }
}

/// 恢复设备（撤销的逆操作，语义上同一个标志位）。
async fn console_device_restore(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    match state.hub.db().restore_device(&id) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(cause) => (StatusCode::NOT_FOUND, cause).into_response(),
    }
}

/// 批量吊销：不可撤销，所以前端不给撤销按钮，只给红色常驻条（docs/20 §4.10）。
async fn console_device_revoke_all(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    match state.hub.db().revoke_all_devices() {
        Ok(revoked) => Json(json!({ "revoked": revoked })).into_response(),
        Err(cause) => (StatusCode::INTERNAL_SERVER_ERROR, cause).into_response(),
    }
}

/// 轮换设备凭据：新明文只在这里回一次，库里仍只存哈希。
async fn console_device_rotate(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    match state.hub.db().rotate_device_token(&id) {
        Ok((token, issued_at, expires_at)) => Json(json!({
            // token 只此一次可见：前端展示后就不再有第二次机会。
            "token": token,
            "hint": crate::db::credential_hint(&token),
            "issuedAt": issued_at,
            "expiresAt": expires_at,
        }))
        .into_response(),
        Err(cause) => (StatusCode::NOT_FOUND, cause).into_response(),
    }
}

async fn console_fleet(State(state): State<Arc<EdgeState>>, headers: HeaderMap) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    Json(json!({ "instances": state.hub.fleet() })).into_response()
}

/// 概览一页（docs/20 §4.3）：KPI、水位、实例表、24h 流量形状。
async fn console_overview(State(state): State<Arc<EdgeState>>, headers: HeaderMap) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    let db = state.hub.db();
    let now = crate::db::unix_now();
    let since_bucket = hour_bucket(now - 24 * 3600);
    let fleet = state.hub.fleet();
    let levels = state.hub.governance().snapshot_levels();
    let buckets = db.traffic_buckets(since_bucket);
    let (throttled, interactive_throttled) = db.throttle_24h(since_bucket);

    let mut interactive = vec![0i64; 24];
    let mut other = vec![0i64; 24];
    for (bucket, interactive_bytes, other_bytes) in buckets {
        let offset = ((bucket - since_bucket) / 3600) as usize;
        if offset < 24 {
            interactive[offset] = interactive_bytes;
            other[offset] = other_bytes;
        }
    }
    let bytes24h: i64 = interactive.iter().sum::<i64>() + other.iter().sum::<i64>();
    let live = state.hub.governance().live_sessions();
    let instances: Vec<serde_json::Value> = fleet
        .iter()
        .map(|item| {
            let level = levels.iter().find(|level| level.instance == item.id);
            // 设备关联来自在途会话：这台实例此刻正被哪些设备用着。
            let mut ids: Vec<String> = live
                .iter()
                .filter(|session| session.instance == item.id)
                .filter_map(|session| session.device.clone())
                .collect();
            ids.sort();
            ids.dedup();
            json!({
                "id": item.id,
                "name": item.name,
                "online": item.online,
                "version": serde_json::Value::Null,
                "connectedFor": serde_json::Value::Null,
                "devices": ids,
                "bytes1h": db.instance_bytes_1h(&item.id, hour_bucket(now)),
                "status": match (item.online, level) {
                    (false, _) => "offline",
                    // 有在途 bulk 或带宽桶快见底 = 「受限」（docs/20 §4.3）。
                    (true, Some(level)) if level.bulk > 0 || level.bw < 0.2 => "limited",
                    _ => "ok",
                },
            })
        })
        .collect();

    let caps = state.hub.governance().caps();
    let waterline = json!({
        "interactive": {
            "level": levels.iter().map(|level| level.interactive).sum::<usize>(),
            "cap": caps.interactive,
        },
        "page": {
            "level": levels.iter().map(|level| level.page).sum::<usize>(),
            "cap": caps.page,
        },
        "bulk": {
            "level": levels.iter().map(|level| level.bulk).sum::<usize>(),
            "cap": caps.bulk,
        },
        "bandwidth": {
            // 带宽用「已用百分比」而不是字节：M2 没有时间序列的字节语义，
            // 但桶的余量是实时事实（docs/20 §4.3）。
            "level": (levels
                .iter()
                .map(|level| 1.0 - level.bw)
                .fold(0.0f64, f64::max) * 100.0)
                .round() as i64,
            "cap": 100,
        },
    });

    Json(json!({
        "kpi": {
            "instancesOnline": fleet.iter().filter(|item| item.online).count(),
            "instancesTotal": fleet.len(),
            "instancesOfflineFor": serde_json::Value::Null,
            "activeDevices": db.devices().iter().filter(|device| !device.revoked).count(),
            "bytes24h": bytes24h,
            "throttled24h": throttled,
            "interactiveThrottled24h": interactive_throttled,
        },
        "waterline": waterline,
        "instances": instances,
        "traffic": { "interactive": interactive, "other": other },
        "generatedAt": now,
    }))
    .into_response()
}

/// 审计页：游标翻页（比 OFFSET 稳——翻页期间有新事件也不会错位）。
async fn console_audit(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    const PAGE: usize = 60;
    let rows = state.hub.db().audit_query(
        params.get("type").map(String::as_str),
        params
            .get("from")
            .and_then(|value| value.parse::<i64>().ok()),
        params
            .get("cursor")
            .and_then(|value| value.parse::<i64>().ok()),
        PAGE + 1,
    );
    let next_cursor = if rows.len() > PAGE {
        rows.get(PAGE - 1)
            .map(|row| row.id)
            .map(|id| id.to_string())
    } else {
        None
    };
    let events: Vec<serde_json::Value> = rows
        .iter()
        .take(PAGE)
        .map(|row| {
            json!({
                "id": row.id,
                "at": row.at,
                "event": row.event,
                "instanceId": row.instance_id,
                "deviceId": row.device_id,
                "detail": serde_json::from_str::<serde_json::Value>(&row.detail_json)
                    .unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    Json(json!({ "events": events, "nextCursor": next_cursor })).into_response()
}

/// CSV 导出：一次最多 5000 行（够审一次，又不至于把浏览器撑死）。
async fn console_audit_csv(
    State(state): State<Arc<EdgeState>>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if session_auth(&state, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "未登录").into_response();
    }
    let rows = state.hub.db().audit_query(
        params.get("type").map(String::as_str),
        params
            .get("from")
            .and_then(|value| value.parse::<i64>().ok()),
        None,
        5000,
    );
    let mut out = String::from("time,event,instance,device,detail\n");
    for row in rows.iter().rev() {
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_field(&format_iso(row.at)),
            csv_field(&row.event),
            csv_field(row.instance_id.as_deref().unwrap_or("")),
            csv_field(row.device_id.as_deref().unwrap_or("")),
            csv_field(&row.detail_json)
        ));
    }
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"audit.csv\""),
            ),
        ],
        out,
    )
        .into_response()
}

/// CSV 字段转义：引号/逗号/换行任一出现就整体加引号（Excel 会拒绝别的写法）。
fn csv_field(value: &str) -> String {
    if value.contains(['"', ',', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn format_iso(at: i64) -> String {
    // 审计导出要能直接进 Excel：不引 chrono，用 civil_from_days 手算 UTC。
    let days = at.div_euclid(86400);
    let secs = at.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// 天 → 民用日期（Howard Hinnant 的算法）。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---- 实例入口：设 cookie 后回根（docs/19 §3.1）----

async fn instance_entry(axum::extract::Path(iid): axum::extract::Path<String>) -> Response {
    let cookie = format!("qx_iid={iid}; Path=/; HttpOnly; Max-Age=31536000; SameSite=Lax");
    (
        StatusCode::FOUND,
        [
            (header::LOCATION, HeaderValue::from_static("/")),
            (
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie).expect("iid 来自路径段"),
            ),
        ],
    )
        .into_response()
}

// ---- 会话代理（数据面）----

/// 配额句柄的共享壳：body 流结束或被放弃（客户端断开）都经 Drop 归还配额。
struct GuardSlot(Mutex<Option<Guard>>);

impl Drop for GuardSlot {
    fn drop(&mut self) {
        self.0.lock().expect("guard 锁").take();
    }
}

impl GuardSlot {
    fn add_bytes(&self, chunk: u64) -> Option<Duration> {
        self.0
            .lock()
            .expect("guard 锁")
            .as_mut()
            .and_then(|guard| guard.add_bytes(chunk))
    }

    fn promote_if_sse(&self, content_type: Option<&HeaderValue>) {
        let is_sse = content_type
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        if is_sse {
            if let Some(guard) = self.0.lock().expect("guard 锁").as_mut() {
                guard.promote_interactive();
            }
        }
    }
}

async fn proxy(
    State(state): State<Arc<EdgeState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request<axum::body::Body>,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let query = parts.uri.query().unwrap_or("");
    let path = parts.uri.path().to_owned();
    // hyper 服务端把升级句柄预置在「请求 extensions」里（axum WS 同源）；先取走。
    let on_edge_upgrade = parts.extensions.remove::<hyper::upgrade::OnUpgrade>();

    // 路由键：设备（服务器级）+ 目标实例（cookie）。
    let Some(device) = device_auth(&state, &parts.headers, query) else {
        if path == "/" {
            // 域名门面：未认证访客一律先到登录页（管理面）；
            // 手机配对入口在登录页页脚，避免把配对页当门面。
            return Redirect::to("/login").into_response();
        }
        return (StatusCode::UNAUTHORIZED, "未配对设备").into_response();
    };
    state.hub.db().touch_device(&device.id);
    // 来源 IP 只在变化时写库（反代热路径，见 db::touch_device_from 的注释）。
    state
        .hub
        .db()
        .touch_device_from(&device.id, &client_ip(&parts.headers, &peer));
    let Some(iid) = cookie_value(&parts.headers, "qx_iid") else {
        return Redirect::to("/qx-fleet").into_response();
    };

    // 治理：分类 + 准入（docs/19 §5）。
    let is_upgrade = parts
        .headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let body_len = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let class = Class::of_request(is_upgrade, body_len);
    // 归因到设备：设备页的 24h 流量 / 占配额 / 当前会话都从这里来（docs/20 §4.2）。
    let guard = match state
        .hub
        .governance()
        .admit(&iid, Some(device.id.as_str()), class)
        .await
    {
        Ok(guard) => guard,
        Err(Reject::Busy { retry_after_secs }) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [
                    (
                        header::RETRY_AFTER,
                        HeaderValue::from_str(&retry_after_secs.to_string()).expect("数字"),
                    ),
                    (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                ],
            )
                .into_response();
        }
        Err(Reject::Shed) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::RETRY_AFTER, HeaderValue::from_static("2"))],
            )
                .into_response();
        }
    };
    let slot = Arc::new(GuardSlot(Mutex::new(Some(guard))));

    // 请求体：有界缓冲（≤16MiB）并记账。
    let body_bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(cause) => {
            return (StatusCode::BAD_REQUEST, format!("读取请求体失败：{cause}")).into_response()
        }
    };
    if body_bytes.len() > MAX_BODY {
        return (StatusCode::PAYLOAD_TOO_LARGE, "请求体超限").into_response();
    }
    slot.add_bytes(body_bytes.len() as u64);

    // 开流（在线性由 hub 判定）。
    let stream = match state.hub.open_stream(&iid).await {
        Ok(stream) => stream,
        Err(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                [(header::RETRY_AFTER, HeaderValue::from_static("2"))],
            )
                .into_response();
        }
    };

    // 组装转发请求：Host 重写 + 逐跳剥离 + 边缘盖章（docs/19 §4.3）。
    let mut builder = Request::builder()
        .method(parts.method.clone())
        .uri(
            axum::http::Uri::builder()
                .path_and_query(
                    parts
                        .uri
                        .path_and_query()
                        .map(|value| value.as_str().to_owned())
                        .unwrap_or_else(|| path.clone()),
                )
                .build()
                .expect("uri 重建"),
        )
        .version(axum::http::Version::HTTP_11);
    {
        let headers = builder.headers_mut().expect("builder 有效");
        for (name, value) in parts.headers.iter() {
            if STRIP_HEADERS.contains(&name.as_str()) {
                continue;
            }
            headers.insert(name, value.clone());
        }
        headers.insert(
            HeaderName::from_static("host"),
            HeaderValue::from_str(&state.public_host).expect("配置合法"),
        );
        headers.insert(
            HeaderName::from_static("x-qx-device"),
            HeaderValue::from_str(&device.id).expect("id hex"),
        );
        if let Some(secret) = state.hub.edge_secret(&iid) {
            if let Ok(value) = HeaderValue::from_str(&secret) {
                headers.insert(HeaderName::from_static("x-qx-edge"), value);
            }
        }
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_str(&peer.ip().to_string())
                .unwrap_or(HeaderValue::from_static("unknown")),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static(state.public_proto),
        );
        // 升级请求必须带 Connection: Upgrade 才有升级语义（剥离逐跳头后按需补回）。
        if is_upgrade {
            headers.insert(
                HeaderName::from_static("connection"),
                HeaderValue::from_static("Upgrade"),
            );
        }
    }
    let forwarded = builder
        .body(axum::body::Body::from(body_bytes))
        .expect("头已校验");

    // 一流一事务（docs/19 §4.2）。
    let mut response = match tunnel::round_trip(stream, forwarded).await {
        Ok(response) => response,
        Err(cause) => {
            return (StatusCode::BAD_GATEWAY, format!("实例不可达：{cause}")).into_response()
        }
    };

    // 响应头处理：SSE 升格 interactive；101 走透传桥。
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let Some(on_edge) = on_edge_upgrade else {
            return (StatusCode::BAD_GATEWAY, "升级句柄缺失").into_response();
        };
        return upgrade_passthrough(&mut response, on_edge, slot).await;
    }
    slot.promote_if_sse(response.headers().get(header::CONTENT_TYPE));

    let (response_parts, response_body) = response.into_parts();
    let data_stream = response_body.into_data_stream();
    let body_slot = slot.clone();
    let shaped =
        futures_util::stream::unfold((data_stream, Some(body_slot)), |mut state| async move {
            let slot = match state.1.take() {
                Some(slot) => slot,
                None => return None,
            };
            match state.0.next().await {
                Some(Ok(chunk)) => {
                    if let Some(delay) = slot.add_bytes(chunk.len() as u64) {
                        tokio::time::sleep(delay).await;
                    }
                    Some((
                        Ok::<axum::body::Bytes, std::io::Error>(chunk),
                        (state.0, Some(slot)),
                    ))
                }
                Some(Err(_)) | None => None,
            }
        });
    let mut builder = Response::builder().status(response_parts.status);
    *builder.headers_mut().expect("builder 有效") = response_parts.headers;
    builder
        .body(axum::body::Body::from_stream(shaped))
        .expect("头已校验")
}

/// 101 透传：边缘与隧道两侧各自升级后桥接原始字节（WS 帧端到端原样通过）。
///
/// 边缘侧句柄来自「请求 extensions」（hyper 服务端语义）；隧道侧来自
/// hyper 客户端的 101 响应（response extensions）。
async fn upgrade_passthrough(
    tunnel_response: &mut hyper::Response<hyper::body::Incoming>,
    on_edge: hyper::upgrade::OnUpgrade,
    slot: Arc<GuardSlot>,
) -> Response {
    let on_tunnel = hyper::upgrade::on(&mut *tunnel_response);

    let mut edge_response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .body(axum::body::Body::empty())
        .expect("builder 有效");
    {
        let headers = edge_response.headers_mut();
        for (name, value) in tunnel_response.headers().iter() {
            headers.insert(name, value.clone());
        }
    }

    tokio::spawn(async move {
        let (edge, tunneled) = tokio::join!(on_edge, on_tunnel);
        // TokioIo 双向桥：Upgraded(hyper rt) → tokio io（hyper-util rt::tokio 的反向 impl）。
        if let (Ok(edge), Ok(tunneled)) = (edge, tunneled) {
            let mut edge = hyper_util::rt::TokioIo::new(edge);
            let mut tunneled = hyper_util::rt::TokioIo::new(tunneled);
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut edge_buf = [0u8; 16 * 1024];
            let mut tunnel_buf = [0u8; 16 * 1024];
            loop {
                tokio::select! {
                    readable = edge.read(&mut edge_buf) => {
                        match readable {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if let Some(delay) = slot.add_bytes(n as u64) {
                                    tokio::time::sleep(delay).await;
                                }
                                if tunneled.write_all(&edge_buf[..n]).await.is_err() { break; }
                            }
                        }
                    }
                    readable = tunneled.read(&mut tunnel_buf) => {
                        match readable {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if let Some(delay) = slot.add_bytes(n as u64) {
                                    tokio::time::sleep(delay).await;
                                }
                                if edge.write_all(&tunnel_buf[..n]).await.is_err() { break; }
                            }
                        }
                    }
                }
            }
            let _ = edge.shutdown().await;
            let _ = tunneled.shutdown().await;
        }
    });

    edge_response
}

// ---- 小工具 ----

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for value in headers.get_all(header::COOKIE) {
        let value = value.to_str().ok()?;
        for part in value.split(';') {
            let part = part.trim();
            if let Some(token) = part.strip_prefix(&prefix) {
                return Some(token.to_owned());
            }
        }
    }
    None
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then_some(value)
    })
}

// ---- 静态页（inline，无构建链）----

fn pair_html(code: &str) -> String {
    format!(
        r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>配对 · 千寻</title><style>
body{{font-family:system-ui;margin:0;background:#0f1115;color:#e6e8ee;display:flex;min-height:100vh;align-items:center;justify-content:center}}
.card{{background:#171a21;border:1px solid #262b36;border-radius:14px;padding:28px;width:min(92vw,360px)}}
h1{{font-size:18px;margin:0 0 6px}} p{{color:#8b93a7;font-size:13px;margin:6px 0 18px}}
input{{width:100%;box-sizing:border-box;padding:12px;border-radius:10px;border:1px solid #2c3240;background:#0f1115;color:#e6e8ee;font-size:16px;margin-bottom:10px}}
button{{width:100%;padding:12px;border:0;border-radius:10px;background:#2563eb;color:#fff;font-size:16px}}
.err{{color:#f87171;font-size:13px;min-height:18px}}
</style></head><body><div class="card">
<h1>配对到千寻服务端</h1><p>输入千寻远程页展示的一次性配对码</p>
<input id="code" placeholder="配对码" value="{code}" autocomplete="off">
<input id="name" placeholder="设备名（如：我的手机）" autocomplete="off">
<div class="err" id="err"></div>
<button onclick="go()">配对</button>
<script>
async function go(){{try{{
const code=document.getElementById('code').value.trim();
const name=document.getElementById('name').value.trim();
const r=await fetch('/api/pair/submit',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify({{code,name}})}});
const data=await r.json();
if(!r.ok){{document.getElementById('err').textContent=data.error||'配对失败';return;}}
localStorage.setItem('qx_token',data.token);
localStorage.setItem('qx_device',data.deviceId);
try{{await fetch('/api/enter',{{method:'POST',headers:{{'content-type':'application/json'}},body:JSON.stringify({{token:data.token}})}});}}catch(e){{}}
 location.href='/qx-fleet';
}}catch(e){{document.getElementById('err').textContent='网络错误';}}}}
</script></div></body></html>"#
    )
}

const FLEET_HTML: &str = r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>设备 · 千寻</title><style>
body{font-family:system-ui;margin:0;background:#0f1115;color:#e6e8ee}
.wrap{max-width:480px;margin:0 auto;padding:24px 16px}
h1{font-size:18px} .hint{color:#8b93a7;font-size:13px}
.item{display:flex;justify-content:space-between;align-items:center;background:#171a21;border:1px solid #262b36;border-radius:12px;padding:14px 16px;margin:10px 0;text-decoration:none;color:inherit}
.item.off{opacity:.45;pointer-events:none}
.dot{width:10px;height:10px;border-radius:50%;margin-right:10px;background:#3fb950;flex:none}
.dot.off{background:#6e7681}
.name{flex:1} .meta{color:#8b93a7;font-size:12px}
</style></head><body><div class="wrap">
<h1>我的设备</h1><p class="hint">点击在线主机进入会话；离线主机需先开机连网</p>
<div id="list"><p class="hint">加载中…</p></div>
<p class="hint"><a href="/qx-pair" style="color:#2563eb">重新配对</a></p>
</div><script>
async function load(){
 const token=localStorage.getItem('qx_token')||new URLSearchParams(location.search).get('token');
 if(!token){location.href='/login';return;}
 try{await fetch('/api/enter',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({token:token})});}catch(e){}
 const r=await fetch('/api/fleet?token='+encodeURIComponent(token));
 if(r.status===401){location.href='/login';return;}
 const data=await r.json();
 const list=document.getElementById('list');
 list.innerHTML='';
 for(const it of data.instances){
   const a=document.createElement('a');
   a.className='item'+(it.online?'':' off');
   a.href='/i/'+it.id+'/';
   a.innerHTML='<span class="dot'+(it.online?'':' off')+'"></span><span class="name">'+it.name+
     '<div class="meta">'+(it.qianxun||'')+(it.dsh?' · DSH '+it.dsh:'')+'</div></span>'+(it.online?'<span>进入 →</span>':'<span class="meta">离线</span>');
   list.appendChild(a);
 }
 if(!data.instances.length)list.innerHTML='<p class="hint">暂无主机实例（先在服务端预置实例，再让千寻连接）</p>';
}
load();setInterval(load,10000);
</script></body></html>"#;

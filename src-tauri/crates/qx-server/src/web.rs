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
}

pub fn router(state: Arc<EdgeState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
        .route("/qx-ws", any(qx_ws))
        .route("/qx-pair", get(pair_page))
        .route("/api/pair/submit", post(pair_submit))
        .route("/qx-fleet", get(fleet_page))
        .route("/api/fleet", get(fleet_api))
        .route("/i/{iid}", get(instance_entry))
        .route("/i/{iid}/", get(instance_entry))
        .route("/i/{iid}/{*rest}", get(instance_entry))
        .fallback(any(proxy))
        .with_state(state)
}

pub async fn serve(listener: tokio::net::TcpListener, state: Arc<EdgeState>) -> Result<(), String> {
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|cause| cause.to_string())
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
    Json(submit): Json<PairSubmit>,
) -> Response {
    let name = submit
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("手机");
    match state.hub.redeem_pair(&submit.code, name) {
        Ok((device_id, token)) => {
            state
                .hub
                .db()
                .audit("pair.submit_ok", None, Some(&device_id), "{}");
            Json(json!({ "deviceId": device_id, "token": token })).into_response()
        }
        Err(reason) => (StatusCode::FORBIDDEN, Json(json!({ "error": reason }))).into_response(),
    }
}

async fn pair_page(Query(params): Query<std::collections::HashMap<String, String>>) -> Response {
    let code = params.get("code").cloned().unwrap_or_default();
    axum::response::Html(pair_html(&code)).into_response()
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
            return Redirect::to("/qx-pair").into_response();
        }
        return (StatusCode::UNAUTHORIZED, "未配对设备").into_response();
    };
    state.hub.db().touch_device(&device.id);
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
    let guard = match state.hub.governance().admit(&iid, class).await {
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
async function load(){{
 const token=localStorage.getItem('qx_token')||new URLSearchParams(location.search).get('token');
 if(!token){{location.href='/qx-pair';return;}}
 const r=await fetch('/api/fleet?token='+encodeURIComponent(token));
 if(r.status===401){{location.href='/qx-pair';return;}}
 const data=await r.json();
 const list=document.getElementById('list');
 list.innerHTML='';
 for(const it of data.instances){{
   const a=document.createElement('a');
   a.className='item'+(it.online?'':' off');
   a.href='/i/'+it.id+'/';
   a.innerHTML='<span class="dot'+(it.online?'':' off')+'"></span><span class="name">'+it.name+
     '<div class="meta">'+(it.qianxun||'')+(it.dsh?' · DSH '+it.dsh:'')+'</div></span>'+(it.online?'<span>进入 →</span>':'<span class="meta">离线</span>');
   list.appendChild(a);
 }}
 if(!data.instances.length)list.innerHTML='<p class="hint">暂无主机实例（先在服务端预置实例，再让千寻连接）</p>';
}}
load();setInterval(load,10000);
</script></body></html>"#;

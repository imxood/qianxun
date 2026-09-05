//! 网关本体：axum 服务 + 回环/局域网双端 + DSH 回环转发。
//!
//! 一个进程一个 axum Router，但绑两个 socket——**永远**绑 `127.0.0.1:port`
//! 给本地外壳用（DSH 页 iframe、Notes 页 fetch），**按需**绑 `bind_ip:port`
//! 给局域网用（手机扫描）。两条入口走同一份共享状态：回环请求免 qx_token
//! 但要走 Host/Origin 栅栏（挡浏览器侧 drive-by 与 DNS rebinding），
//! 局域网请求必须带 qx_token。端口按构建模式默认 23090/23091，实例内
//! 恒定——iframe URL 永不变（DSH revive 热吸收的前提）。
//!
//! DSH 0.1.2 起的浏览器鉴权由共享原语 dsh_upstream 在服务端完成
//! （token 与 cookie 都留在千寻侧，回环/局域网都一样）。

use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{ws::WebSocketUpgrade, OriginalUri, State},
    http::{HeaderMap, Request, Response, StatusCode},
    routing::any,
    Router,
};
use tokio::sync::watch;

use crate::dsh_upstream::{self, plain, query_param, Upstream};
use crate::remote::RemoteDevice;

/// 运行中的网关句柄：停止信号 + 监督任务 + 监听地址 + 启动配置指纹。
/// `lan_addr = None` 表示只绑了回环（远程功能未启用）。
pub struct GatewayHandle {
    pub shutdown: watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
    /// LAN 监听地址；未启用远程时为 None。
    pub lan_addr: Option<SocketAddr>,
    /// 回环监听地址（永远存在）。DSH 页 iframe URL 即 `http://{loopback_addr}`。
    pub loopback_addr: SocketAddr,
    pub fingerprint: u64,
}

/// 网关共享状态：上游原语（origin/启动 URL/cookie）+ 设备表快照 +
/// 当前回环地址（用于 Host 路由分发）。
#[derive(Clone)]
pub struct GatewayState {
    pub upstream: Upstream,
    pub devices: Arc<Vec<RemoteDevice>>,
    pub loopback_addr: SocketAddr,
}

impl GatewayState {
    /// 设备鉴权（仅局域网入口走）。
    fn token_valid(&self, presented: &str) -> bool {
        !presented.is_empty()
            && self
                .devices
                .iter()
                .any(|device| !device.revoked && device.token == presented)
    }

    /// 当前请求来自回环入口？靠 Host 头与本机回环端口比对——同进程绑了
    /// 多个 socket，但只有真正命中回环 listener 的请求才匹配此端口。
    fn is_loopback(&self, headers: &HeaderMap) -> bool {
        let Some(host) = headers.get("host").and_then(|value| value.to_str().ok()) else {
            return false;
        };
        let Some((name, port)) = host.rsplit_once(':') else {
            return false;
        };
        port == self.loopback_addr.port().to_string()
            && matches!(name, "127.0.0.1" | "localhost" | "[::1]")
    }
}

/// 启动网关。永远绑回环 `127.0.0.1:port`；`bind_ip` 非空时同时绑局域网。
/// fingerprint = 启动配置指纹（commands::fingerprint），存入句柄供 sync 比对。
/// `upstream` 是 DSH origin（host:port）；`dsh_url` 是 DSH 就绪时打印的
/// 完整 URL（已含 `?token=`）—— 兑换 cookie 时整段打过去；None = DSH 未就绪。
pub async fn start(
    bind_ip: &str,
    port: u16,
    upstream: String,
    dsh_url: Option<String>,
    devices: Vec<RemoteDevice>,
    fingerprint: u64,
) -> Result<GatewayHandle, String> {
    let loopback_bind: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|cause| format!("回环地址不合法：{cause}"))?;
    let upstream_state = Upstream::new(upstream, dsh_url.clone());
    let loopback_listener = tokio::net::TcpListener::bind(loopback_bind)
        .await
        .map_err(|cause| format!("回环监听失败（{loopback_bind}）：{cause}"))?;
    let loopback_addr = loopback_listener
        .local_addr()
        .map_err(|cause| cause.to_string())?;

    let state = GatewayState {
        upstream: upstream_state.clone(),
        devices: Arc::new(devices),
        loopback_addr,
    };
    // 顺手把登录兑换做掉：首个请求就不必等一次兑换往返。
    if dsh_url.is_some() {
        match upstream_state.ensure_cookie().await {
            Some(_) => crate::logging::log("info", "网关已向 DSH 兑换登录 cookie"),
            None => crate::logging::log(
                "warn",
                "网关暂未取得 DSH 登录 cookie（DSH 未就绪？）；转发时会自动重试",
            ),
        }
    }
    // `/api/remote.mux` 是 DSH 唯一的 WS mux 路径（@deepseek-ai/dsh-api-gateway
    // 注册的 registerUpgrade 唯一项）。WS 路由必须显式列出——走兜底
    // handler 时 forward 走的是 reqwest，不能转发 WS 升级（要 tokio-tungstenite
    // 桥），会变 404。
    let app = Router::new()
        .route("/api/remote.mux", any(ws_handler))
        .route("/{*rest}", any(handler))
        .route("/", any(handler))
        .with_state(state.clone());

    // 可选绑局域网——失败时不让半开状态出现，调用方决定是否回退到回环
    // 唯一模式。bind_ip 与回环同地址（127.0.0.1）时直接跳过 LAN，避免双绑
    // 同一 socket 报错。
    let (lan_addr, lan_listener) = if !bind_ip.is_empty() && bind_ip != "127.0.0.1" {
        let lan_bind: SocketAddr = format!("{bind_ip}:{port}")
            .parse()
            .map_err(|cause| format!("绑定地址不合法（{bind_ip}:{port}）：{cause}"))?;
        let listener = tokio::net::TcpListener::bind(lan_bind)
            .await
            .map_err(|cause| format!("网关监听失败（{lan_bind}）：{cause}"))?;
        let local = listener.local_addr().map_err(|cause| cause.to_string())?;
        (Some(local), Some(listener))
    } else {
        (None, None)
    };

    // 单个监督任务托起所有 server，共享同一停止信号——任一 server 退出就
    // 结束整组（restart 由 sync 层重新建）。
    let (shutdown, mut signal) = watch::channel(false);
    let task = tokio::spawn(async move {
        // axum 0.8 的 Serve 实现 IntoFuture（不是直接 Future），select! 接受
        // 两者但 Either 要 Future 形态——用 .into_future() 统一一下。
        let loopback_server = axum::serve(loopback_listener, app.clone()).into_future();
        // LAN 未启用时的占位分支（pending）与真实 serve 的 Future 形态统一。
        type LanServer = futures_util::future::Either<
            std::future::Pending<Result<(), std::io::Error>>,
            std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), std::io::Error>> + Send>>,
        >;
        let lan_server: LanServer = match lan_listener {
            Some(listener) => futures_util::future::Either::Right(Box::pin(
                axum::serve(listener, app).into_future(),
            )),
            None => futures_util::future::Either::Left(std::future::pending()),
        };
        tokio::select! {
            _ = signal.changed() => {
                // 停机请求：丢弃所有 server（连接随之关闭）。
                // 落日志：网关每重建一次就杀光全部 SSE/WS 连接，
                // 「连接异常」类问题先看这里有没有意外停机。
                crate::logging::log(
                    "info",
                    "网关停止：经由网关的全部连接（HTTP/SSE/WS）随即断开",
                );
            }
            result = loopback_server => {
                if let Err(cause) = result {
                    crate::logging::log("warn", &format!("回环网关退出：{cause}"));
                }
            }
            result = lan_server => {
                if let Err(cause) = result {
                    crate::logging::log("warn", &format!("局域网网关退出：{cause}"));
                }
            }
        }
    });
    Ok(GatewayHandle {
        shutdown,
        task,
        lan_addr,
        loopback_addr,
        fingerprint,
    })
}

/// 统一入口：按 Host 头分发——回环走栅栏免鉴权，局域网走设备配对。
async fn handler(State(state): State<GatewayState>, request: Request<Body>) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();

    if state.is_loopback(request.headers()) {
        // 回环入口：栅栏通过即转发（含 SSE 流响应与 DSH 页面本身）。
        if !access_allowed(request.headers()) {
            crate::logging::log("warn", &format!("[http] 拒绝非本机外壳来源请求：{path}"));
            return plain(StatusCode::FORBIDDEN, "非本机外壳来源，拒绝访问");
        }
        return dsh_upstream::forward(&state.upstream, request).await;
    }

    // 局域网入口：配对 → 鉴权 → 转发（含 SSE 流响应）。
    if path == "/qx-gate" {
        let token = query_param(&query, "token");
        if state.token_valid(&token) {
            return response_with_cookie(&token);
        }
        return plain(StatusCode::UNAUTHORIZED, "无效或已吊销的配对 token");
    }
    if !authorized(&state, &query, &request.headers().clone()) {
        return plain(
            StatusCode::UNAUTHORIZED,
            "未配对设备：请用千寻生成的配对链接打开 /qx-gate?token=…",
        );
    }
    dsh_upstream::forward(&state.upstream, request).await
}

/// WS 下行桥入口：按 Host 头分发鉴权。两条入口都升级为 WS 双向桥。
async fn ws_handler(
    State(state): State<GatewayState>,
    uri: OriginalUri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response<Body> {
    let path = uri.path().to_owned();
    if state.is_loopback(&headers) {
        if !access_allowed(&headers) {
            crate::logging::log(
                "warn",
                &format!("[ws] 拒绝非本机外壳来源的 WS 升级：{path}"),
            );
            return plain(StatusCode::FORBIDDEN, "非本机外壳来源，拒绝访问");
        }
    } else if !authorized(&state, uri.query().unwrap_or(""), &headers) {
        crate::logging::log("warn", &format!("[ws] 拒绝未配对设备的 WS 升级：{path}"));
        return plain(StatusCode::UNAUTHORIZED, "未配对设备");
    }
    dsh_upstream::upgrade_ws(&state.upstream, ws, &path).await
}

/// 回环入口的栅栏：
/// - `Host` 必须是回环名带端口——挡 DNS rebinding（rebind 后 Host 是攻击域）；
/// - `Origin` 出现时（浏览器对跨站 fetch/XHR/WS 必带）必须是外壳或回环
///   http 源——挡任意网页对本端口的 drive-by POST/WS。GET 导航与同源
///   子资源不带 Origin，放行。
fn access_allowed(headers: &HeaderMap) -> bool {
    let host_ok = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .is_some_and(loopback_authority);
    if !host_ok {
        return false;
    }
    match headers.get("origin").and_then(|value| value.to_str().ok()) {
        None | Some("") => true,
        Some(origin) => origin_allowed(origin),
    }
}

/// Host 形如 `127.0.0.1:17301`（IPv6 字面量带方括号）；必须是回环名。
fn loopback_authority(host: &str) -> bool {
    let Some((name, port)) = host.rsplit_once(':') else {
        return false;
    };
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

/// 允许的 Origin：桌面外壳（tauri.localhost，Tauri v2 Windows 语义）与
/// 回环 http 源（DSH 页自身、dev server 的 shell 页）。
fn origin_allowed(origin: &str) -> bool {
    origin == "http://tauri.localhost"
        || origin == "https://tauri.localhost"
        || origin.starts_with("http://127.0.0.1:")
        || origin.starts_with("http://localhost:")
}

/// 局域网鉴权：cookie（常规与 WS 升级请求都带）优先，query 兜底。
fn authorized(state: &GatewayState, query: &str, headers: &HeaderMap) -> bool {
    let cookie_token = headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| {
            text.split(';')
                .find_map(|part| part.trim().strip_prefix("qx_token=").map(str::to_owned))
        })
        .unwrap_or_default();
    let token = if !cookie_token.is_empty() {
        cookie_token
    } else {
        query_param(query, "token")
    };
    state.token_valid(&token)
}

fn response_with_cookie(token: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header("location", "/")
        .header(
            "set-cookie",
            format!("qx_token={token}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax"),
        )
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// 供 commands 生成配对 URL 展示二维码：http://<bind>:<port>/qx-gate?token=…
pub fn pair_url(bind_ip: &str, port: u16, token: &str) -> String {
    format!("http://{bind_ip}:{port}/qx-gate?token={token}")
}

/// 生成 256bit hex token。
pub fn new_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 生成短 id（配对时间戳 + 随机尾巴，可读可吊销）。
pub fn new_device_id() -> String {
    use rand::Rng;
    let suffix: u32 = rand::thread_rng().gen_range(1000..10000);
    format!(
        "dev-{}-{suffix}",
        chrono::Local::now().format("%Y%m%d%H%M%S")
    )
}

#[cfg(test)]
mod tests {
    use super::{access_allowed, loopback_authority, origin_allowed};
    use axum::http::HeaderMap;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                axum::http::header::HeaderName::from_lowercase(name.as_bytes()).unwrap(),
                axum::http::header::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn host必须是回环名带端口() {
        assert!(loopback_authority("127.0.0.1:17400"));
        assert!(loopback_authority("localhost:17400"));
        assert!(loopback_authority("[::1]:17400"));
        assert!(!loopback_authority("evil.com:17400"));
        assert!(!loopback_authority("127.0.0.1"));
        assert!(!loopback_authority("127.0.0.1:"));
        assert!(!loopback_authority("127.0.0.1:abc"));
    }

    #[test]
    fn origin允许外壳与回环() {
        assert!(origin_allowed("http://tauri.localhost"));
        assert!(origin_allowed("http://127.0.0.1:5180"));
        assert!(origin_allowed("http://localhost:5180"));
        assert!(!origin_allowed("https://evil.com"));
        assert!(!origin_allowed("http://evil.com"));
        assert!(!origin_allowed("null"));
    }

    #[test]
    fn 栅栏组合判定() {
        // 常规 iframe 导航：Host 回环、无 Origin → 放行。
        assert!(access_allowed(&headers(&[("host", "127.0.0.1:17400")])));
        // 外壳 fetch：tauri.localhost → 放行。
        assert!(access_allowed(&headers(&[
            ("host", "127.0.0.1:17400"),
            ("origin", "http://tauri.localhost"),
        ])));
        // DSH 页内 WS：回环 origin → 放行。
        assert!(access_allowed(&headers(&[
            ("host", "127.0.0.1:17400"),
            ("origin", "http://127.0.0.1:17400"),
        ])));
        // 驱动式攻击：Host 对但 Origin 是任意网页 → 拒。
        assert!(!access_allowed(&headers(&[
            ("host", "127.0.0.1:17400"),
            ("origin", "https://evil.com"),
        ])));
        // DNS rebinding：Host 是攻击域 → 拒。
        assert!(!access_allowed(&headers(&[
            ("host", "evil.com:17400"),
            ("origin", "http://tauri.localhost"),
        ])));
        // 无 Host（构造残缺请求）→ 拒。
        assert!(!access_allowed(&headers(&[])));
    }
}

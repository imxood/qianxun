//! 网关本体：axum 服务 + 回环/局域网双端 + DSH 回环转发。
//!
//! 一个进程一个 axum Router，但绑两个 socket——**永远**绑 `127.0.0.1:port`
//! 给本地外壳用（DSH 页 iframe、Notes 页 fetch），**按需**绑 `bind_ip:port`
//! 给局域网用（手机扫描）。两条入口走同一份共享状态：回环请求免 qx_token
//! 但要走 Host/Origin 栅栏（挡浏览器侧 drive-by 与 DNS rebinding），
//! 局域网请求必须带 qx_token。端口按构建模式默认 23090/23091，实例内
//! 恒定——iframe URL 永不变（DSH revive 热吸收的前提）。
//!
//! LAN 的健壮性语义：配置的网卡不可绑（EasyTier 掉线等）时**降级为仅
//! 回环**，后台按 `LAN_RETRY_INTERVAL` 周期重试，网卡恢复自动上线；
//! 配置本身不动，失败绝不影响回环服务。
//!
//! DSH 0.1.2 起的浏览器鉴权由共享原语 dsh_upstream 在服务端完成
//! （token 与 cookie 都留在千寻侧，回环/局域网都一样）。

use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{ws::WebSocketUpgrade, OriginalUri, State},
    http::{HeaderMap, Request, Response, StatusCode},
    routing::any,
    Router,
};
use tokio::sync::watch;

use crate::dsh_upstream::{self, plain, query_param, Upstream};
use crate::remote::{MobileUi, RemoteDevice};

/// LAN 绑定失败后的重试周期。EasyTier 这类虚拟网卡晚于千寻启动、中途
/// 掉线都是常态：周期重试即可自愈，无需用户干预。
const LAN_RETRY_INTERVAL: Duration = Duration::from_secs(30);
/// 连续绑定失败时的日志节流：每 10 次（约 5 分钟）提醒一次；首败与
/// 「上线/掉线」状态翻转必记（见 `lan_serve_loop`）。
const LAN_FAILURE_LOG_EVERY: u32 = 10;

/// 运行中的网关句柄：停止信号 + 监督任务 + 监听地址 + 启动配置指纹。
/// `lan_addr` 经 watch 共享、可动态变化：None = 远程未启用，或配置的
/// 网卡当前不可绑（降级为仅回环 + 后台周期重试中，见 `lan_serve_loop`）。
pub struct GatewayHandle {
    pub shutdown: watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
    /// LAN 监听地址（随后台重试动态出现/消失；远程未启用时恒 None）。
    pub lan_addr: watch::Receiver<Option<SocketAddr>>,
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
    /// 移动定制层（/qx-mobile/* 的文件服务）。
    pub mobile: Arc<MobileUi>,
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

    /// `/qx-mobile/*` 的鉴权：回环入口放行（本机外壳/调试直连），
    /// 局域网入口与普通转发一致走配对鉴权。
    pub(crate) fn mobile_authorized(&self, query: &str, headers: &HeaderMap) -> bool {
        self.is_loopback(headers) || authorized(self, query, headers)
    }
}

/// 启动网关。永远绑回环 `127.0.0.1:port`；`bind_ip` 非空时同时绑局域网。
/// fingerprint = 启动配置指纹（commands::fingerprint），存入句柄供 sync 比对。
/// `upstream` 是 DSH origin（host:port）；`dsh_url` 是 DSH 就绪时打印的
/// 完整 URL（已含 `?token=`）—— 兑换 cookie 时整段打过去；None = DSH 未就绪。
/// `mobile_access_dir` 是移动定制层目录（`<DSH_HOME>/mobile-access`）。
pub async fn start(
    bind_ip: &str,
    port: u16,
    upstream: String,
    dsh_url: Option<String>,
    devices: Vec<RemoteDevice>,
    fingerprint: u64,
    mobile_access_dir: std::path::PathBuf,
) -> Result<GatewayHandle, String> {
    let loopback_bind: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|cause| format!("回环地址不合法：{cause}"))?;
    // 定制目录尽力创建：方便用户/agent 直接往里放 custom.css/js。
    let _ = std::fs::create_dir_all(&mobile_access_dir);
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
        mobile: Arc::new(MobileUi::new(mobile_access_dir)),
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
        // 移动定制层：自有路由先于兜底（不转发 DSH，避免内部面漏给上游）。
        .route("/qx-mobile/info", any(super::mobile_ui::info))
        .route("/qx-mobile/version", any(super::mobile_ui::version))
        .route("/qx-mobile/bootstrap.js", any(super::mobile_ui::bootstrap))
        .route("/qx-mobile/custom.css", any(super::mobile_ui::custom_css))
        .route("/qx-mobile/custom.js", any(super::mobile_ui::custom_js))
        .route("/qx-mobile/{*rest}", any(super::mobile_ui::unknown))
        .route("/{*rest}", any(handler))
        .route("/", any(handler))
        .with_state(state.clone());

    // 可选绑局域网——**绑定失败不拖累回环**：EasyTier 这类虚拟网卡晚于
    // 千寻启动、中途掉线是常态（os error 10049 曾让整个网关起不来，
    // DSH 页报「回环代理尚未监听」且重启 DSH 无解）。首绑失败 → 记一次
    // warn 并降级为仅回环，监督任务里按 LAN_RETRY_INTERVAL 周期重试，
    // 网卡恢复后自动监听。bind_ip 与回环同地址（127.0.0.1）时跳过 LAN，
    // 避免双绑同一 socket 报错。
    let (lan_listener, lan_bind) = if !bind_ip.is_empty() && bind_ip != "127.0.0.1" {
        let lan_bind: SocketAddr = format!("{bind_ip}:{port}")
            .parse()
            .map_err(|cause| format!("绑定地址不合法（{bind_ip}:{port}）：{cause}"))?;
        match tokio::net::TcpListener::bind(lan_bind).await {
            Ok(listener) => (Some(listener), Some(lan_bind)),
            Err(cause) => {
                crate::logging::log(
                    "warn",
                    &format!(
                        "局域网 {lan_bind} 暂不可用（{cause}）；已降级为仅回环，每 30s 自动重试，网卡恢复后自动监听"
                    ),
                );
                (None, Some(lan_bind))
            }
        }
    } else {
        (None, None)
    };
    let lan_addr = lan_listener
        .as_ref()
        .and_then(|listener| listener.local_addr().ok());
    let (lan_status_tx, lan_status_rx) = watch::channel(lan_addr);
    // 首绑成功 = 无失败计数；首绑失败 = start() 已记一次 warn，计数从 1 起。
    let lan_failures = u32::from(lan_addr.is_none());

    // 单个监督任务托起所有 server，共享同一停止信号——任一 server 退出就
    // 结束整组（restart 由 sync 层重新建）。LAN 侧不是一次性 serve 而是
    // 「重试 + 服务」循环：绑定失败回到定时重试，网卡恢复即自动上线。
    let (shutdown, mut signal) = watch::channel(false);
    let lan_retry: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = match lan_bind
    {
        Some(bind) => Box::pin(lan_serve_loop(
            lan_listener,
            bind,
            app.clone(),
            lan_status_tx,
            signal.clone(),
            lan_failures,
        )),
        None => Box::pin(std::future::pending()),
    };
    let task = tokio::spawn(async move {
        // axum 0.8 的 Serve 实现 IntoFuture（不是直接 Future），select! 接受
        // 两者但 Either 要 Future 形态——用 .into_future() 统一一下。
        let loopback_server = axum::serve(loopback_listener, app).into_future();
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
            _ = lan_retry => {}
        }
    });
    Ok(GatewayHandle {
        shutdown,
        task,
        lan_addr: lan_status_rx,
        loopback_addr,
        fingerprint,
    })
}

/// 局域网端口的周期重试 + 服务循环。`listener` 是首绑结果（成功 = 直接
/// 上线；None = 首绑失败，从重试开始）。绑定失败只记节流日志（首败已在
/// `start` 记过 warn，此后每 LAN_FAILURE_LOG_EVERY 次提醒一次，约 5 分
/// 钟一条，避免 30s 一条刷屏）；上线/掉线的状态翻转必记，并在
/// `lan_status` 上广播，供远程页状态与自检读取。
async fn lan_serve_loop(
    mut listener: Option<tokio::net::TcpListener>,
    bind: SocketAddr,
    app: Router,
    lan_status: watch::Sender<Option<SocketAddr>>,
    mut signal: watch::Receiver<bool>,
    mut failures: u32,
) {
    loop {
        if *signal.borrow_and_update() {
            return;
        }
        // 优先用首绑的 listener；没有（首绑失败或上一轮服务结束）则现绑。
        let bound = match listener.take() {
            Some(l) => l,
            None => match tokio::net::TcpListener::bind(bind).await {
                Ok(l) => l,
                Err(cause) => {
                    failures += 1;
                    if failures.is_multiple_of(LAN_FAILURE_LOG_EVERY) {
                        crate::logging::log(
                            "warn",
                            &format!(
                                "局域网网关仍不可用（{bind}）：{cause}（已连续 {failures} 次，节流提醒）"
                            ),
                        );
                    }
                    tokio::select! {
                        _ = signal.changed() => return,
                        _ = tokio::time::sleep(LAN_RETRY_INTERVAL) => {}
                    }
                    continue;
                }
            },
        };
        failures = 0;
        let _ = lan_status.send(Some(bind));
        crate::logging::log("info", &format!("局域网网关已监听 {bind}（网卡已恢复）"));
        if let Err(cause) = axum::serve(bound, app.clone()).await {
            crate::logging::log("warn", &format!("局域网网关退出：{cause}"));
        }
        let _ = lan_status.send(None);
        crate::logging::log("warn", "局域网网关退出，将按周期重试绑定");
        tokio::select! {
            _ = signal.changed() => return,
            _ = tokio::time::sleep(LAN_RETRY_INTERVAL) => {}
        }
    }
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
    // 手机端经网关拿到的 DSH 页面在此注入移动定制层（回环桌面页保持纯净）。
    let method = request.method().clone();
    let response = dsh_upstream::forward(&state.upstream, request).await;
    inject_mobile_layer_into(method, response).await
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

/// HTML 注入缓冲上限：DSH 的 index.html 远小于此；超过则放弃注入原样透传。
const HTML_INJECT_LIMIT: usize = 2 * 1024 * 1024;

/// 注入移动定制层的引导标签（外链同源资源，CSP `self` 放行；defer 不阻塞首屏）。
const MOBILE_LAYER_SNIPPET: &str = "<link rel=\"stylesheet\" href=\"/qx-mobile/custom.css\" /><script src=\"/qx-mobile/bootstrap.js\" defer></script>";

/// 满足「GET + 200 + text/html + 含 </head>」时，把定制层标签插到 </head> 前。
/// 纯函数便于测试；任何不满足都返回 None（原样透传，绝不阻断/破坏页面）。
fn inject_mobile_layer(
    method: &axum::http::Method,
    status: u16,
    content_type: &str,
    html: &str,
) -> Option<String> {
    if *method != axum::http::Method::GET || status != 200 {
        return None;
    }
    if !content_type.to_ascii_lowercase().starts_with("text/html") {
        return None;
    }
    let head_end = html.to_ascii_lowercase().find("</head>")?;
    Some(format!(
        "{}{MOBILE_LAYER_SNIPPET}{}",
        &html[..head_end],
        &html[head_end..]
    ))
}

/// 局域网回包的移动层注入：命中「200 + text/html」才缓冲改写（≤2MiB），
/// 其余一律原样透传（SSE/流式响应不受影响）。无 content-length 且超限的
/// 罕见分块页在缓冲阶段失败——此时响应体已消费，按上游中断降级并落日志。
async fn inject_mobile_layer_into(
    method: axum::http::Method,
    response: Response<Body>,
) -> Response<Body> {
    let headers = response.headers().clone();
    let Some(content_type) = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    else {
        return response;
    };
    if response.status() != StatusCode::OK
        || !content_type.to_ascii_lowercase().starts_with("text/html")
    {
        return response;
    }
    // 明确超限：不消费响应体，原样流式透传。
    if headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > HTML_INJECT_LIMIT)
    {
        return response;
    }
    let cache_control = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, HTML_INJECT_LIMIT).await {
        Ok(bytes) => bytes,
        Err(cause) => {
            crate::logging::log(
                "warn",
                &format!("移动层注入放弃（HTML 超限/响应中断）：{cause}"),
            );
            return plain(StatusCode::BAD_GATEWAY, "DSH 页面超限，注入失败");
        }
    };
    let text = match String::from_utf8(bytes.to_vec()) {
        Ok(text) => text,
        // 非 UTF-8（理论上游不应出现，防御压缩字节漏网）：**原样透传**，
        // 绝不能拿错误页顶掉正常页面——手机用户看到的就是整个响应体。
        Err(_) => {
            crate::logging::log("warn", "DSH 页面非 UTF-8，跳过注入（原样透传）");
            let mut builder = Response::builder()
                .status(parts.status)
                .header(axum::http::header::CONTENT_TYPE, content_type);
            if let Some(cache) = &cache_control {
                builder = builder.header(axum::http::header::CACHE_CONTROL, cache);
            }
            return builder
                .body(Body::from(bytes))
                .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"));
        }
    };
    // 注入后内容已改写：etag/last-modified 一律丢弃，禁止旧实体复用；
    // cache-control 保留。注入失败（无 </head>）则原文透传。
    let injected =
        inject_mobile_layer(&method, parts.status.as_u16(), &content_type, &text).unwrap_or(text);
    let mut builder = Response::builder()
        .status(parts.status)
        .header(axum::http::header::CONTENT_TYPE, content_type);
    if let Some(cache) = cache_control {
        builder = builder.header(axum::http::header::CACHE_CONTROL, cache);
    }
    builder
        .body(Body::from(injected))
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
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
    use super::{access_allowed, inject_mobile_layer, loopback_authority, origin_allowed, start};
    use axum::http::{HeaderMap, Method};

    const GET: Method = Method::GET;

    /// 回归：LAN 绑定失败（曾因 EasyTier 掉线 os error 10049 让整个网关
    /// 起不来，DSH 页报「回环代理尚未监听」且重启 DSH 无解）必须降级为
    /// 仅回环，回环照常服务；LAN 处于重试态（lan_addr = None）。
    #[tokio::test]
    async fn lan_绑定失败时降级为仅回环且回环照常服务() {
        // 占住 127.0.0.2 的端口，模拟「配置的网卡地址当前不可绑」。
        let blocker = std::net::TcpListener::bind("127.0.0.2:0").unwrap();
        let port = blocker.local_addr().unwrap().port();
        let handle = start(
            "127.0.0.2",
            port,
            "127.0.0.1:0".to_owned(),
            None,
            Vec::new(),
            0,
            std::env::temp_dir().join("qianxun-gateway-degrade-test"),
        )
        .await
        .expect("LAN 绑定失败不应拖垮网关启动");
        assert!(handle.lan_addr.borrow().is_none(), "LAN 应处于降级重试态");
        // 回环照常可达。
        tokio::net::TcpStream::connect(handle.loopback_addr)
            .await
            .expect("回环必须照常监听");
        let _ = handle.shutdown.send(true);
    }

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
    fn 注入_在head结束标签前插入定制层() {
        let html = "<html><head><title>t</title></head><body></body></html>";
        let injected =
            inject_mobile_layer(&GET, 200, "text/html; charset=utf-8", html).expect("应注入");
        assert!(injected.contains(super::MOBILE_LAYER_SNIPPET));
        let position = injected.find(super::MOBILE_LAYER_SNIPPET).unwrap();
        assert!(injected[..position].contains("</title>"));
        let after = &injected[position + super::MOBILE_LAYER_SNIPPET.len()..];
        assert!(after.starts_with("</head>"));
        assert!(after.ends_with("<body></body></html>"));
    }

    #[test]
    fn 注入_大小写不敏感且幂等形状() {
        let html = "<html><HEAD></HEAD><body></body></html>";
        let injected = inject_mobile_layer(&GET, 200, "text/html", html).expect("应注入");
        // 插入点在「小写化后首个 </head>」之前，原文其余部分逐字节保留。
        assert!(injected.ends_with("</HEAD><body></body></html>"));
    }

    #[test]
    fn 不注入_条件不满足() {
        let html = "<html><head></head><body></body></html>";
        // 非 GET
        assert!(inject_mobile_layer(&Method::POST, 200, "text/html", html).is_none());
        // 非 200
        assert!(inject_mobile_layer(&GET, 404, "text/html", html).is_none());
        // 非 HTML
        assert!(inject_mobile_layer(&GET, 200, "application/json", "{}").is_none());
        // 没有 </head>（片段/流式页面）
        assert!(inject_mobile_layer(&GET, 200, "text/html", "<p>fragment</p>").is_none());
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

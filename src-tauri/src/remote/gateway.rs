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
use crate::remote::{text_response, MobileUi, RemoteDevice};

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
    /// 本地服务反代监听地址（docs/20；未启用/未绑定时恒 None）。
    pub proxy_addr: watch::Receiver<Option<SocketAddr>>,
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
    /// 一次性配对 code 表（docs/18 A4）。`/qx-gate?code=` 凭它兑换长期 token。
    pub pair_codes: PairCodes,
}

impl GatewayState {
    /// 设备鉴权（仅局域网入口走）。
    ///
    /// 常量时间比较（docs/18 A4 加固）：逐 `.any(|d| d.token == presented)`
    /// 会在首个不匹配字符处短路，理论上可被计时攻击逐字节试探。
    fn token_valid(&self, presented: &str) -> bool {
        !presented.is_empty()
            && self.devices.iter().any(|device| {
                !device.revoked && constant_time_eq(device.token.as_bytes(), presented.as_bytes())
            })
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
/// `proxy` 是本地服务反代面配置（docs/20）：启用时在 LAN 网卡上另绑
/// `port + 2`，整源代理给回环目标。
/// 参数全部是「启动快照」字段，sync 指纹比对后一次性传入——结构体化
/// 只是把同一组字段换个名字，故允许超参。
#[allow(clippy::too_many_arguments)]
pub async fn start(
    bind_ip: &str,
    port: u16,
    upstream: String,
    dsh_url: Option<String>,
    dsh_cookie: Option<String>,
    devices: Vec<RemoteDevice>,
    fingerprint: u64,
    mobile_access_dir: std::path::PathBuf,
    proxy: super::proxy::ProxyLaunch,
) -> Result<GatewayHandle, String> {
    let loopback_bind: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|cause| format!("回环地址不合法：{cause}"))?;
    // 定制目录尽力创建：方便用户/agent 直接往里放 custom.css/js。
    let _ = std::fs::create_dir_all(&mobile_access_dir);
    // 共享实例（复用用户自己跑着的 dsh web）：cookie 已经自签好，直接用；
    // 没有 token 可兑换，故不走 exchange 那条路。
    let upstream_state = match dsh_cookie.clone() {
        Some(cookie) => Upstream::with_shared_cookie(upstream.clone(), cookie),
        None => Upstream::new(upstream.clone(), dsh_url.clone()),
    };
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
        // 与 remote_add 共用进程级同一张表：配对 URL 在网关启动前就已签发。
        pair_codes: global_pair_codes().clone(),
    };
    // 顺手把登录兑换做掉：首个请求就不必等一次兑换往返。
    // 共享实例的 cookie 是自签的，本来就没有 dsh_url 可兑换。
    if dsh_cookie.is_some() {
        crate::logging::log("info", "网关使用自签 cookie 连接共享的 DSH 实例");
    } else if dsh_url.is_some() {
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
        // 桌面外壳层：注入 DSH 页的外链拦截脚本（仅回环入口可取）。
        .route("/qx-shell/links.js", any(shell_links))
        .route("/{*rest}", any(handler))
        .route("/", any(handler))
        .with_state(state.clone());
    // 发布到 RouterHandle（docs/19 §3.3）：隧道端每流取最新版本——
    // 配对/吊销/DSH revive 后 sync 重建网关时会再次发布新表。
    super::router_handle().store(std::sync::Arc::new(app.clone()));

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

    // 本地服务反代面（docs/20）：仅 LAN 入口需要；绑失败不拖累网关
    // （warn 降级 + 与 LAN 相同的周期重试自愈）。端口 = 网关端口 + 2。
    let (proxy_listener, proxy_bind) = match (lan_bind, proxy.enabled) {
        (Some(lan), true) => match super::proxy::ProxyLaunch::addr_for(lan) {
            Some(proxy_sock) => match tokio::net::TcpListener::bind(proxy_sock).await {
                Ok(listener) => (Some(listener), Some(proxy_sock)),
                Err(cause) => {
                    crate::logging::log(
                            "warn",
                            &format!(
                                "本地服务反代 {proxy_sock} 暂不可用（{cause}）；已降级，每 30s 自动重试"
                            ),
                        );
                    (None, Some(proxy_sock))
                }
            },
            None => (None, None),
        },
        _ => (None, None),
    };
    let proxy_listen_addr = proxy_listener
        .as_ref()
        .and_then(|listener| listener.local_addr().ok());
    let (proxy_status_tx, proxy_status_rx) = watch::channel(proxy_listen_addr);
    let proxy_failures = u32::from(proxy_listen_addr.is_none());
    // 停止信号先建：LAN 与反代两条重试循环都要 clone 它。
    let (shutdown, mut signal) = watch::channel(false);
    let proxy_router = super::proxy::router(state.clone(), port.saturating_add(2), proxy);
    let proxy_retry: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        match proxy_bind {
            Some(bind) => Box::pin(lan_serve_loop(
                proxy_listener,
                bind,
                proxy_router,
                proxy_status_tx,
                signal.clone(),
                proxy_failures,
                "本地服务反代",
            )),
            None => Box::pin(std::future::pending()),
        };

    // 单个监督任务托起所有 server，共享同一停止信号——任一 server 退出就
    // 结束整组（restart 由 sync 层重新建）。LAN 侧不是一次性 serve 而是
    // 「重试 + 服务」循环：绑定失败回到定时重试，网卡恢复即自动上线。
    let lan_retry: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = match lan_bind
    {
        Some(bind) => Box::pin(lan_serve_loop(
            lan_listener,
            bind,
            app.clone(),
            lan_status_tx,
            signal.clone(),
            lan_failures,
            "局域网网关",
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
            _ = proxy_retry => {}
        }
    });
    Ok(GatewayHandle {
        shutdown,
        task,
        lan_addr: lan_status_rx,
        loopback_addr,
        proxy_addr: proxy_status_rx,
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
    label: &'static str,
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
                                "{label}仍不可用（{bind}）：{cause}（已连续 {failures} 次，节流提醒）"
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
        crate::logging::log("info", &format!("{label}已监听 {bind}（网卡已恢复）"));
        if let Err(cause) = axum::serve(bound, app.clone()).await {
            crate::logging::log("warn", &format!("{label}退出：{cause}"));
        }
        let _ = lan_status.send(None);
        crate::logging::log("warn", &format!("{label}退出，将按周期重试绑定"));
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
        // 桌面外壳层注入：DSH 页内的外链点击由拦截脚本转发给千寻外壳
        // （WebView2 吞掉一切弹窗请求，链接点不开——见 window.rs 的
        // window_open_external 注释）。非 HTML/SSE 一律原样透传。
        let method = request.method().clone();
        let response = dsh_upstream::forward(&state.upstream, request).await;
        return inject_layer_into(method, response, inject_desktop_layer).await;
    }

    // 局域网入口：配对 → 鉴权 → 转发（含 SSE 流响应）。
    if path == "/qx-gate" {
        // 首选一次性 code（docs/18 A4）：二维码/URL 里带的是 5 分钟有效的
        // 一次性凭据，兑换后立刻消费，长期 token 只进 cookie 不进 URL。
        let code = query_param(&query, "code");
        if let Some(token) = state.pair_codes.redeem(&code) {
            if state.token_valid(&token) {
                return response_with_cookie(&token);
            }
            return plain(StatusCode::UNAUTHORIZED, "配对码对应的设备已被吊销");
        }
        // 兼容旧版客户端仍直传长期 token；新生成的配对链接不再走这条路径。
        let token = query_param(&query, "token");
        if state.token_valid(&token) {
            return response_with_cookie(&token);
        }
        return plain(
            StatusCode::UNAUTHORIZED,
            "配对码无效、已过期或已被使用；请重新扫码获取",
        );
    }
    if !authorized(&state, &query, &request.headers().clone()) {
        return plain(
            StatusCode::UNAUTHORIZED,
            "未配对设备：请用千寻生成的配对链接打开 /qx-gate?token=…",
        );
    }
    // 手机端经网关拿到的 DSH 页面在此注入移动定制层（回环桌面页走
    // 桌面外壳层注入，见 handler 的回环分支）。
    let method = request.method().clone();
    let response = dsh_upstream::forward(&state.upstream, request).await;
    inject_layer_into(method, response, inject_mobile_layer).await
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
/// 服务端入口的第三条通路：边缘盖章头（X-QX-Edge/Device）——边缘已完成
/// 设备鉴权并结构性剥离入站伪造（docs/19 §4.3），常数时间比对进程密钥。
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
    state.token_valid(&token) || edge_stamped(headers)
}

/// 边缘盖章识别：X-QX-Device 非空 + X-QX-Edge 与进程密钥常数时间一致。
fn edge_stamped(headers: &HeaderMap) -> bool {
    let device = headers
        .get("x-qx-device")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if device.is_empty() {
        return false;
    }
    headers
        .get("x-qx-edge")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|secret| constant_time_eq(secret.as_bytes(), super::edge_secret().as_bytes()))
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

/// 注入桌面外壳层的引导标签：外链拦截脚本（千寻外壳 iframe 专用，
/// 与外壳握手后才接管点击，普通浏览器直开网关页时零行为）。
const DESKTOP_LAYER_SNIPPET: &str = "<script src=\"/qx-shell/links.js\" defer></script>";

/// 桌面外壳层拦截脚本（内嵌二进制，随千寻发布，no-cache 下发）。
const DESKTOP_LINKS_JS: &str = include_str!("assets/qx-shell-links.js");

/// `/qx-shell/links.js`：仅回环入口可取（桌面外壳 iframe 专用；
/// 局域网设备走移动层，不给内部脚本面）。
async fn shell_links(State(state): State<GatewayState>, headers: HeaderMap) -> Response<Body> {
    if !state.is_loopback(&headers) {
        return plain(StatusCode::UNAUTHORIZED, "仅本机外壳可获取桌面外壳层");
    }
    text_response(
        DESKTOP_LINKS_JS,
        "text/javascript; charset=utf-8",
        "no-cache",
    )
}

/// 满足「GET + 200 + text/html + 含 </head>」时，把定制层标签插到 </head> 前。
/// 纯函数便于测试；任何不满足都返回 None（原样透传，绝不阻断/破坏页面）。
fn inject_layer(
    method: &axum::http::Method,
    status: u16,
    content_type: &str,
    html: &str,
    snippet: &str,
) -> Option<String> {
    if *method != axum::http::Method::GET || status != 200 {
        return None;
    }
    if !content_type.to_ascii_lowercase().starts_with("text/html") {
        return None;
    }
    let head_end = html.to_ascii_lowercase().find("</head>")?;
    Some(format!(
        "{}{snippet}{}",
        &html[..head_end],
        &html[head_end..]
    ))
}

/// 移动定制层注入（LAN 入口）。
fn inject_mobile_layer(
    method: &axum::http::Method,
    status: u16,
    content_type: &str,
    html: &str,
) -> Option<String> {
    inject_layer(method, status, content_type, html, MOBILE_LAYER_SNIPPET)
}

/// 桌面外壳层注入（回环入口）。
fn inject_desktop_layer(
    method: &axum::http::Method,
    status: u16,
    content_type: &str,
    html: &str,
) -> Option<String> {
    inject_layer(method, status, content_type, html, DESKTOP_LAYER_SNIPPET)
}

/// 回包的定制层注入：命中「200 + text/html」才缓冲改写（≤2MiB），
/// 其余一律原样透传（SSE/流式响应不受影响）。无 content-length 且超限的
/// 罕见分块页在缓冲阶段失败——此时响应体已消费，按上游中断降级并落日志。
/// `inject` 是纯注入函数（移动层/桌面外壳层各一），生产与测试共用同一实现。
async fn inject_layer_into(
    method: axum::http::Method,
    response: Response<Body>,
    inject: fn(&axum::http::Method, u16, &str, &str) -> Option<String>,
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
                &format!("定制层注入放弃（HTML 超限/响应中断）：{cause}"),
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
    let injected = inject(&method, parts.status.as_u16(), &content_type, &text).unwrap_or(text);
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
/// 配对 URL：携带一次性短期 code 而非长期 token（docs/18 A4）。
///
/// 长期凭据只落在兑换后的 cookie 里，不再出现在二维码、URL 与日志中。
pub fn pair_url_with_code(bind_ip: &str, port: u16, code: &str) -> String {
    format!("http://{bind_ip}:{port}/qx-gate?code={code}")
}

/// 生成 256bit hex token。
pub fn new_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 配对一次性 code 的有效期（docs/18 A4）。
///
/// 原实现把**长期 token 本身**放进二维码/URL，且 `Max-Age` 是一年、记录无过期
/// 字段——凭据长期有效、明文展示、还进了 URL（会进浏览器历史与服务器日志）。
/// 改成「一次性短期 code 换 cookie」后，URL 里出现的只是一次性凭据。
const PAIR_CODE_TTL: Duration = Duration::from_secs(300);

/// 一次性的配对兑换凭据（docs/18 A4）。
#[derive(Clone, Debug)]
pub struct PairCode {
    /// 兑换成功后写入 cookie 的长期 token。
    pub token: String,
    pub expires_at: std::time::Instant,
}

/// 进程内的一次性 code 表：code → 目标 token。
///
/// 刻意只放内存：不落盘，进程退出即全部失效（用户重新扫码即可）。
/// 消费后立即移除，天然「一次性」。
#[derive(Clone, Default)]
pub struct PairCodes(Arc<std::sync::Mutex<std::collections::HashMap<String, PairCode>>>);

/// 全局 code 表：`remote_add` 在网关启动前就要签发 code（配对 URL 由
/// 桌面端立即返回给用户扫码），而 `GatewayState` 是网关启动后才构造的。
/// 进程内单例，两者共用同一张表。
static GLOBAL_PAIR_CODES: std::sync::OnceLock<PairCodes> = std::sync::OnceLock::new();

/// 取得全局一次性 code 表。
pub fn global_pair_codes() -> &'static PairCodes {
    GLOBAL_PAIR_CODES.get_or_init(PairCodes::new)
}

impl PairCodes {
    pub fn new() -> Self {
        Self::default()
    }

    /// 为某个设备 token 签发一次性 code（覆盖同 token 的旧 code）。
    pub fn issue(&self, token: &str) -> String {
        use rand::RngCore;
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        // 小写十六进制，便于手机键盘手输。
        let code = format!("{code:02x}", code = bytes[0])
            + &bytes[1..]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.retain(|_, entry| entry.expires_at > std::time::Instant::now());
        guard.insert(
            code.clone(),
            PairCode {
                token: token.to_owned(),
                expires_at: std::time::Instant::now() + PAIR_CODE_TTL,
            },
        );
        code
    }

    /// 兑换：命中且未过期则消费（一次性）并返回目标 token。
    pub fn redeem(&self, presented: &str) -> Option<String> {
        if presented.is_empty() {
            return None;
        }
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 常量时间比较，避免按前缀逐字节试探（docs/18 A4 附带的加固）。
        let matched = guard
            .keys()
            .find(|candidate| constant_time_eq(candidate.as_bytes(), presented.as_bytes()));
        let key = matched?.clone();
        let entry = guard.get(&key)?;
        if entry.expires_at <= std::time::Instant::now() {
            guard.remove(&key);
            return None;
        }
        // 先取出 token 再移除：一次性语义的兑现点就在这次 remove。
        let token = entry.token.clone();
        guard.remove(&key);
        Some(token)
    }

    /// 存活 code 数量：供测试断言「一次性」语义，生产路径不需要。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 长度不等即 false；等长则按字节累积异或，避免早退泄露前缀长度信息。
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
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
    use super::{
        access_allowed, constant_time_eq, inject_mobile_layer, loopback_authority, origin_allowed,
        pair_url_with_code, start, PairCodes,
    };
    use axum::http::{HeaderMap, Method};

    /// docs/18 A4：配对 code 必须一次性、短时效、且不与长期 token 混同。
    #[test]
    fn 配对code一次性且短时效() {
        let codes = PairCodes::new();
        let token = "t".repeat(64);
        let code = codes.issue(&token);
        assert_eq!(code.len(), 32, "code 应为 16 字节 hex");
        assert!(!code.contains(&token), "code 不得内嵌长期 token");
        assert_eq!(codes.len(), 1);

        // 首次兑换成功，拿到长期 token
        assert_eq!(codes.redeem(&code), Some(token.clone()));
        // 一次性：同一个 code 再来直接失效
        assert_eq!(codes.redeem(&code), None);
        assert!(codes.is_empty());

        // 错误 code / 空串都不放行
        let code2 = codes.issue(&token);
        assert_eq!(codes.redeem(""), None);
        assert_eq!(codes.redeem(&"0".repeat(32)), None);
        // 正确 code 仍可用（前面的错误尝试没有误消费）
        assert_eq!(codes.redeem(&code2), Some(token));
    }

    /// 过期 code 必须被拒（构造已过期的条目而不真等 5 分钟）。
    #[test]
    fn 过期配对code被拒() {
        let codes = PairCodes::new();
        let code = codes.issue("t");
        // 直接把唯一条目的到期时间改到过去，模拟 TTL 流逝。
        {
            let mut guard = codes.0.lock().unwrap();
            for entry in guard.values_mut() {
                entry.expires_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
            }
        }
        assert_eq!(codes.redeem(&code), None);
        assert!(codes.is_empty(), "过期条目应被清理");
    }

    #[test]
    fn 常量时间比较语义正确() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"), "长度不等直接 false");
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn 配对url只带code不带token() {
        let url = pair_url_with_code("192.168.1.9", 17400, "abc123");
        assert_eq!(url, "http://192.168.1.9:17400/qx-gate?code=abc123");
        assert!(!url.contains("token"), "URL 不得出现长期 token 参数");
    }

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
            None,
            Vec::new(),
            0,
            std::env::temp_dir().join("qianxun-gateway-degrade-test"),
            crate::remote::proxy::ProxyLaunch::default(),
        )
        .await
        .expect("LAN 绑定失败不应拖垮网关启动");
        assert!(handle.lan_addr.borrow().is_none(), "LAN 应处于降级重试态");
        assert!(
            handle.proxy_addr.borrow().is_none(),
            "LAN 降级时反代面也不应监听"
        );
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
    fn 桌面外壳层_在head结束标签前插入拦截脚本() {
        let html = "<html><head><title>t</title></head><body></body></html>";
        let injected = super::inject_desktop_layer(&GET, 200, "text/html", html).expect("应注入");
        assert!(injected.contains(super::DESKTOP_LAYER_SNIPPET));
        assert!(injected.contains("qx-shell/links.js"));
        assert!(!injected.contains("qx-mobile"));
        let position = injected.find(super::DESKTOP_LAYER_SNIPPET).unwrap();
        assert!(injected[..position].contains("</title>"));
        assert!(injected[position + super::DESKTOP_LAYER_SNIPPET.len()..].starts_with("</head>"));
    }

    #[test]
    fn 桌面外壳层_条件不满足不注入() {
        let html = "<html><head></head><body></body></html>";
        // 非 GET / 非 200 / 非 HTML 一律原样（None = 调用侧透传）。
        assert!(super::inject_desktop_layer(&Method::POST, 200, "text/html", html).is_none());
        assert!(super::inject_desktop_layer(&GET, 302, "text/html", html).is_none());
        assert!(super::inject_desktop_layer(&GET, 200, "text/css", html).is_none());
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

    /// 端到端（mock 上游替身 DSH）：回环入口的 HTML 页被注入桌面外壳层；
    /// `/qx-shell/links.js` 仅回环 Host 可取（LAN Host 视作未配对设备 401）；
    /// 非 HTML 响应原样透传。真实 axum 服务 + 真实 HTTP 客户端。
    #[tokio::test]
    async fn 回环入口注入桌面外壳层且脚本仅回环可取() {
        use axum::response::Html;
        use axum::{routing::get, Router};

        // 占个回环端口拿到空闲端口号再让网关去绑（先绑后放的窗口极小）。
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        // mock 上游：替身 DSH 的首页（HTML）与一个纯文本端点。
        let upstream = Router::new()
            .route(
                "/",
                get(|| async {
                    Html("<html><head><title>dsh</title></head><body>hi</body></html>")
                }),
            )
            .route("/api/data", get(|| async { "plain-ok" }))
            .into_make_service();
        let up_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = up_listener.local_addr().unwrap();
        let upstream_task = tokio::spawn(async move {
            axum::serve(up_listener, upstream).await.unwrap();
        });

        let handle = start(
            "",
            port,
            upstream_addr.to_string(),
            None,
            None,
            Vec::new(),
            0,
            std::env::temp_dir().join("qianxun-gateway-desktop-layer-test"),
            crate::remote::proxy::ProxyLaunch::default(),
        )
        .await
        .expect("网关应启动");

        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}", handle.loopback_addr.port());

        // 1) 回环 GET 首页：注入桌面外壳层标签（移动层不出现）。
        let html = client
            .get(&base)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(html.contains(r#"<script src="/qx-shell/links.js" defer></script>"#));
        assert!(!html.contains("qx-mobile"));

        // 2) 拦截脚本本尊：回环可取，内容是外壳链接桥。
        let script = client
            .get(format!("{base}/qx-shell/links.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(script.status(), 200);
        assert!(script.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/javascript"));
        assert!(script.text().await.unwrap().contains("__qxShell"));

        // 3) 同一路由换 LAN Host：is_loopback 不成立 → 未配对 401（不漏脚本）。
        let denied = client
            .get(format!("{base}/qx-shell/links.js"))
            .header(
                "host",
                format!("192.168.1.10:{}", handle.loopback_addr.port()),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401);

        // 4) 非 HTML 响应原样透传，绝无注入痕迹。
        let plain = client
            .get(format!("{base}/api/data"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(plain, "plain-ok");

        let _ = handle.shutdown.send(true);
        upstream_task.abort();
    }
}

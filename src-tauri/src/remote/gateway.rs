//! 网关本体：axum 服务 + DSH 回环转发 + WS 双向桥。
//!
//! DSH 0.1.2 起自带浏览器鉴权：就绪 URL 里的启动 token 只用于 `GET /`
//! 兑换一张 Host 绑定的签名 cookie，`/api/*` 与 WS 升级只认 cookie
//! （token 直接放 query 不被接受）。所以网关在启动时（及 cookie 缺失
//! /失效时）替远程设备完成兑换：token 与 cookie 都留在服务端，手机
//! 侧只持有千寻自己的 `qx_token`。

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, Request, Response, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::http::Request as HttpRequest;

use crate::remote::RemoteDevice;

/// DSH 签名 cookie 的名字前缀（@deepseek-ai/dsh-client-connection）。
const DSH_COOKIE_PREFIX: &str = "dsh-auth-";

/// 单次兑换/转发允许的耗时；网关在回环与 EasyTier 网内，给小超时。
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// 转发请求体的缓冲上限（64 MiB）：重试需要可重放的体，但拒绝无界内存。
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// 跳过转发的逐跳头（RFC 7230；cookie/set-cookie 由两端各自管理）。
const HOP_HEADERS: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "set-cookie",
];

/// 手机侧不该漏给 DSH 的头：
/// - `host`：DSH 的 Host/Origin 栅栏按 Host 判定，必须让它看到网关→DSH
///   的真实 authority（reqwest 按 URL 自动设置）；
/// - `cookie`：`qx_token` 是网关层凭据，不归 DSH；
/// - `origin`：手机页面的 origin 是网关，原样转发会被 DSH 栅栏 403。
const SKIP_UPSTREAM_HEADERS: [&str; 3] = ["host", "cookie", "origin"];

/// 运行中的网关句柄：停止信号 + 任务 + 实际监听地址（绑定成功的证明）+
/// 启动配置指纹（sync 用它判断「配置没变才保留」，设备表是启动快照）。
pub struct GatewayHandle {
    pub shutdown: watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
    pub local_addr: std::net::SocketAddr,
    pub fingerprint: u64,
}

/// 网关共享状态：DSH origin（host:port）+ 设备表快照（每次启动重建）+
/// 服务端持有的 DSH 启动 token 与其兑换出的签名 cookie。
#[derive(Clone)]
pub struct GatewayState {
    pub upstream: String,
    pub devices: Arc<Vec<RemoteDevice>>,
    pub client: reqwest::Client,
    /// DSH 就绪 URL 里的启动 token；空 = 旧版 DSH，无浏览器鉴权。
    pub launch_token: Option<String>,
    /// 兑换出的 `dsh-auth-*` cookie（Name=Value），跨请求复用。
    dsh_cookie: Arc<tokio::sync::Mutex<Option<String>>>,
}

impl GatewayState {
    fn upstream_host(&self) -> &str {
        self.upstream
            .trim_start_matches("http://")
            .trim_end_matches('/')
    }

    /// cookie/query 命中任一未吊销 token？
    fn token_valid(&self, presented: &str) -> bool {
        !presented.is_empty()
            && self
                .devices
                .iter()
                .any(|device| !device.revoked && device.token == presented)
    }

    /// 取回上游可用的 DSH cookie：优先缓存，缺失时兑换一次。
    async fn ensure_cookie(&self) -> Option<String> {
        let mut cached = self.dsh_cookie.lock().await;
        if let Some(cookie) = cached.as_ref() {
            return Some(cookie.clone());
        }
        let cookie = exchange_cookie(self).await?;
        *cached = Some(cookie.clone());
        Some(cookie)
    }

    /// 缓存作废并强制重兑（上游 401 = cookie 失效，比如 DSH_HOME 被换）。
    async fn refresh_cookie(&self) -> Option<String> {
        let mut cached = self.dsh_cookie.lock().await;
        *cached = None;
        let cookie = exchange_cookie(self).await?;
        *cached = Some(cookie.clone());
        Some(cookie)
    }
}

/// 用启动 token 向 DSH `GET /?token=…` 兑换签名 cookie（Name=Value）。
/// 失败返回 None（DSH 未就绪/旧版无鉴权/token 不对）。
async fn exchange_cookie(state: &GatewayState) -> Option<String> {
    let token = state.launch_token.as_deref().filter(|t| !t.is_empty())?;
    let url = format!("http://{}/?token={token}", state.upstream_host());
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(EXCHANGE_TIMEOUT)
        .build()
        .ok()?
        .get(&url)
        .send()
        .await
        .ok()?;
    if !response.status().is_redirection() {
        return None;
    }
    response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            let pair = value.split(';').next()?.trim();
            pair.starts_with(DSH_COOKIE_PREFIX).then(|| pair.to_owned())
        })
}

/// 启动网关。绑定失败（地址不可用/被占）即返错，不留半开状态。
/// fingerprint = 启动配置指纹（commands::fingerprint），存入句柄供 sync 比对。
/// `launch_token` 是 DSH 就绪 URL 里的启动 token（空 = 旧版无鉴权）。
pub async fn start(
    bind_ip: &str,
    port: u16,
    upstream: String,
    launch_token: String,
    devices: Vec<RemoteDevice>,
    fingerprint: u64,
) -> Result<GatewayHandle, String> {
    let bind: std::net::SocketAddr = format!("{bind_ip}:{port}")
        .parse()
        .map_err(|cause| format!("绑定地址不合法（{bind_ip}:{port}）：{cause}"))?;
    let state = GatewayState {
        upstream,
        devices: Arc::new(devices),
        client: reqwest::Client::new(),
        launch_token: (!launch_token.is_empty()).then_some(launch_token),
        dsh_cookie: Arc::new(tokio::sync::Mutex::new(None)),
    };
    // 顺手把登录兑换做掉：首个扫码请求就不必等一次兑换往返。
    if state.launch_token.is_some() {
        match state.ensure_cookie().await {
            Some(_) => {
                crate::logging::log("info", "远程网关已向 DSH 兑换登录 cookie");
            }
            None => {
                crate::logging::log(
                    "warn",
                    "远程网关暂未取得 DSH 登录 cookie（DSH 未就绪？）；转发时会自动重试",
                );
            }
        }
    }
    // 专用路由先于兜底：两条 WS 下行各自成桥，其余路径统一转发。
    let app = Router::new()
        .route("/api/events.mux", any(ws_handler))
        .route("/api/events.host", any(ws_handler))
        .route("/{*rest}", any(handler))
        .route("/", any(handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|cause| format!("网关监听失败（{bind}）：{cause}"))?;
    let local_addr = listener.local_addr().map_err(|cause| cause.to_string())?;
    let (shutdown, mut signal) = watch::channel(false);
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, app);
        tokio::select! {
            result = server => {
                if let Err(cause) = result {
                    crate::logging::log("warn", &format!("远程网关退出：{cause}"));
                }
            }
            _ = signal.changed() => {
                // 停机请求：丢弃 server（连接随之关闭）。
            }
        }
    });
    Ok(GatewayHandle {
        shutdown,
        task,
        local_addr,
        fingerprint,
    })
}

/// 统一入口：配对外，其余全部 HTTP 转发（含 SSE 流响应）。
async fn handler(State(state): State<GatewayState>, request: Request<Body>) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or_default().to_owned();

    // 配对入口：/qx-gate?token=…（唯一免鉴权路径）。
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

    forward(&state, request).await
}

/// WS 下行桥入口：鉴权后升级，浏览器 ↔ DSH 帧级透传（保留原始路径）。
/// 上游连接携带兑换出的 DSH cookie；没有（旧版 DSH）就裸连。
async fn ws_handler(
    State(state): State<GatewayState>,
    uri: axum::extract::OriginalUri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response<Body> {
    if !authorized(&state, uri.query().unwrap_or(""), &headers) {
        return plain(StatusCode::UNAUTHORIZED, "未配对设备");
    }
    let upstream_url = format!("ws://{}{}", state.upstream_host(), uri.path());
    let mut request = match HttpRequest::builder().uri(&upstream_url).body(()) {
        Ok(request) => request,
        Err(cause) => return plain(StatusCode::BAD_GATEWAY, &format!("上游请求构造失败：{cause}")),
    };
    if let Some(cookie) = state.ensure_cookie().await {
        if let Ok(value) = cookie.parse() {
            request.headers_mut().insert("cookie", value);
        }
    }
    ws.on_upgrade(move |client| bridge_ws(client, request))
        .into_response()
}

/// 鉴权：cookie（常规与 WS 升级请求都带）优先，query 兜底。
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

/// 双向桥：浏览器 WS ↔ DSH WS（帧级透传，ping/pong/close 各自终结）。
async fn bridge_ws(client: WebSocket, upstream: HttpRequest<()>) {
    let Ok((upstream, _)) = tokio_tungstenite::connect_async(upstream).await else {
        return; // 上游不可达：直接关客户端（浏览器会自动重连）。
    };
    let (mut client_tx, mut client_rx) = client.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    let to_upstream = async {
        while let Some(Ok(frame)) = client_rx.next().await {
            if upstream_tx.send(to_tungstenite(frame)).await.is_err() {
                break;
            }
        }
    };
    let to_client = async {
        while let Some(Ok(frame)) = upstream_rx.next().await {
            if client_tx.send(to_axum(frame)).await.is_err() {
                break;
            }
        }
    };
    tokio::join!(to_upstream, to_client);
    let _ = client_tx.close().await;
    let _ = upstream_tx.close().await;
}

/// axum WS 帧 → tungstenite 帧（字节语义透传）。
fn to_tungstenite(frame: axum::extract::ws::Message) -> tokio_tungstenite::tungstenite::Message {
    use axum::extract::ws::Message as In;
    use tokio_tungstenite::tungstenite::Message as Out;
    match frame {
        In::Text(text) => Out::Text(text.as_str().to_owned().into()),
        In::Binary(bytes) => Out::Binary(bytes.to_vec().into()),
        In::Ping(bytes) => Out::Ping(bytes.to_vec().into()),
        In::Pong(bytes) => Out::Pong(bytes.to_vec().into()),
        In::Close(frame) => {
            Out::Close(frame.map(
                |close| tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code: close.code.into(),
                    reason: close.reason.as_str().to_owned().into(),
                },
            ))
        }
    }
}

/// tungstenite 帧 → axum WS 帧。
fn to_axum(frame: tokio_tungstenite::tungstenite::Message) -> axum::extract::ws::Message {
    use axum::extract::ws::Message as Out;
    use tokio_tungstenite::tungstenite::Message as In;
    match frame {
        In::Text(text) => Out::Text(text.as_str().to_owned().into()),
        In::Binary(bytes) => Out::Binary(bytes.to_vec().into()),
        In::Ping(bytes) => Out::Ping(bytes.to_vec().into()),
        In::Pong(bytes) => Out::Pong(bytes.to_vec().into()),
        In::Close(frame) => Out::Close(frame.map(|close| axum::extract::ws::CloseFrame {
            code: close.code.into(),
            reason: close.reason.as_str().to_owned().into(),
        })),
        In::Frame(_) => Out::Binary(Vec::new().into()),
    }
}

/// HTTP 转发：方法/路径透传，响应流式回写（SSE 也走这条路）。
/// 手机侧头按 [`SKIP_UPSTREAM_HEADERS`] 剥除，换上网关持有的 DSH
/// cookie；上游 401 时强制重兑一次再试（cookie 失效自愈）。
async fn forward(state: &GatewayState, request: Request<Body>) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let url = format!(
        "http://{}{}{}",
        state.upstream_host(),
        parts.uri.path(),
        parts
            .uri
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default()
    );
    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    // 请求体先缓冲成字节：POST 载荷都很小，换来 401 重试可以原样重放
    // （流式体只能消费一次）。超限即拒绝，不给无界内存机会。
    let body_bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(cause) => {
            return plain(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("请求体无法缓冲：{cause}"),
            )
        }
    };

    let build_request = move |cookie: &Option<String>| {
        let mut outgoing = state
            .client
            .request(method.clone(), &url)
            .timeout(UPSTREAM_TIMEOUT);
        for (name, value) in parts.headers.iter() {
            if HOP_HEADERS.contains(&name.as_str())
                || SKIP_UPSTREAM_HEADERS.contains(&name.as_str())
                || name.as_str().starts_with("sec-fetch-")
            {
                continue;
            }
            if let Ok(header_value) = value.to_str() {
                outgoing = outgoing.header(name.as_str(), header_value);
            }
        }
        if let Some(cookie) = cookie {
            outgoing = outgoing.header(reqwest::header::COOKIE, cookie);
        }
        if !body_bytes.is_empty() {
            outgoing = outgoing.body(body_bytes.clone());
        }
        outgoing
    };

    let cookie = state.ensure_cookie().await;
    match build_request(&cookie).send().await {
        Ok(upstream) if upstream.status() == reqwest::StatusCode::UNAUTHORIZED && cookie.is_some() => {
            // cookie 失效（如 DSH_HOME 被替换）：重兑一次，失败就把 401 原样回给手机。
            let Some(fresh) = state.refresh_cookie().await else {
                return relay(upstream).await;
            };
            match build_request(&Some(fresh)).send().await {
                Ok(retried) => relay(retried).await,
                Err(cause) => unreachable_upstream(&cause),
            }
        }
        Ok(upstream) => relay(upstream).await,
        Err(cause) => unreachable_upstream(&cause),
    }
}

/// 把上游响应转成网关响应：流式回写，逐跳头与 DSH 的 set-cookie 不外泄。
async fn relay(upstream: reqwest::Response) -> Response<Body> {
    let mut response = Response::builder().status(upstream.status().as_u16());
    for (name, value) in upstream.headers().iter() {
        if HOP_HEADERS.contains(&name.as_str()) {
            continue;
        }
        response = response.header(name.clone(), value.clone());
    }
    let stream = upstream.bytes_stream().map_err(std::io::Error::other);
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
}

fn unreachable_upstream(cause: &reqwest::Error) -> Response<Body> {
    plain(
        StatusCode::BAD_GATEWAY,
        &format!("DSH 上游不可达（{cause}）：请确认千寻内 DSH 正在运行"),
    )
}

// ---- 小工具 ----

fn query_param(query: &str, key: &str) -> String {
    for pair in query.split('&') {
        let mut split = pair.splitn(2, '=');
        if split.next() == Some(key) {
            return split.next().map(url_decode).unwrap_or_default();
        }
    }
    String::new()
}

/// 极简 percent-decode（token 只含 hex，够用）。
fn url_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn plain(status: StatusCode, text: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Body::from(text.to_owned()))
        .unwrap_or_else(|_| Response::new(Body::from(text.to_owned())))
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

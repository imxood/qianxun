//! 网关本体：axum 服务 + DSH 回环转发 + WS 双向桥。

use std::sync::Arc;

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

use crate::remote::{MobileUi, RemoteDevice};

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

/// 运行中的网关句柄：停止信号 + 任务 + 实际监听地址（绑定成功的证明）。
pub struct GatewayHandle {
    pub shutdown: watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
    pub local_addr: std::net::SocketAddr,
}

/// 网关共享状态：DSH origin（host:port）+ 设备表快照（每次启动重建）。
#[derive(Clone)]
pub struct GatewayState {
    pub upstream: String,
    pub devices: Arc<Vec<RemoteDevice>>,
    pub client: reqwest::Client,
    /// 移动定制层（/qx-mobile/* 的文件服务）。
    pub mobile: Arc<MobileUi>,
}

impl GatewayState {
    fn upstream_host(&self) -> &str {
        self.upstream
            .trim_start_matches("http://")
            .trim_end_matches('/')
    }

    /// DSH 上游是否就绪（未就绪时 sync() 用 `127.0.0.1:0` 占位）。
    pub(crate) fn upstream_ready(&self) -> bool {
        self.upstream != "127.0.0.1:0"
    }

    /// cookie/query 命中任一未吊销 token？
    pub(crate) fn token_valid(&self, presented: &str) -> bool {
        !presented.is_empty()
            && self
                .devices
                .iter()
                .any(|device| !device.revoked && device.token == presented)
    }
}

/// 启动网关。绑定失败（地址不可用/被占）即返错，不留半开状态。
pub async fn start(
    bind_ip: &str,
    port: u16,
    upstream: String,
    devices: Vec<RemoteDevice>,
    mobile_access_dir: std::path::PathBuf,
) -> Result<GatewayHandle, String> {
    let bind: std::net::SocketAddr = format!("{bind_ip}:{port}")
        .parse()
        .map_err(|cause| format!("绑定地址不合法（{bind_ip}:{port}）：{cause}"))?;
    // 定制目录尽力创建：方便用户/agent 直接往里放 custom.css/js。
    let _ = std::fs::create_dir_all(&mobile_access_dir);
    let state = GatewayState {
        upstream,
        devices: Arc::new(devices),
        client: reqwest::Client::new(),
        mobile: Arc::new(MobileUi::new(mobile_access_dir)),
    };
    // 专用路由先于兜底：两条 WS 下行、/qx-mobile/* 各自成路，其余路径统一转发。
    let app = Router::new()
        .route("/api/events.mux", any(ws_handler))
        .route("/api/events.host", any(ws_handler))
        .route("/qx-mobile/info", any(super::mobile_ui::info))
        .route("/qx-mobile/version", any(super::mobile_ui::version))
        .route("/qx-mobile/bootstrap.js", any(super::mobile_ui::bootstrap))
        .route("/qx-mobile/custom.css", any(super::mobile_ui::custom_css))
        .route("/qx-mobile/custom.js", any(super::mobile_ui::custom_js))
        .route("/qx-mobile/{*rest}", any(super::mobile_ui::unknown))
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
    ws.on_upgrade(move |client| bridge_ws(client, upstream_url))
        .into_response()
}

/// 鉴权：cookie（常规与 WS 升级请求都带）优先，query 兜底。
pub(crate) fn authorized(state: &GatewayState, query: &str, headers: &HeaderMap) -> bool {
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
async fn bridge_ws(client: WebSocket, upstream_url: String) {
    let Ok((upstream, _)) = tokio_tungstenite::connect_async(upstream_url).await else {
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

/// HTML 注入缓冲上限：DSH 的 index.html 远小于此；超过则放弃注入原样透传。
const HTML_INJECT_LIMIT: usize = 2 * 1024 * 1024;

/// 注入移动定制层的引导标签（外链同源资源，CSP `self` 放行；defer 不阻塞首屏）。
const MOBILE_LAYER_SNIPPET: &str = "<link rel=\"stylesheet\" href=\"/qx-mobile/custom.css\" /><script src=\"/qx-mobile/bootstrap.js\" defer></script>";

/// 满足「GET + 200 + text/html + 含 </head>」时，把定制层标签插到 </head> 前。
/// 纯函数便于测试；任何不满足都返回 None（原样透传，绝不阻断/破坏页面）。
pub(crate) fn inject_mobile_layer(
    method: &reqwest::Method,
    status: u16,
    content_type: &str,
    html: &str,
) -> Option<String> {
    if *method != reqwest::Method::GET || status != 200 {
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

/// HTTP 转发：方法/路径/头/体透传，响应流式回写（SSE 也走这条路）；
/// 对 DSH 回来的 HTML 页面注入移动定制层（方案 A 的注入点）。
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
    let mut outgoing = state.client.request(method.clone(), &url);
    for (name, value) in parts.headers.iter() {
        if HOP_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if let Ok(header_value) = value.to_str() {
            outgoing = outgoing.header(name.as_str(), header_value);
        }
    }
    let stream = Body::into_data_stream(body).map_err(std::io::Error::other);
    outgoing = outgoing.body(reqwest::Body::wrap_stream(stream));

    match outgoing.send().await {
        Ok(upstream) => {
            let injectable = method == reqwest::Method::GET
                && upstream.status() == reqwest::StatusCode::OK
                && upstream
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"));
            if !injectable {
                return stream_upstream(upstream).await;
            }
            // HTML 候选：缓冲（≤2MiB）后改写。超限/截断则原样透传，绝不半改。
            // 上游缓存策略保留（etag/last-modified 丢弃：内容已改写，禁止旧实体复用）。
            let cache_control = upstream
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            let mut upstream_stream = upstream.bytes_stream();
            let mut overflow = false;
            while let Some(chunk) = upstream_stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        bytes.extend_from_slice(&chunk);
                        if bytes.len() > HTML_INJECT_LIMIT {
                            overflow = true;
                            break;
                        }
                    }
                    Err(cause) => {
                        return plain(StatusCode::BAD_GATEWAY, &format!("DSH 响应中断（{cause}）"))
                    }
                }
            }
            if overflow {
                // 超大 HTML：已缓冲部分 + 剩余流拼接透传（放弃注入，体与原文逐字节一致）。
                let prefix =
                    futures_util::stream::once(
                        async move { Ok::<_, std::io::Error>(bytes.into()) },
                    );
                let rest = upstream_stream.map_err(std::io::Error::other);
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/html; charset=utf-8")
                    .body(Body::from_stream(prefix.chain(rest)))
                    .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"));
            }
            let Ok(text) = String::from_utf8(bytes) else {
                return plain(StatusCode::BAD_GATEWAY, "DSH 页面非 UTF-8，跳过注入");
            };
            let body = inject_mobile_layer(&method, 200, "text/html", &text).unwrap_or(text);
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/html; charset=utf-8");
            if let Some(cache) = cache_control {
                builder = builder.header("cache-control", cache);
            }
            builder
                .body(Body::from(body))
                .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
        }
        Err(cause) => plain(
            StatusCode::BAD_GATEWAY,
            &format!("DSH 上游不可达（{cause}）：请确认千寻内 DSH 正在运行"),
        ),
    }
}

/// 常规透传：状态/头照抄，体流式回写（SSE 逐块、WS 已在专用路由处理）。
async fn stream_upstream(upstream: reqwest::Response) -> Response<Body> {
    let mut response = Response::builder().status(upstream.status().as_u16());
    for (name, value) in upstream.headers().iter() {
        if HOP_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if let Ok(value) = value.to_str() {
            response = response.header(name.as_str(), value);
        }
    }
    let stream = upstream.bytes_stream().map_err(std::io::Error::other);
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
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

pub(crate) fn plain(status: StatusCode, text: &str) -> Response<Body> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const GET: reqwest::Method = reqwest::Method::GET;

    #[test]
    fn 注入_在head结束标签前插入定制层() {
        let html = "<html><head><title>t</title></head><body></body></html>";
        let injected =
            inject_mobile_layer(&GET, 200, "text/html; charset=utf-8", html).expect("应注入");
        assert!(injected.contains(MOBILE_LAYER_SNIPPET));
        let position = injected.find(MOBILE_LAYER_SNIPPET).unwrap();
        assert!(injected[..position].contains("</title>"));
        let after = &injected[position + MOBILE_LAYER_SNIPPET.len()..];
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
        assert!(inject_mobile_layer(&reqwest::Method::POST, 200, "text/html", html).is_none());
        // 非 200
        assert!(inject_mobile_layer(&GET, 404, "text/html", html).is_none());
        // 非 HTML
        assert!(inject_mobile_layer(&GET, 200, "application/json", "{}").is_none());
        // 没有 </head>（片段/流式页面）
        assert!(inject_mobile_layer(&GET, 200, "text/html", "<p>fragment</p>").is_none());
    }
}

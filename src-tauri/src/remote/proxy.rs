//! 本地服务反代（docs/20）：手机浏览器直达电脑上的 localhost 服务。
//!
//! 解决的问题：手机配对后经网关只能访问 DSH 本体；开发者在本机起的
//! dev server（`127.0.0.1:5173` 这类）手机够不着——localhost 是手机自己。
//!
//! 形态：**专用 socket + 整源代理**。网关在 LAN 网卡上额外绑一个端口
//! （网关端口 + 2），这个端口的整个 origin 都代理给「cookie 里指定的
//! 目标」：
//! - 首次访问 `http://<lan-ip>:<proxy-port>/?_qx_to=127.0.0.1:5173`
//!   （可再带 `&token=` 完成配对）→ 校验目标 → 种 `qx_proxy_to` cookie
//!   → 302 回 `/`；
//! - 之后该 origin 上的全部请求（根相对子资源、fetch、SSE、WebSocket
//!   升级）都原样转发给目标——**不做任何 HTML/路径改写**，这是选「专用
//!   端口」而不是「网关路径前缀」的根本原因：前缀方案会被根相对 URL
//!   打穿（`/src/main.ts` 会绕过前缀直落 DSH），且 HMR 的 WS 没法跟上。
//!
//! 安全边界（默认收敛，运营者可放宽）：
//! - 鉴权与网关完全一致（配对 cookie / query token / 回环放行）；
//! - 目标只允许**回环字面量**（`127.0.0.0/8`、`localhost`、`[::1]`）；
//!   「允许内网目标」开关打开后才放行 RFC1918 / 链路本地 / IPv6 ULA——
//!   域名目标一律拒绝（不做 DNS 解析，杜绝 rebinding 类绕行）；
//! - 回包剥 `x-frame-options` 与 `content-security-policy`（本 socket 限
//!   定），让目标服务也能被 DSH 页 iframe 引用；其余字节级透传。

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ws, FromRequestParts, OriginalUri, Request, State};
use axum::http::{header, HeaderMap, HeaderName, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use futures_util::{SinkExt, StreamExt};

use crate::dsh_upstream::{plain, query_param};
use crate::remote::gateway::GatewayState;

/// WS 桥空闲超时：与 dsh_upstream 的 DSH 桥同值（docs/18 A3），手机
/// 息屏/弱网下的半开连接不能无限挂泵。
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
/// 请求体上限：本地 dev server 场景足够大（大文件上传另说）。
const PROXY_BODY_LIMIT: usize = 64 * 1024 * 1024;
/// 目标 cookie 名。值即 `host:port` 目标，见 [`parse_target`]。
const TARGET_COOKIE: &str = "qx_proxy_to";
/// 目标 cookie 有效期：12 小时——一天的开发会话，隔夜换目标重种。
const TARGET_COOKIE_MAX_AGE: usize = 12 * 60 * 60;

/// 反代面启动配置（settings.remote.proxyEnabled / proxyAllowLan）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProxyLaunch {
    pub enabled: bool,
    /// 允许把内网（RFC1918 / 链路本地 / IPv6 ULA）作为反代目标。
    pub allow_lan_targets: bool,
}

impl ProxyLaunch {
    /// 网关默认端口的偏移：release 23090+2=23092 / debug 23091+2=23093，
    /// 与网关本体错开且不与安装版互抢。端口过小（<1024 的特权段）或
    /// 顶格（>65533）时返回 None——测试传 port=0 时也不会去绑 2 号端口。
    pub fn port_for(gateway_port: u16) -> Option<u16> {
        gateway_port
            .checked_add(2)
            .filter(|derived| *derived >= 1024)
    }

    /// 反代面监听地址：与网关 LAN 绑定**同 IP**，端口走 [`port_for`]。
    /// 接收已解析的网关 SocketAddr（自带端口），推导全程结构化——不做
    /// 字符串拼接（曾把 `ip:gw_port` 与 `:proxy_port` 粘成 `ip:gw:proxy`
    /// 三段非法地址，`?` 一路炸掉整个网关，连回环入口都没了）。
    pub fn addr_for(gateway_bind: SocketAddr) -> Option<SocketAddr> {
        Some(SocketAddr::new(
            gateway_bind.ip(),
            Self::port_for(gateway_bind.port())?,
        ))
    }
}

/// 反代面共享状态：复用网关的鉴权原语与设备表快照。
#[derive(Clone)]
pub struct ProxyState {
    gateway: GatewayState,
    proxy_port: u16,
    allow_lan_targets: bool,
    client: reqwest::Client,
}

/// 构造反代 Router。`proxy_port` 参与回环放行判定（本机调试直连）。
pub fn router(state: GatewayState, proxy_port: u16, launch: ProxyLaunch) -> Router {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        // 跟随重定向会吃掉 Location 改写（手机会被指去不可达的
        // http://127.0.0.1:...），必须原样把 3xx 交还浏览器。
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let proxy = ProxyState {
        gateway: state,
        proxy_port,
        allow_lan_targets: launch.allow_lan_targets,
        client,
    };
    Router::new()
        .route("/", any(entry))
        .fallback(any(dispatch))
        .with_state(proxy)
}

impl ProxyState {
    /// 反代面鉴权：回环 Host + 本端口（本机调试直连）放行；其余与网关
    /// LAN 入口一致（配对 cookie / query token）。
    fn authorized(&self, query: &str, headers: &HeaderMap) -> bool {
        loopback_host_on_port(headers, self.proxy_port)
            || self.gateway.mobile_authorized(query, headers)
    }
}

/// Host 是回环名且端口恰为 `port`（本机直连反代 socket 的免 token 通道，
/// 语义对齐 gateway::is_loopback——只有真正命中本 listener 的请求才匹配）。
fn loopback_host_on_port(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some((name, host_port)) = host.rsplit_once(':') else {
        return false;
    };
    host_port == port.to_string() && matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

/// 解析并校验反代目标。只接受 `host:port` 形式：
/// - 回环字面量永远允许：`127.0.0.0/8`、`localhost`、`[::1]`；
/// - `allow_lan` 时放行私网/链路本地/ULA 的 **IP 字面量**；
/// - 其它域名一律拒绝——不做 DNS 解析，杜绝「解析到公网」的绕行。
pub fn parse_target(raw: &str, allow_lan: bool) -> Option<String> {
    let raw = raw.trim();
    let (host, port_text) = raw.rsplit_once(':')?;
    if port_text.is_empty() || !port_text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port_text.parse().ok()?;
    if port == 0 {
        return None;
    }
    let loopback = matches!(host, "localhost" | "[::1]" | "::1")
        || host
            .parse::<std::net::Ipv4Addr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    if loopback {
        return Some(format!("{host}:{port}"));
    }
    if !allow_lan {
        return None;
    }
    // 私网 IPv4（RFC1918 + 链路本地）。
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        let octets = ip.octets();
        let private = octets[0] == 10
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            || (octets[0] == 192 && octets[1] == 168)
            || (octets[0] == 169 && octets[1] == 254);
        return private.then(|| format!("{host}:{port}"));
    }
    // IPv6 链路本地 / ULA（EasyTier 也可能发这族地址）。
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let lower = bare.to_ascii_lowercase();
    if lower.starts_with("fe80:") || lower.starts_with("fc") || lower.starts_with("fd") {
        return Some(format!("[{bare}]:{port}"));
    }
    None
}

// ---- 路由 handler ----

/// 入口：`/?_qx_to=<host:port>`（可再带 `&token=` 完成配对）。
/// 校验目标 → 种 cookie → 302 回 `/`；不带参数则给引导页。
async fn entry(
    State(state): State<ProxyState>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response<Body> {
    let query = uri.query().unwrap_or_default();
    if !state.authorized(query, &headers) {
        return plain(
            StatusCode::UNAUTHORIZED,
            "未配对设备：请先用千寻远程页生成的配对链接完成配对",
        );
    }
    let requested = query_param(query, "_qx_to");
    if requested.is_empty() {
        return landing(&state);
    }
    match parse_target(&requested, state.allow_lan_targets) {
        Some(target) => {
            crate::logging::log("info", &format!("[qx-proxy] 反代目标切换为 {target}"));
            Response::builder()
                .status(StatusCode::FOUND)
                .header(header::LOCATION, "/")
                .header(
                    header::SET_COOKIE,
                    format!(
                        "{TARGET_COOKIE}={target}; Path=/; Max-Age={TARGET_COOKIE_MAX_AGE}; HttpOnly; SameSite=Lax"
                    ),
                )
                .body(Body::empty())
                .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
        }
        None => plain(
            StatusCode::BAD_REQUEST,
            &format!(
                "目标不合法：「{requested}」。仅支持 127.0.0.1 / localhost / [::1] 加端口；\
                 内网目标需先在千寻远程页开启「允许内网目标」"
            ),
        ),
    }
}

/// 其余一切路径：无目标 cookie 给引导页；WS 升级走桥；其余 HTTP 原样转发。
async fn dispatch(
    State(state): State<ProxyState>,
    uri: OriginalUri,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response<Body> {
    let query = uri.query().unwrap_or_default().to_owned();
    if !state.authorized(&query, &headers) {
        return plain(
            StatusCode::UNAUTHORIZED,
            "未配对设备：请先用千寻远程页生成的配对链接完成配对",
        );
    }
    let target = cookie_value(&headers, TARGET_COOKIE)
        .and_then(|value| parse_target(&value, state.allow_lan_targets));
    let Some(target) = target else {
        return landing(&state);
    };
    if is_upgrade(&headers) {
        return relay_ws(state, target, uri, request).await;
    }
    forward_http(state, &target, uri, headers, request).await
}

/// 目标未指定时的引导页（已鉴权才会到这里）：说明用法 + 一个换目标的表单。
fn landing(state: &ProxyState) -> Response<Body> {
    let allow_hint = if state.allow_lan_targets {
        "已允许内网目标"
    } else {
        "当前仅允许本机（回环）目标；要访问内网服务，在千寻远程页开启「允许内网目标」"
    };
    let html = format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>千寻本地服务反代</title></head>\
<body style=\"font-family:system-ui;max-width:32rem;margin:3rem auto;padding:0 1rem;line-height:1.6\">\
<h2>千寻本地服务反代</h2>\
<p>在地址栏访问 <code>/?_qx_to=&lt;目标&gt;</code> 即可把本源切换到目标服务，\
例如 <code>/?_qx_to=127.0.0.1:5173</code>。切换后整站（含子资源与 WebSocket）都代理给目标。</p>\
<p style=\"color:#666\">{allow_hint}</p>\
<form method=\"get\" action=\"/\" style=\"display:flex;gap:.5rem;margin-top:1rem\">\
<input name=\"_qx_to\" required placeholder=\"127.0.0.1:5173\" \
style=\"flex:1;padding:.5rem;border:1px solid #ccc;border-radius:.5rem\">\
<button style=\"padding:.5rem 1rem\">打开</button></form>\
</body></html>"
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
}

/// HTTP 转发：方法/头/体原样（剥逐跳头与千寻自身 cookie），响应流式透传
/// （剥框架防码头；Location 改写回本源）。
async fn forward_http(
    state: ProxyState,
    target: &str,
    uri: OriginalUri,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response<Body> {
    let _ = headers;
    let path = uri.path().to_owned();
    let query = uri.query().unwrap_or_default().to_owned();
    let url = if query.is_empty() {
        format!("http://{target}{path}")
    } else {
        format!("http://{target}{path}?{query}")
    };
    let method = request.method().clone();
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, PROXY_BODY_LIMIT).await {
        Ok(bytes) => bytes,
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE, "请求体超过 64MB 上限"),
    };
    let mut request = state.client.request(method, &url);
    for (name, value) in parts.headers.iter() {
        if skip_request_header(name) {
            continue;
        }
        if name == header::COOKIE {
            if let Ok(text) = value.to_str() {
                if let Some(filtered) = filter_qx_cookies(text) {
                    request = request.header(name, filtered);
                }
            }
            continue;
        }
        if matches!(name.as_str(), "origin" | "referer") {
            if let Ok(text) = value.to_str() {
                if let Some(rewritten) = rewrite_authority(text, target) {
                    request = request.header(name, rewritten);
                }
            }
            continue;
        }
        request = request.header(name, value);
    }
    let response = match request.body(bytes).send().await {
        Ok(response) => response,
        Err(cause) => {
            crate::logging::log("warn", &format!("[qx-proxy] {target} 不可达：{cause}"));
            return plain(
                StatusCode::BAD_GATEWAY,
                &format!("目标服务不可达（{target}）：{cause}"),
            );
        }
    };
    let mut builder = Response::builder().status(response.status());
    for (name, value) in response.headers() {
        if skip_response_header(name) {
            continue;
        }
        if name == header::LOCATION {
            if let Ok(text) = value.to_str() {
                builder = builder.header(name, rewrite_location(text, target));
            }
            continue;
        }
        builder = builder.header(name, value);
    }
    // 204/304 无响应体；给流式构造器喂空体会与头冲突，单独处理。
    if matches!(
        response.status(),
        StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED
    ) {
        return builder
            .body(Body::empty())
            .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"));
    }
    let stream = response.bytes_stream();
    builder
        .body(Body::from_stream(stream.map(|chunk| {
            chunk.map_err(|cause| {
                crate::logging::log("warn", &format!("[qx-proxy] 响应流中断：{cause}"));
                std::io::Error::other(cause)
            })
        })))
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
}

/// WS 中继：手机 WS ↔ 目标 WS（帧级透传，ping/pong/close 各自终结；
/// 空闲超时与 dsh_upstream 的 DSH 桥同策略）。子协议不透传（本地 dev
/// server 的 HMR 不用；需要时是明确的后续项）。
async fn relay_ws(
    state: ProxyState,
    target: String,
    uri: OriginalUri,
    request: Request<Body>,
) -> Response<Body> {
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_owned())
        .unwrap_or_else(|| "/".to_owned());
    let (mut parts, _body) = request.into_parts();
    let ws = match ws::WebSocketUpgrade::from_request_parts(&mut parts, &state).await {
        Ok(ws) => ws,
        Err(rejection) => return rejection.into_response(),
    };
    let url = format!("ws://{target}{path_and_query}");
    let label = path_and_query.clone();
    ws.on_upgrade(move |client| bridge_ws(client, url, label))
        .into_response()
}

/// 双向桥（镜像 dsh_upstream::bridge_ws 的泵与日志纪律）。
async fn bridge_ws(client: ws::WebSocket, url: String, label: String) {
    let (upstream, _) = match tokio_tungstenite::connect_async(url.as_str()).await {
        Ok(pair) => pair,
        Err(cause) => {
            crate::logging::log(
                "warn",
                &format!("[qx-proxy-ws] {label} 上游连接失败：{cause}（手机侧表现为 1006）"),
            );
            return;
        }
    };
    crate::logging::log("info", &format!("[qx-proxy-ws] {label} 上游已连通"));
    let (mut client_tx, mut client_rx) = client.split();
    let (mut up_tx, mut up_rx) = upstream.split();
    let to_upstream = async {
        loop {
            match tokio::time::timeout(WS_IDLE_TIMEOUT, client_rx.next()).await {
                Err(_) => break "手机侧空闲超时（疑似半开连接）".to_owned(),
                Ok(None) => break "手机侧正常关闭".to_owned(),
                Ok(Some(Err(cause))) => break format!("手机侧读错误：{cause}"),
                Ok(Some(Ok(frame))) => {
                    if up_tx.send(to_tungstenite(frame)).await.is_err() {
                        break "上行发送失败（上游已断）".to_owned();
                    }
                }
            }
        }
    };
    let to_client = async {
        loop {
            match tokio::time::timeout(WS_IDLE_TIMEOUT, up_rx.next()).await {
                Err(_) => break "上游侧空闲超时（疑似半开连接）".to_owned(),
                Ok(None) => break "上游侧正常关闭".to_owned(),
                Ok(Some(Err(cause))) => break format!("上游侧读错误：{cause}"),
                Ok(Some(Ok(frame))) => {
                    if client_tx.send(to_axum(frame)).await.is_err() {
                        break "下行发送失败（手机已断）".to_owned();
                    }
                }
            }
        }
    };
    let (up_result, down_result) = tokio::join!(to_upstream, to_client);
    crate::logging::log(
        "info",
        &format!("[qx-proxy-ws] {label} 连接结束：上行[{up_result}]，下行[{down_result}]"),
    );
    let _ = client_tx.close().await;
}

// ---- 小工具 ----

/// axum WS 帧 -> tungstenite 帧（与 dsh_upstream 同形，字节语义透传）。
fn to_tungstenite(frame: ws::Message) -> tokio_tungstenite::tungstenite::Message {
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

/// tungstenite 帧 -> axum WS 帧。
fn to_axum(frame: tokio_tungstenite::tungstenite::Message) -> ws::Message {
    use axum::extract::ws::Message as Out;
    use tokio_tungstenite::tungstenite::Message as In;
    match frame {
        In::Text(text) => Out::Text(text.as_str().to_owned().into()),
        In::Binary(bytes) => Out::Binary(bytes.to_vec().into()),
        In::Ping(bytes) => Out::Ping(bytes.to_vec().into()),
        In::Pong(bytes) => Out::Pong(bytes.to_vec().into()),
        In::Close(frame) => Out::Close(frame.map(|close| ws::CloseFrame {
            code: close.code.into(),
            reason: close.reason.as_str().to_owned().into(),
        })),
        In::Frame(_) => Out::Binary(Vec::new().into()),
    }
}

/// 请求是否要求协议升级（WebSocket 等）。
fn is_upgrade(headers: &HeaderMap) -> bool {
    let connection = headers
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    headers.contains_key(header::UPGRADE)
        && connection
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
}

/// 转发请求时剥掉的头：逐跳头 + 需要重建的头（host/cookie 由 reqwest 与
/// 过滤器负责；content-length/te 由 hyper 按新体重新定帧）。
fn skip_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "cookie"
            | "expect"
            | "content-length"
    )
}

/// 回包剥掉的头：逐跳头 + 框架防码头（x-frame-options / CSP，仅本
/// socket——目标是开发者自己的本地服务，可被 DSH 页 iframe 引用是需求）。
fn skip_response_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "x-frame-options"
            | "content-security-policy"
            | "content-security-policy-report-only"
    )
}

/// 取 cookie 值（同名取首个）。
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| {
            text.split(';').find_map(|part| {
                part.trim()
                    .strip_prefix(&format!("{name}="))
                    .map(str::to_owned)
            })
        })
}

/// 从转发的 Cookie 头里剥掉千寻自身的凭据（qx_token / qx_proxy_to）——
/// 本地 dev server 无权看到网关配对凭据。全部被剥空时返回 None（不发）。
fn filter_qx_cookies(text: &str) -> Option<String> {
    let kept: Vec<&str> = text
        .split(';')
        .map(str::trim)
        .filter(|part| {
            !part.is_empty() && !part.starts_with("qx_token=") && !part.starts_with("qx_proxy_to=")
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// Origin/Referer 改写：把 authority 换成目标（dev server 的同源守卫），
/// 保留路径/查询。解析失败返回 None（调用侧丢弃该头）。
fn rewrite_authority(value: &str, target: &str) -> Option<String> {
    let (scheme, rest) = value.split_once("://")?;
    let (authority, tail) = match rest.split_once('/') {
        Some((authority, tail)) => (authority, Some(tail)),
        None => (rest, None),
    };
    if authority.is_empty() {
        return None;
    }
    Some(match tail {
        Some(tail) => format!("{scheme}://{target}/{tail}"),
        None => format!("{scheme}://{target}"),
    })
}

/// Location 改写：目标 origin 绝对地址改回根相对（手机直连目标不可达）；
/// 其余（相对路径/外部地址）原样。
fn rewrite_location(location: &str, target: &str) -> String {
    let absolute = format!("http://{target}");
    if location == absolute {
        return "/".to_owned();
    }
    if let Some(rest) = location.strip_prefix(&format!("{absolute}/")) {
        return format!("/{rest}");
    }
    location.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{MobileUi, RemoteDevice};

    fn device(token: &str) -> RemoteDevice {
        RemoteDevice {
            id: "d".to_owned(),
            name: "n".to_owned(),
            token: token.to_owned(),
            created_at: 0,
            revoked: false,
        }
    }

    /// 测试用 ProxyState：上游占位 0 端口（本模块测试不打上游）。
    fn test_state(token: &str, allow_lan: bool) -> ProxyState {
        let gateway = GatewayState {
            upstream: crate::dsh_upstream::Upstream::new("127.0.0.1:0".to_owned(), None),
            devices: std::sync::Arc::new(vec![device(token)]),
            loopback_addr: "127.0.0.1:1".parse().unwrap(),
            mobile: std::sync::Arc::new(MobileUi::new(std::env::temp_dir().join("qx-proxy-test"))),
            pair_codes: Default::default(),
        };
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        ProxyState {
            gateway,
            proxy_port: 23092,
            allow_lan_targets: allow_lan,
            client,
        }
    }

    fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::from_lowercase(name.as_bytes()).unwrap(),
                header::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn 目标解析_回环永远放行() {
        assert_eq!(
            parse_target("127.0.0.1:5173", false),
            Some("127.0.0.1:5173".to_owned())
        );
        assert_eq!(
            parse_target("localhost:3000", false),
            Some("localhost:3000".to_owned())
        );
        assert_eq!(
            parse_target("[::1]:8080", false),
            Some("[::1]:8080".to_owned())
        );
        // 127.0.0.0/8 整段都是回环。
        assert_eq!(
            parse_target("127.9.9.9:1", false),
            Some("127.9.9.9:1".to_owned())
        );
        // 前后空白容忍。
        assert_eq!(
            parse_target(" 127.0.0.1:5173 ", false),
            Some("127.0.0.1:5173".to_owned())
        );
    }

    #[test]
    fn 目标解析_默认拒公网与域名_开闸后放私网() {
        // 公网 IP / 域名 / 无端口 / 端口 0 / 残缺：一律拒绝。
        assert_eq!(parse_target("8.8.8.8:443", false), None);
        assert_eq!(parse_target("8.8.8.8:443", true), None);
        assert_eq!(parse_target("evil.com:5173", false), None);
        assert_eq!(parse_target("evil.com:5173", true), None);
        assert_eq!(parse_target("127.0.0.1", false), None);
        assert_eq!(parse_target("127.0.0.1:", false), None);
        assert_eq!(parse_target("127.0.0.1:abc", false), None);
        assert_eq!(parse_target("127.0.0.1:0", false), None);
        // 开闸后：RFC1918 / 链路本地 / IPv6 ULA 放行；公网与域名仍拒。
        assert_eq!(
            parse_target("192.168.20.2:8080", true),
            Some("192.168.20.2:8080".to_owned())
        );
        assert_eq!(
            parse_target("10.1.2.3:80", true),
            Some("10.1.2.3:80".to_owned())
        );
        assert_eq!(
            parse_target("172.16.0.9:80", true),
            Some("172.16.0.9:80".to_owned())
        );
        assert_eq!(
            parse_target("169.254.1.1:80", true),
            Some("169.254.1.1:80".to_owned())
        );
        assert_eq!(
            parse_target("[fd00::2]:9000", true),
            Some("[fd00::2]:9000".to_owned())
        );
        assert_eq!(parse_target("[2001:db8::1]:80", true), None);
    }

    #[test]
    fn 端口推导与回环host判定() {
        assert_eq!(ProxyLaunch::port_for(23090), Some(23092));
        assert_eq!(ProxyLaunch::port_for(65535), None);
        // 特权段不绑（测试 port=0 不得去绑 2 号端口）。
        assert_eq!(ProxyLaunch::port_for(0), None);
        assert_eq!(ProxyLaunch::port_for(1021), None);
        assert_eq!(ProxyLaunch::port_for(1022), Some(1024));
        let headers = header_map(&[("host", "127.0.0.1:23092")]);
        assert!(loopback_host_on_port(&headers, 23092));
        assert!(!loopback_host_on_port(&headers, 23090), "端口必须精确匹配");
        let lan_headers = header_map(&[("host", "192.168.20.2:23092")]);
        assert!(
            !loopback_host_on_port(&lan_headers, 23092),
            "LAN Host 不算回环"
        );
    }

    /// 回归（2026-11-04 真机事故）：反代地址曾用 `format!("{lan}:{port}")`
    /// 拼接已含端口的网关 SocketAddr，产出 `192.168.20.2:23090:23092`，
    /// parse 失败后 `?` 炸掉整个网关（回环一起挂，DSH 页报
    /// 「回环代理尚未监听」）。推导必须结构化：同 IP、端口 = 网关 + 2。
    #[test]
    fn addr_for_同ip取网关端口加二_端口越界返回none() {
        let lan: std::net::SocketAddr = "192.168.20.2:23090".parse().unwrap();
        assert_eq!(
            ProxyLaunch::addr_for(lan),
            Some("192.168.20.2:23092".parse().unwrap())
        );
        // 端口顶格：port_for 返回 None -> addr_for 也是 None（不是绑 2 号端口）。
        let top: std::net::SocketAddr = "10.0.0.1:65535".parse().unwrap();
        assert_eq!(ProxyLaunch::addr_for(top), None);
        // IPv6 网关同样成立。
        let v6: std::net::SocketAddr = "[fd00::1]:23090".parse().unwrap();
        assert_eq!(
            ProxyLaunch::addr_for(v6),
            Some("[fd00::1]:23092".parse().unwrap())
        );
    }

    #[test]
    fn cookie过滤器_剥千寻凭据_保目标自己的() {
        assert_eq!(
            filter_qx_cookies("qx_token=AAA; qx_proxy_to=127.0.0.1:5173; sid=xyz"),
            Some("sid=xyz".to_owned())
        );
        assert_eq!(filter_qx_cookies("qx_token=AAA"), None, "全剥空则不发");
        assert_eq!(filter_qx_cookies("sid=xyz"), Some("sid=xyz".to_owned()));
    }

    #[test]
    fn origin改写_保路径换authority() {
        assert_eq!(
            rewrite_authority("http://192.168.20.2:23092/src/main.ts", "127.0.0.1:5173"),
            Some("http://127.0.0.1:5173/src/main.ts".to_owned())
        );
        assert_eq!(
            rewrite_authority("http://192.168.20.2:23092", "127.0.0.1:5173"),
            Some("http://127.0.0.1:5173".to_owned())
        );
        assert_eq!(rewrite_authority("garbage", "127.0.0.1:5173"), None);
    }

    #[test]
    fn location改写_目标origin回根相对() {
        assert_eq!(
            rewrite_location("http://127.0.0.1:5173/login", "127.0.0.1:5173"),
            "/login"
        );
        assert_eq!(
            rewrite_location("http://127.0.0.1:5173", "127.0.0.1:5173"),
            "/"
        );
        assert_eq!(rewrite_location("/relative", "127.0.0.1:5173"), "/relative");
        // 外部 OAuth 跳转原样保留。
        assert_eq!(
            rewrite_location("https://oauth.example.com/cb", "127.0.0.1:5173"),
            "https://oauth.example.com/cb"
        );
    }

    #[test]
    fn upgrade判定() {
        let headers = header_map(&[
            ("connection", "keep-alive, Upgrade"),
            ("upgrade", "websocket"),
        ]);
        assert!(is_upgrade(&headers));
        let plain_headers = header_map(&[("connection", "keep-alive")]);
        assert!(!is_upgrade(&plain_headers));
    }

    #[test]
    fn 帧转换_close映射() {
        let close = ws::Message::Close(Some(ws::CloseFrame {
            code: 1000,
            reason: "bye".to_owned().into(),
        }));
        let out = to_tungstenite(close);
        assert!(matches!(
            out,
            tokio_tungstenite::tungstenite::Message::Close(Some(_))
        ));
    }

    /// 端到端（真实 axum + reqwest）：入口种 cookie、鉴权拒绝、转发与
    /// 凭据剥离、引导页。上游替身绑 127.0.0.1；反代面绑 127.0.0.2
    /// （Host 非回环名 → 必须带 token，正好覆盖鉴权路径）。
    #[tokio::test]
    async fn 反代端到端_入口转发与剥离() {
        use axum::routing::get;

        // 上游替身：回显 host/path/cookie 头。
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let app = Router::new().route(
            "/{*rest}",
            get(|headers: HeaderMap, uri: OriginalUri| async move {
                format!(
                    "host={}|path={}|cookie={}",
                    headers
                        .get("host")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                    uri.path(),
                    headers
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or(""),
                )
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(upstream, app).await;
        });

        // 反代面：绑 127.0.0.2（非回环名 → token 必需）。
        let token = "t".repeat(64);
        let state = test_state(&token, false);
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.2:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();
        let router = router(
            state.gateway.clone(),
            proxy_port,
            ProxyLaunch {
                enabled: true,
                allow_lan_targets: false,
            },
        );
        tokio::spawn(async move {
            let _ = axum::serve(proxy_listener, router).await;
        });
        let base = format!("http://127.0.0.2:{proxy_port}");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        // 1) 未带 token → 401。
        let response = client.get(&base).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // 2) 入口（带 token + 目标）→ 302 + 种 cookie。
        let response = client
            .get(format!(
                "{base}/?_qx_to=127.0.0.1:{upstream_port}&token={token}"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let cookie = response
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(
            cookie.starts_with("qx_proxy_to=127.0.0.1:"),
            "应种目标 cookie：{cookie}"
        );

        // 3) 非法目标 → 400。
        let response = client
            .get(format!("{base}/?_qx_to=8.8.8.8:443&token={token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // 4) 带 cookie 转发：host/path 重写正确，千寻 cookie 被剥。
        let response = client
            .get(format!("{base}/assets/app.js"))
            .header(
                "cookie",
                format!("qx_proxy_to=127.0.0.1:{upstream_port}; qx_token={token}"),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(
            body.contains(&format!("host=127.0.0.1:{upstream_port}")),
            "上游应看到目标 authority：{body}"
        );
        assert!(body.contains("path=/assets/app.js"), "{body}");
        assert!(
            !body.contains("qx_token=") && !body.contains("qx_proxy_to="),
            "千寻凭据不得漏给目标：{body}"
        );

        // 5) 引导页：有 token 无目标 cookie → 200 HTML。
        let response = client
            .get(format!("{base}/?token={token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .starts_with("text/html"));
    }

    /// WS 中继端到端：上游 echo WS（tokio-tungstenite accept），手机侧
    /// connect_async 带目标 cookie，收发一帧往返。
    #[tokio::test]
    async fn ws中继端到端回显() {
        use base64::Engine as _;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        // 上游 echo WS。
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = upstream.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(message)) = ws.next().await {
                if (message.is_text() || message.is_binary()) && ws.send(message).await.is_err() {
                    break;
                }
            }
        });

        let token = "t".repeat(64);
        let state = test_state(&token, false);
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.2:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();
        let router = router(state.gateway, proxy_port, ProxyLaunch::default());
        tokio::spawn(async move {
            let _ = axum::serve(proxy_listener, router).await;
        });

        // 手机侧：手工构造带配对 + 目标两个 cookie 的 WS 升级请求
        // （tungstenite 只校验不代填 WS 头，与 dsh_upstream::upgrade_ws 的
        // 实测教训一致；鉴权要求 qx_token——真机手机配对后本就同时持有）。
        let key = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        let request = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(format!("ws://127.0.0.2:{proxy_port}/@vite/connect"))
            .header("host", format!("127.0.0.2:{proxy_port}"))
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", key)
            .header(
                "cookie",
                format!("qx_token={token}; qx_proxy_to=127.0.0.1:{upstream_port}"),
            )
            .body(())
            .unwrap();
        let (mut ws, response) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(response.status(), 101);
        ws.send(Message::text("ping-hmr")).await.unwrap();
        let echoed = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("回显超时")
            .expect("流结束")
            .expect("帧错误");
        assert_eq!(echoed, Message::text("ping-hmr"));
        let _ = ws.close(None).await;
    }
}

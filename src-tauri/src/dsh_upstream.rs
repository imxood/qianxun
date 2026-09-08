//! 对 DSH 上游的转发原语：cookie 兑换、HTTP 流式转发、WS 双向桥。
//!
//! DSH 0.1.2 起自带浏览器鉴权：就绪 URL 里的启动 token 只用于 `GET /`
//! 兑换一张 Host 绑定的签名 cookie（`HttpOnly; SameSite=Strict`），
//! `/api/*` 与 WS 升级只认 cookie。桌面回环代理（harness::proxy）与
//! 远程网关（remote::gateway）都需要「token 与 cookie 留在服务端、
//! 转发时附加」这一套，所以抽成共享原语；两处只在**入口鉴权**上不同。
//!
//! 为什么 cookie 必须留在服务端：Strict cookie 在跨站 iframe（桌面壳
//! `tauri.localhost` 内嵌 `127.0.0.1` 页面）里永远不会被浏览器携带，
//! 浏览器侧持有毫无用处；服务端持有则彻底绕开 SameSite 语义。

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::ws::{WebSocket, WebSocketUpgrade},
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::http::Request as HttpRequest;

/// DSH 签名 cookie 的名字前缀（@deepseek-ai/dsh-client-connection）。
const DSH_COOKIE_PREFIX: &str = "dsh-auth-";

/// 上游建连超时；上游在回环，给小超时。
/// 注意：转发请求**不设总超时**——`.timeout()` 是「发起→响应体读完」
/// 全程计时，会把 `/plugins/events`（HMR SSE）等长连接在时限处腰斩
/// （实测 30s 截断，浏览器报 ERR_INCOMPLETE_CHUNKED_ENCODING）。回环
/// 上游真死时 TCP 立即关闭，流自然报错，不需要总超时兜底。
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// 转发请求体的缓冲上限（64 MiB）：重试需要可重放的体，但拒绝无界内存。
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// 跳过转发的逐跳头（RFC 7230）；`set-cookie` 也不外泄——cookie 归服务端持有。
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

/// 调用方不该漏给 DSH 的头：
/// - `host`：DSH 的 Host/Origin 栅栏按 Host 判定，必须让它看到代理→DSH
///   的真实 authority（reqwest 按 URL 自动设置）；
/// - `cookie`：调用方侧的凭据（qx_token 等）不归 DSH。
///
/// `origin` 与 `sec-fetch-*` **不**剥除：浏览器 → 回环代理 → 上游 DSH 是
/// 同主机同端口的转发语义（iframe 在 `127.0.0.1:<gateway>` 上发起
/// XHR/fetch，到 `127.0.0.1:<dsh>` 的 DSH；两者都是回环 127.0.0.1），上
/// 游 dshmarket 等插件的 same-origin 守卫需要看见 `Origin`/`Sec-Fetch-Site`
/// 才能放行。**前提是 DSH 仅以回环上游形态被千寻启动**（千寻启动时
/// `--host` 强制 `127.0.0.1`，见 harness::supervisor）；如果哪天支持非回
/// 环上游，这条假设需要在网关层重新审视（应只对回环上游合成同源头）。
const SKIP_UPSTREAM_HEADERS: [&str; 2] = ["host", "cookie"];

/// 共享上游状态：DSH origin（host:port）+ 完整就绪 URL（含 `?token=`）+
/// 兑换出的签名 cookie。内部字段可热更新（DSH revive 后端口/ token 变化
/// 时由 sync 刷新），axum State 克隆共享同一份。
#[derive(Clone)]
pub struct Upstream {
    inner: Arc<Mutex<Inner>>,
    client: reqwest::Client,
}

struct Inner {
    /// DSH origin，形如 `127.0.0.1:17300`（不含 scheme，边界归一化保证）。
    origin: String,
    /// DSH 就绪时的完整 URL（含 `?token=`）；None = DSH 还没就绪。
    /// 兑换 cookie 时整段打过去（`GET {dsh_url}`），不拆 token——
    /// 拆出再拼回容易在传输里漏字符。
    dsh_url: Option<String>,
    /// 兑换出的 `dsh-auth-*` cookie（Name=Value），跨请求复用。
    cookie: Option<String>,
}

/// origin 契约归一化：调用方给的是 supervisor 的 `origin`（带 scheme，
/// `http://127.0.0.1:17300`）或裸 host:port，这里统一剥成 host:port——
/// 拼接上游 URL 时只在这里加一次 scheme。旧网关靠 upstream_host() 散落
/// 剥 scheme，重构后收敛到边界一处。
fn normalize_origin(origin: impl Into<String>) -> String {
    let origin = origin.into();
    let bare = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .unwrap_or(&origin);
    bare.trim_end_matches('/').to_owned()
}

impl Upstream {
    /// 建立上游状态。`origin` 带 scheme（`http://127.0.0.1:17300`）或裸
    /// host:port 均可，内部归一化；DSH 未就绪时给占位（`127.0.0.1:0`），
    /// 就绪事件再热更新。
    pub fn new(origin: impl Into<String>, dsh_url: Option<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                origin: normalize_origin(origin),
                dsh_url,
                cookie: None,
            })),
            client: reqwest::Client::builder()
                .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// DSH 就绪：热更新 origin 与启动 URL 并作废缓存 cookie。
    /// 生产路径走的是 fingerprint 触发整网关重建（dsh_url 已纳入指纹），
    /// 本方法作为精细化备选保留：测试与未来「不想重启只换 dsh_url」
    /// 的场景。#[allow(dead_code)] 是因为当前 sync 不会调它。
    #[allow(dead_code)]
    pub async fn set_ready(&self, origin: impl Into<String>, dsh_url: String) {
        let mut inner = self.inner.lock().await;
        inner.origin = normalize_origin(origin);
        inner.dsh_url = Some(dsh_url);
        inner.cookie = None;
    }

    /// DSH 停止：作废启动 URL 与 cookie；origin 保留。
    /// 同 set_ready，备选 API。
    #[allow(dead_code)]
    pub async fn set_dsh_gone(&self) {
        let mut inner = self.inner.lock().await;
        inner.dsh_url = None;
        inner.cookie = None;
    }

    /// 当前 origin（host:port）。异步是因为字段可热更新。
    pub async fn origin(&self) -> String {
        self.inner.lock().await.origin.clone()
    }

    /// DSH 是否就绪（启动 URL 已就位）。移动端 `/qx-mobile/info`
    /// 用它做在线探测，App 端据此显示连接在线状态。
    pub async fn ready(&self) -> bool {
        self.inner.lock().await.dsh_url.is_some()
    }

    /// 取回上游可用的 DSH cookie：优先缓存，缺失时兑换一次。
    /// 持锁跨越兑换：并发的首批请求只触发一次兑换（同旧网关语义）。
    pub async fn ensure_cookie(&self) -> Option<String> {
        let mut inner = self.inner.lock().await;
        if let Some(cookie) = inner.cookie.as_ref() {
            return Some(cookie.clone());
        }
        let cookie = exchange_cookie(inner.dsh_url.as_deref()?).await?;
        inner.cookie = Some(cookie.clone());
        Some(cookie)
    }

    /// 缓存作废并强制重兑（上游 401 = cookie 失效，比如 DSH_HOME 被换）。
    pub async fn refresh_cookie(&self) -> Option<String> {
        let mut inner = self.inner.lock().await;
        inner.cookie = None;
        let cookie = exchange_cookie(inner.dsh_url.as_deref()?).await?;
        inner.cookie = Some(cookie.clone());
        Some(cookie)
    }

    #[cfg(test)]
    /// 测试专用：直接注入缓存 cookie，免去起真 DSH。
    pub(crate) async fn test_set_cookie(&self, cookie: &str) {
        self.inner.lock().await.cookie = Some(cookie.to_owned());
    }
}

/// 用 DSH 就绪时打印的完整 URL（已含 `?token=`）直接 `GET` 兑换签名 cookie。
/// 失败返回 None（DSH 未就绪/URL 无效/旧版无鉴权——旧版无重定向即失败，
/// 转发路径会以无 cookie 裸连继续工作）。
async fn exchange_cookie(dsh_url: &str) -> Option<String> {
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(EXCHANGE_TIMEOUT)
        .build()
        .ok()?
        .get(dsh_url)
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

/// HTTP 转发：方法/路径透传，响应流式回写（SSE 也走这条路）。
/// 调用方头按 [`SKIP_UPSTREAM_HEADERS`] 剥除 host/cookie（host 由
/// reqwest 按 URL 自动设置，cookie 换成服务端持有的 dsh-auth-*）；
///
/// `Origin` 与 `Sec-Fetch-Site` 在网关这一层**改写**为与上游 DSH 同源
/// 的值，避免透传过来的 iframe 真实 origin（端口不同）触发 DSH 的连
/// 接栅栏与 dshmarket 等插件的 same-origin 守卫报 403「untrusted origin」。
/// 前提：DSH 永远以 `127.0.0.1:<port>` 的回环形态被千寻启动（见
/// harness::supervisor），代理转发的两端是同主机的同源关系，合成同源
/// 头是合法的。
///
/// 上游 401 时无条件作废缓存并重兑一次（覆盖 cookie 失效与首批请求与
/// 兑换并发赛跑两种情形；无 dsh_url 时 refresh 返回 None，把 401 原样
/// 回给调用方）。
pub(crate) async fn forward(upstream: &Upstream, request: Request<Body>) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let url = format!(
        "http://{}{}{}",
        upstream.origin().await,
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

    // 一次锁定上游状态取出 authority，整轮转发（首次 + 401 重试）复用——
    // 401 重试走 build_request 闭包时不能再 .await，否则就不是 Fn 了。
    // DSH revive 期间 origin 会热更新，但 forge 整转发只在一个就绪窗口内，
    // 一次锁拿到的 authority 与 cookie 是同一时刻的快照，自洽。
    let upstream_authority = upstream.inner.lock().await.origin.clone();
    let synthetic_origin = format!("http://{}", upstream_authority);
    let caller_had_origin = parts
        .headers
        .iter()
        .any(|(name, _)| name.as_str() == "origin");
    let caller_had_sec_fetch_site = parts
        .headers
        .iter()
        .any(|(name, _)| name.as_str() == "sec-fetch-site");

    let build_request = |client: &reqwest::Client, cookie: &Option<String>| {
        // 不设 .timeout()：转发的也有 SSE/流式响应，总超时会腰斩长连接
        // （原因见 UPSTREAM_CONNECT_TIMEOUT 处注释）。
        let mut outgoing = client.request(method.clone(), &url);
        for (name, value) in parts.headers.iter() {
            if HOP_HEADERS.contains(&name.as_str())
                || SKIP_UPSTREAM_HEADERS.contains(&name.as_str())
            {
                continue;
            }
            // Origin 必须**重写**为上游 DSH 的 authority，而非透传 iframe
            // 真实的 origin（`http://127.0.0.1:<gateway>`）：DSH 的连接栅栏
            // 按 `Origin === Host` 判定同源，端口不同的两套 origin 一定 403；
            // dshmarket 的 sameOrigin() 也走同一比较。两边都期望 Origin 等于
            // 上游 DSH 的 host:port，所以这里覆盖式写最干净。透传会让
            // dshmarket 全部安装 POST 报 403「untrusted origin」。
            if name.as_str() == "origin" {
                outgoing = outgoing.header(reqwest::header::ORIGIN, &synthetic_origin);
                continue;
            }
            // Sec-Fetch-Site：浏览器对跨站 iframe 的元数据，理论上代理无权
            // 编造。这里把上游视作与 iframe 同主机（都是 127.0.0.1 的回环
            // 转发），合成 `same-origin` 让 dshmarket 等插件的 same-origin
            // 守卫走最宽松的快路径；DSH 不依赖此头。
            if name.as_str() == "sec-fetch-site" {
                outgoing = outgoing.header("sec-fetch-site", "same-origin");
                continue;
            }
            // 调用方的 Accept-Encoding 一律丢弃：网关的 reqwest 未启用解压
            // 特性，上游若按它回 gzip/br，网关既解不了也无法对压缩字节做
            // 移动层注入（手机 WebView 必发 gzip，曾致「非 UTF-8」502）。
            // 统一以 identity 请求；SSE/流式响应与 identity 无冲突。
            if name.as_str() == "accept-encoding" {
                continue;
            }
            if let Ok(header_value) = value.to_str() {
                outgoing = outgoing.header(name.as_str(), header_value);
            }
        }
        // 调用方没带 origin 时也补一个——例如纯 fetch 没设 mode / 不触发
        // 浏览器 Origin；不补的话 dshmarket fallback 比对 Host 失败。
        if !caller_had_origin {
            outgoing = outgoing.header(reqwest::header::ORIGIN, &synthetic_origin);
        }
        // 调用方没带 sec-fetch-site 时同样补一个：dshmarket 2026-09-06 的本
        // 地补丁优先看 Sec-Fetch-Site，没有的话退回 Origin/Host 比对，已
        // 经会被上一条 Origin 修好；这里补是为了一次过 fast-path。
        if !caller_had_sec_fetch_site {
            outgoing = outgoing.header("sec-fetch-site", "same-origin");
        }
        outgoing = outgoing.header("accept-encoding", "identity");
        if let Some(cookie) = cookie {
            outgoing = outgoing.header(reqwest::header::COOKIE, cookie);
        }
        if !body_bytes.is_empty() {
            outgoing = outgoing.body(body_bytes.clone());
        }
        outgoing
    };

    let cookie = upstream.ensure_cookie().await;
    match build_request(&upstream.client, &cookie).send().await {
        Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
            // cookie 失效（DSH_HOME 被替换）或首批请求跑在兑换前面：
            // refresh 作废缓存并重兑（dsh_url 未就绪时返回 None，原样回 401）。
            let Some(fresh) = upstream.refresh_cookie().await else {
                return relay(resp).await;
            };
            match build_request(&upstream.client, &Some(fresh)).send().await {
                Ok(retried) => relay(retried).await,
                Err(cause) => unreachable_upstream(&cause),
            }
        }
        Ok(resp) => relay(resp).await,
        Err(cause) => unreachable_upstream(&cause),
    }
}

/// 把上游响应转成代理响应：流式回写，逐跳头与 DSH 的 set-cookie 不外泄。
async fn relay(upstream: reqwest::Response) -> Response<Body> {
    let mut response = Response::builder().status(upstream.status().as_u16());
    for (name, value) in upstream.headers().iter() {
        if HOP_HEADERS.contains(&name.as_str()) {
            continue;
        }
        response = response.header(name.clone(), value.clone());
    }
    // 流错误必须留痕：SSE/流式响应中途断开只有这里能观察到。
    let stream = upstream.bytes_stream().map_err(|cause| {
        crate::logging::log("warn", &format!("[forward] 上游响应流中断：{cause}"));
        std::io::Error::other(cause)
    });
    response
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| plain(StatusCode::BAD_GATEWAY, "响应构造失败"))
}

fn unreachable_upstream(cause: &reqwest::Error) -> Response<Body> {
    crate::logging::log("warn", &format!("[forward] DSH 上游不可达：{cause}"));
    plain(
        StatusCode::BAD_GATEWAY,
        &format!("DSH 上游不可达（{cause}）：请确认千寻内 DSH 正在运行"),
    )
}

/// 随机生成 16 字节 base64 编码作为 `Sec-WebSocket-Key`。
/// 遵循 RFC 6455 §4.1：客户端必须发送一个 16 字节随机 nonce，base64 编码；
/// 服务端回 `Sec-WebSocket-Accept = base64(sha1(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))`。
/// `tokio_tungstenite::tungstenite::client::generate_key` 在 0.26 不导出
///（0.28 才公开），自己生成即可——规则同 RFC。
fn generate_ws_key() -> String {
    use base64::Engine as _;
    let mut bytes = [0u8; 16];
    rand::Rng::fill(&mut rand::thread_rng(), &mut bytes[..]);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// WS 升级入口的公共段：构造同路径的上游请求、附 cookie，交给双向桥。
/// `scheme_host` 形如 `127.0.0.1:17300`（ws:// 由这里拼）。
///
/// 重要：tungstenite 0.26 的 `IntoClientRequest for Request` **不会自动加
/// WS upgrade 头**——`generate_request` 只校验已有头。我们必须自己塞
/// `Sec-WebSocket-Key` / `Version: 13` / `Connection: Upgrade` /
/// `Upgrade: websocket`，否则上游 DSH 报
/// "Missing, duplicated or incorrect header sec-websocket-key"，桥被拒
/// （实测：连接秒断成 1006，无任何错误信息回流到浏览器）。
pub(crate) async fn upgrade_ws(
    upstream: &Upstream,
    ws: WebSocketUpgrade,
    path: &str,
) -> Response<Body> {
    let origin = upstream.origin().await;
    let mut request = match HttpRequest::builder()
        .method("GET")
        .uri(format!("ws://{origin}{path}"))
        // Host 显式给：HTTP/1.1 硬性要求，Node 侧的 DSH 缺 Host 直接 400；
        // 不赌 tungstenite 是否代填。
        .header("Host", &origin)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_ws_key())
        .body(())
    {
        Ok(request) => request,
        Err(cause) => {
            return plain(
                StatusCode::BAD_GATEWAY,
                &format!("上游请求构造失败：{cause}"),
            )
        }
    };
    if let Some(cookie) = upstream.ensure_cookie().await {
        if let Ok(value) = cookie.parse() {
            request.headers_mut().insert("cookie", value);
        }
    }
    let path = path.to_owned();
    ws.on_upgrade(move |client| bridge_ws(client, request, path))
        .into_response()
}

/// 双向桥：浏览器 WS ↔ DSH WS（帧级透传，ping/pong/close 各自终结）。
/// 全程留痕：上游连接成败（含原因）与两侧泵的结束原因都进日志——
/// 上游连接失败时浏览器只会看到裸 1006（无 close 帧），不落日志无法归因。
async fn bridge_ws(client: WebSocket, upstream_request: HttpRequest<()>, path: String) {
    let (upstream, _) = match tokio_tungstenite::connect_async(upstream_request).await {
        Ok(pair) => pair,
        Err(cause) => {
            crate::logging::log(
                "warn",
                &format!("[ws-bridge] {path} 上游连接失败：{cause}（浏览器侧表现为 1006）"),
            );
            return;
        }
    };
    crate::logging::log("info", &format!("[ws-bridge] {path} 上游已连通"));
    let (mut client_tx, mut client_rx) = client.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    // 两侧泵各自跑到对端关闭/出错为止；结束原因转文字进收尾日志。
    let to_upstream = async {
        loop {
            match client_rx.next().await {
                Some(Ok(frame)) => {
                    if upstream_tx.send(to_tungstenite(frame)).await.is_err() {
                        break "上行发送失败（上游已断）".to_owned();
                    }
                }
                Some(Err(cause)) => break format!("浏览器侧读错误：{cause}"),
                None => break "浏览器侧正常关闭".to_owned(),
            }
        }
    };
    let to_client = async {
        loop {
            match upstream_rx.next().await {
                Some(Ok(frame)) => {
                    if client_tx.send(to_axum(frame)).await.is_err() {
                        break "下行发送失败（浏览器已断）".to_owned();
                    }
                }
                Some(Err(cause)) => break format!("上游侧读错误：{cause}"),
                None => break "上游侧正常关闭".to_owned(),
            }
        }
    };
    let (up_result, down_result) = tokio::join!(to_upstream, to_client);
    crate::logging::log(
        "info",
        &format!("[ws-bridge] {path} 连接结束：上行[{up_result}]，下行[{down_result}]"),
    );
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

// ---- 小工具（两个入口共用） ----

pub(crate) fn plain(status: StatusCode, text: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Body::from(text.to_owned()))
        .unwrap_or_else(|_| Response::new(Body::from(text.to_owned())))
}

pub(crate) fn query_param(query: &str, key: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin归一化剥scheme与尾斜杠() {
        // supervisor 的 origin 带 scheme（http://127.0.0.1:17300）；
        // 占位与部分调用方给裸 host:port。边界统一归一化。
        let upstream = Upstream::new("http://127.0.0.1:17300", None);
        assert_eq!(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(upstream.origin()),
            "127.0.0.1:17300"
        );
        let upstream = Upstream::new("127.0.0.1:0/", None);
        assert_eq!(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(upstream.origin()),
            "127.0.0.1:0"
        );
    }

    #[test]
    fn set_ready的origin同样归一化() {
        let upstream = Upstream::new("127.0.0.1:0", None);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            upstream
                .set_ready(
                    "http://127.0.0.1:17300",
                    "http://127.0.0.1:17300/?token=t".into(),
                )
                .await;
            assert_eq!(upstream.origin().await, "127.0.0.1:17300");
        });
    }

    #[test]
    fn 未就绪时ensure_cookie返回none而不报错() {
        let upstream = Upstream::new("127.0.0.1:17300", None);
        assert_eq!(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { upstream.ensure_cookie().await }),
            None
        );
    }

    #[test]
    fn 缓存命中不再兑换() {
        let upstream = Upstream::new("127.0.0.1:17300", None);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            upstream.test_set_cookie("dsh-auth-x=v1.sig").await;
            // dsh_url 是 None：若错误地走兑换会返回 None，命中缓存才拿得到。
            assert_eq!(
                upstream.ensure_cookie().await.as_deref(),
                Some("dsh-auth-x=v1.sig")
            );
        });
    }

    #[test]
    fn set_ready作废旧cookie() {
        let upstream = Upstream::new("127.0.0.1:0", None);
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            upstream.test_set_cookie("dsh-auth-old=stale").await;
            upstream
                .set_ready("127.0.0.1:17300", "http://127.0.0.1:17300/?token=t".into())
                .await;
            assert_eq!(upstream.origin().await, "127.0.0.1:17300");
            // cookie 已被作废且 dsh_url 指向假地址 → 兑换失败 → None。
            assert_eq!(upstream.ensure_cookie().await, None);
        });
    }

    #[test]
    fn query_param取值并解码() {
        assert_eq!(query_param("a=1&token=ab%20c", "token"), "ab c");
        assert_eq!(query_param("a=1", "b"), "");
        assert_eq!(query_param("token=", "token"), "");
    }

    // ---- 转发头改写（dshmarket same-origin 守卫的根因修复）----

    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode as AxStatusCode};
    use std::sync::Arc;

    /// 拉起一个本地 mock 上游，记下每个请求的 header，跑一次 forward 后
    /// 把观察到的头与合成逻辑的预期对照。
    async fn captured_upstream_headers(
        upstream_origin: &'static str,
        caller_headers: Vec<(&'static str, &'static str)>,
    ) -> (axum::http::HeaderMap, axum::http::HeaderMap) {
        use axum::routing::any;
        use axum::Router;
        use std::sync::Mutex as StdMutex;

        let captured: Arc<StdMutex<axum::http::HeaderMap>> =
            Arc::new(StdMutex::new(axum::http::HeaderMap::new()));
        let observed_origin: Arc<StdMutex<axum::http::HeaderMap>> =
            Arc::new(StdMutex::new(axum::http::HeaderMap::new()));
        let cap = Arc::clone(&captured);
        let obs = Arc::clone(&observed_origin);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let upstream = Upstream::new(format!("127.0.0.1:{}", local.port()), None);
        upstream.test_set_cookie("dsh-auth-test=v1.sig").await;
        let app = Router::new()
            .route(
                "/echo",
                any(move |req: HttpRequest<Body>| {
                    let cap = Arc::clone(&cap);
                    let obs = Arc::clone(&obs);
                    async move {
                        *cap.lock().unwrap() = req.headers().clone();
                        // 单独观察 Origin 与 Sec-Fetch-Site，便于断言。
                        let mut only = axum::http::HeaderMap::new();
                        for name in ["origin", "sec-fetch-site"] {
                            if let Some(v) = req.headers().get(name) {
                                only.insert(
                                    name.parse::<axum::http::HeaderName>().unwrap(),
                                    v.clone(),
                                );
                            }
                        }
                        *obs.lock().unwrap() = only;
                        AxStatusCode::NO_CONTENT
                    }
                }),
            )
            .with_state(());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // 给 server 一瞬间起一下（accept 第一个连接前 listener 已就绪）。
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut builder = HttpRequest::builder()
            .method("POST")
            .uri("/echo")
            .header("host", "127.0.0.1:23090")
            .header("content-type", "application/json");
        for (k, v) in caller_headers {
            builder = builder.header(k, v);
        }
        let request = builder.body(Body::from("{}")).unwrap();
        let _ = forward(&upstream, request).await;

        server.abort();
        let _ = server.await;
        let _ = upstream_origin; // 静态标记：caller 期望 origin 形如上游 authority
        let captured_snapshot = captured.lock().unwrap().clone();
        let observed_snapshot = observed_origin.lock().unwrap().clone();
        (captured_snapshot, observed_snapshot)
    }

    #[tokio::test]
    async fn 调用方未带origin时合成与上游同源的origin() {
        // 用端口 0 占位：mock 上游拿到的是 OS 派发的真实端口，所以断言
        // 拿到 origin 后从里面解析端口号不为 0（即 mock 真的可达）。
        let (_, observed) = captured_upstream_headers(
            "127.0.0.1:0",
            vec![], // 完全不带 Origin/Sec-Fetch-Site
        )
        .await;
        let origin = observed
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let port: u16 = origin
            .strip_prefix("http://127.0.0.1:")
            .and_then(|s| s.parse().ok())
            .expect("synthetic origin 形如 http://127.0.0.1:<port>");
        assert!(
            port > 0,
            "网关必须为上游 DSH 合成同源 Origin（端口非占位 0）：{origin}"
        );
        assert_eq!(
            observed.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
            Some("same-origin"),
            "网关必须补 Sec-Fetch-Site=same-origin，让 dshmarket 2026-09-06 补丁走快路径"
        );
    }

    #[tokio::test]
    async fn 调用方iframe的真实origin被改写为上游authority() {
        // iframe 实际是 `http://127.0.0.1:23090`（网关端口），透传到 DSH
        // 会被栅栏与 dshmarket 同时判 403——验证我们把它改写成上游 DSH
        // 的真实 authority（不是透传那个 23090）。
        let (_, observed) = captured_upstream_headers(
            "127.0.0.1:0",
            vec![
                ("origin", "http://127.0.0.1:23090"),
                ("sec-fetch-site", "cross-site"),
            ],
        )
        .await;
        let origin = observed
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            !origin.contains(":23090"),
            "iframe 真实 origin 必须改写为上游 DSH 的 authority，不能透传 23090：{origin}"
        );
        assert!(
            origin.starts_with("http://127.0.0.1:"),
            "synthetic origin 形如 http://127.0.0.1:<port>：{origin}"
        );
        assert_eq!(
            observed.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
            Some("same-origin"),
            "跨站 Sec-Fetch-Site 也必须改写，否则 dshmarket 直接拒"
        );
    }

    #[tokio::test]
    async fn host与cookie不被透传() {
        // host 由 reqwest 按 URL 自动设；cookie 由网关换上服务端持有的
        // dsh-auth-*；两者都不该带着调用方原值去上游。
        let (captured, _) =
            captured_upstream_headers("127.0.0.1:0", vec![("cookie", "qx_token=should-not-leak")])
                .await;
        // 上游看到的是网关代发的 dsh-auth-*，不是调用方的 qx_token。
        let cookie = captured
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            cookie.contains("dsh-auth-test="),
            "上游 cookie 必须来自服务端持有：{cookie}"
        );
        assert!(
            !cookie.contains("qx_token"),
            "调用方 qx_token 不该泄到上游 DSH：{cookie}"
        );
    }
}

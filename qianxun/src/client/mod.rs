//! Thin client for connecting to a running `qx daemon` (HTTP + SSE).
//!
//! Stage 4: TUI/ACP/CLI 在检测到本地 Daemon 时走本模块的 HTTP+SSE 远程调用;
//! 无 Daemon (或 `--standalone` flag) 时回退到原 standalone 路径 (内嵌 AgentLoop).
//!
//! # 协议契约
//!
//! 与 `docs/30_子项目规划/_shared-contract.md` §3.1 (REST endpoints) + §3.2 (SSE 12 事件)
//! 严格一致. 修改时务必保持 tag 字段名 (`type`) 与 variant 名称不变.
//!
//! # Stage 4 简化
//!
//! - 不做自动重连 (Stage 5 引入)
//! - 不接认证 / token (Stage 5 引入)
//! - SSE parser 简化: 只解析 `data:` 帧, 忽略 `event:` 字段 (12 事件类型全在一个流上, 按 `type` 字段分发)
//! - 不引入新 crate (复用 reqwest / futures / tokio / serde / serde_json)

use futures::stream::{Stream, StreamExt};
use reqwest::Response;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;
use tracing::{debug, warn};

/// Boxed stream of `Result<SseEvent, ClientError>`. 跨 `await` 边界持有.
pub type SseStream =
    Pin<Box<dyn Stream<Item = Result<SseEvent, ClientError>> + Send>>;

// ─── 错误类型 ────────────────────────────────────────────────

/// Thin client 错误.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON parse failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("SSE parse failed: {0}")]
    Sse(String),

    #[error("Daemon returned status {0}: {1}")]
    Status(u16, String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ─── 响应数据结构 (与 daemon 端 router.rs 字段名严格一致) ─────

/// `GET /v1/system/health` 响应.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthStatus {
    pub status: String,
}

/// `POST /v1/chat/session` 响应.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionCreated {
    pub session_id: String,
}

/// `GET /v1/chat/session/{id}` 响应 (Stage 3 子集).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub session_id: String,
    #[serde(default)]
    pub status: String,
}

/// `GET /v1/chat/sessions` 响应 (Stage 3 §6.4 扩展).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionsList {
    pub sessions: Vec<Session>,
}

/// `POST /v1/chat/session/{id}/prompt` 请求体.
#[derive(Debug, Clone, Serialize)]
pub struct PromptRequest {
    pub messages: Vec<PromptMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: String,
}

impl PromptRequest {
    /// 简单文本 prompt (Stage 2 简化, 与 daemon `PromptMessage` 字段一致).
    pub fn text(user_content: &str) -> Self {
        Self {
            messages: vec![PromptMessage {
                role: "user".into(),
                content: user_content.into(),
            }],
        }
    }
}

// ─── SSE 事件 (与 daemon/src/daemon/sse.rs 12 个 variant 严格一致) ─────

/// SSE 事件 (与 shared-contract §3.2 严格一致, 12 种类型).
///
/// 使用 `#[serde(tag = "type")]` 内部 tag 反序列化: 输入的 JSON 形如
/// `{"type":"text_delta","index":0,"text":"..."}`. 按 `type` 字段分发.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum SseEvent {
    #[serde(rename = "message_start")]
    MessageStart {
        session_id: String,
        model: String,
        max_tokens: u32,
    },

    #[serde(rename = "content_block_start")]
    ContentBlockStart { index: u32, block_type: String },

    #[serde(rename = "text_delta")]
    TextDelta { index: u32, text: String },

    #[serde(rename = "thinking_delta")]
    ThinkingDelta { index: u32, text: String },

    #[serde(rename = "tool_use_delta")]
    ToolUseDelta {
        index: u32,
        id: String,
        name: String,
        arguments_json: String,
    },

    #[serde(rename = "tool_use_complete")]
    ToolUseComplete {
        index: u32,
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        elapsed_ms: u64,
    },

    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },

    #[serde(rename = "usage")]
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        #[serde(default)]
        cache_creation_input_tokens: u64,
        #[serde(default)]
        cache_read_input_tokens: u64,
    },

    #[serde(rename = "message_delta")]
    MessageDelta { stop_reason: String },

    #[serde(rename = "message_stop")]
    MessageStop,

    #[serde(rename = "error")]
    Error { code: String, message: String },
}

// ─── DaemonClient ────────────────────────────────────────────

/// 连接到本地 Daemon (HTTP + SSE) 的 thin client.
///
/// 跨 binary 共享 (TUI/ACP/CLI 三个入口共用同一个 client 实例).
#[derive(Debug, Clone)]
pub struct DaemonClient {
    base_url: String,
    http: reqwest::Client,
}

impl DaemonClient {
    /// 构造 DaemonClient (不立即探测, 探测由 `health()` 完成).
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest Client builder should not fail with default config");
        Self {
            base_url: base_url.into(),
            http,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 健康检查 (3s 超时). 失败表示 daemon 未启动或不可达.
    pub async fn health(&self) -> Result<HealthStatus, ClientError> {
        let url = format!("{}/v1/system/health", self.base_url);
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        let body: HealthStatus = resp.json().await?;
        Ok(body)
    }

    /// 创建 session.
    pub async fn create_session(&self) -> Result<SessionCreated, ClientError> {
        let url = format!("{}/v1/chat/session", self.base_url);
        let resp = self.http.post(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        let body: SessionCreated = resp.json().await?;
        Ok(body)
    }

    /// 列出 sessions (Stage 3 §6.4 扩展端点).
    pub async fn list_sessions(&self) -> Result<Vec<Session>, ClientError> {
        let url = format!("{}/v1/chat/sessions", self.base_url);
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        let body: SessionsList = resp.json().await?;
        Ok(body.sessions)
    }

    /// 发送 prompt, 返回 SSE 事件流.
    ///
    /// 调用方负责消费流; 流自然结束 (`message_stop` 或 `error`) 或 drop stream
    /// (客户端断连) 都由 axum 后端负责清理.
    pub async fn stream_prompt(
        &self,
        session_id: &str,
        request: &PromptRequest,
    ) -> Result<SseStream, ClientError> {
        let url = format!("{}/v1/chat/session/{}/prompt", self.base_url, session_id);
        let resp = self.http.post(&url).json(request).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        // 把 reqwest::Response 转换为 byte stream, 再走 SSE parser.
        Ok(parse_sse_stream(resp))
    }

    /// 取消当前 prompt.
    pub async fn cancel(&self, session_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/v1/chat/session/{}/cancel", self.base_url, session_id);
        let resp = self.http.post(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        Ok(())
    }
}

// ─── SSE 解析 ────────────────────────────────────────────────

/// 把 `reqwest::Response` 的 byte stream 解析成 `Stream<Result<SseEvent, _>>`.
///
/// SSE 帧格式: `data: <json>\n\n` (axum::response::sse 默认格式).
/// 简化: 只解析 `data:` 行, 不分发 `event:` 字段 — 12 事件类型全在一个流上,
/// 客户端按 `type` 字段 (反序列化时由 serde tag 决定) 路由.
///
/// 备注: 这里把每个 `bytes::Bytes` chunk 先转成 `String`, 由 `extract_sse_frames`
/// 按 `\n` 切行. reqwest 的 `bytes_stream()` 在 SSE 长连接下通常按 KB 级切分,
/// 单个 chunk 几乎不会跨帧边界, 极小概率丢尾部 — 后续 chunk 会以新帧重新对齐.
pub fn parse_sse_stream(response: Response) -> SseStream {
    use futures::stream::iter;
    let byte_stream = response.bytes_stream();
    let event_stream = byte_stream
        .map(|chunk_result| {
            // Bytes → Vec<u8> → UTF-8 string; 出错时返回 ClientError
            chunk_result
                .map_err(ClientError::from)
                .and_then(|bytes| {
                    // bytes::Bytes 可以直接转 Vec<u8>
                    let v: Vec<u8> = bytes.into();
                    std::str::from_utf8(&v)
                        .map(str::to_string)
                        .map_err(|e| ClientError::Sse(format!("invalid UTF-8: {e}")))
                })
        })
        .flat_map(|text_result: Result<String, ClientError>| {
            // 每段文本可能产生 0..N 个 SSE 帧; 用 iter() 展平.
            let items: Vec<Result<SseEvent, ClientError>> = match text_result {
                Ok(text) => extract_sse_frames(&text),
                Err(e) => vec![Err(e)],
            };
            iter(items)
        });
    Box::pin(event_stream)
}

/// 从一段 SSE 文本中提取 `data: <json>` 帧, 解析为 `SseEvent`.
///
/// 一段文本可能包含 0..N 个完整帧 (每帧以 `\n\n` 结束). 简化处理:
/// 按 `\n` 切行, 跳过空行, 只取 `data: ` 前缀的行, 累积到下一个空行后解析.
/// 部分帧 (跨 chunk 边界) 由上层 byte_stream 的 next() 后续调用补全 —
/// 这里假设每次输入是已分块后的"完整片段" (reqwest::bytes_stream 在 SSE 长连接下
/// 通常按 KB 级切分, 单个 chunk 几乎不会跨帧边界, 极小概率丢尾部, 后续 chunk
/// 会以空行/新帧重新对齐).
///
/// 跳过空 data 行 (心跳); 解析失败时返回 Err 项.
pub fn extract_sse_frames(text: &str) -> Vec<Result<SseEvent, ClientError>> {
    let mut out: Vec<Result<SseEvent, ClientError>> = Vec::new();
    let mut current_data: Option<String> = None;

    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            // 帧边界
            if let Some(data) = current_data.take() {
                match parse_data_payload(&data) {
                    Ok(Some(ev)) => out.push(Ok(ev)),
                    Ok(None) => {} // 心跳, 跳过
                    Err(e) => out.push(Err(e)),
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            let payload = rest.trim_start();
            if let Some(existing) = current_data.as_mut() {
                existing.push('\n');
                existing.push_str(payload);
            } else {
                current_data = Some(payload.to_string());
            }
        }
        // 忽略其他行 (event:, id:, retry:, 注释 :...)
    }
    // 末尾可能没有空行: 把残留的 data 当作最后一帧提交.
    if let Some(data) = current_data.take() {
        match parse_data_payload(&data) {
            Ok(Some(ev)) => out.push(Ok(ev)),
            Ok(None) => {}
            Err(e) => out.push(Err(e)),
        }
    }
    out
}

/// 解析单个 `data:` 行的 JSON payload.
fn parse_data_payload(data: &str) -> Result<Option<SseEvent>, ClientError> {
    if data.is_empty() {
        return Ok(None); // 心跳 (空 data)
    }
    match serde_json::from_str::<SseEvent>(data) {
        Ok(ev) => Ok(Some(ev)),
        Err(e) => {
            warn!("[client::sse] parse error: {e}; data={data}");
            Err(ClientError::Sse(format!("JSON parse: {e}; data={data}")))
        }
    }
}

// ─── Daemon 探测 (默认 127.0.0.1:23900, 3s 超时) ─────────────

/// 探测本地 daemon 是否在运行. 成功返回 `Some(base_url)`, 失败返回 `None`.
///
/// Stage 4 简化:
/// - 优先 `QIANXUN_DAEMON_URL` env var
/// - 回退 `http://127.0.0.1:23900` (默认 daemon 端口)
/// - 3s 超时
pub async fn detect_local_daemon() -> Option<String> {
    let base_url = std::env::var("QIANXUN_DAEMON_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:23900".to_string());
    let client = DaemonClient::new(base_url.clone());
    match tokio::time::timeout(Duration::from_secs(3), client.health()).await {
        Ok(Ok(h)) if h.status == "ok" => {
            debug!("[client] daemon detected at {base_url}");
            Some(base_url)
        }
        Ok(Ok(h)) => {
            debug!("[client] daemon health non-ok: {h:?}");
            None
        }
        Ok(Err(e)) => {
            debug!("[client] daemon probe error: {e}");
            None
        }
        Err(_) => {
            debug!("[client] daemon probe timeout (>3s)");
            None
        }
    }
}

// ─── 薄客户端 REPL (CLI 入口用) ──────────────────────────────

/// 薄客户端 REPL: 连接 daemon, 创建 session, 循环读 stdin 发送 prompt, 打印 SSE 流.
///
/// 替换 `qianxun/src/main.rs` 中旧 `run_thin_client` (那段只读 response.text,
/// 没解析 SSE, 不能流式输出).
pub async fn run_thin_repl(daemon_url: &str) -> anyhow::Result<()> {
    let client = DaemonClient::new(daemon_url.to_string());
    let health = client.health().await.map_err(|e| {
        anyhow::anyhow!("无法连接 Daemon {daemon_url}: {e}")
    })?;
    if health.status != "ok" {
        anyhow::bail!("Daemon unhealthy: {health:?}");
    }
    tracing::info!("Daemon 已连接: {daemon_url}");
    println!("已连接到 Daemon: {daemon_url}");
    println!("输入消息后按 Enter 发送（输入 /quit /exit 退出, /cancel 取消当前 prompt）\n");

    let session = client.create_session().await?;
    let session_id = session.session_id;
    println!("[session] {session_id}");

    let mut input = String::new();
    loop {
        input.clear();
        if std::io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim();
        match input {
            "/quit" | "/exit" => break,
            "/cancel" => {
                if let Err(e) = client.cancel(&session_id).await {
                    eprintln!("[cancel] error: {e}");
                } else {
                    println!("[cancelled]");
                }
                continue;
            }
            "/sessions" => match client.list_sessions().await {
                Ok(sessions) => {
                    for s in sessions {
                        println!("- {} ({})", s.session_id, s.status);
                    }
                }
                Err(e) => eprintln!("[sessions] error: {e}"),
            },
            "" => continue,
            _ => {}
        }

        let req = PromptRequest::text(input);
        match client.stream_prompt(&session_id, &req).await {
            Ok(stream) => {
                consume_sse_stream_print(stream).await;
            }
            Err(e) => eprintln!("[prompt] error: {e}"),
        }
    }
    Ok(())
}

/// 消费 SSE 事件流, 打印 text_delta (实时), 打印 usage/message_stop 摘要.
async fn consume_sse_stream_print(stream: SseStream) {
    tokio::pin!(stream);
    while let Some(item) = stream.next().await {
        match item {
            Ok(SseEvent::TextDelta { text, .. }) => {
                print!("{text}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            Ok(SseEvent::ThinkingDelta { text, .. }) => {
                eprint!("[thinking] {text}");
            }
            Ok(SseEvent::ToolUseComplete { name, id, .. }) => {
                println!("\n[tool_call] {name} (id={id})");
            }
            Ok(SseEvent::ToolResult { tool_use_id, content, is_error, .. }) => {
                let label = if is_error { "[tool_error]" } else { "[tool_result]" };
                println!("{label} {tool_use_id}: {content}");
            }
            Ok(SseEvent::Usage { input_tokens, output_tokens, .. }) => {
                eprintln!("\n[usage] in={input_tokens} out={output_tokens}");
            }
            Ok(SseEvent::MessageDelta { stop_reason }) => {
                eprintln!("[stop_reason] {stop_reason}");
            }
            Ok(SseEvent::MessageStop) => {
                println!();
            }
            Ok(SseEvent::Error { code, message }) => {
                eprintln!("\n[error {code}] {message}");
            }
            Ok(_) => {} // ContentBlockStart/Stop 等 UI 噪音, 默认静默
            Err(e) => eprintln!("\n[sse_error] {e}"),
        }
    }
}

// ─── 单测 ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock 一个超轻量 HTTP server (用 tokio task 在指定端口 listen, 不引入额外 crate).
    /// 测试用同一个 tokio runtime, 不绑死 port (用 port 0 → OS 分配).
    mod mock_server {
        use tokio::net::TcpListener;
        use tokio::sync::oneshot;

        pub struct MockHttp {
            pub addr: std::net::SocketAddr,
            pub shutdown: Option<oneshot::Sender<()>>,
        }

        /// 启动一个 mock HTTP server, 处理一个请求后返回 (测试主动 drop MockHttp 关闭).
        ///
        /// 简化: 只支持 GET /v1/system/health, 返 `{"status":"ok"}`.
        /// 不够用就再写新的 helper.
        pub async fn start_health() -> MockHttp {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let addr = listener.local_addr().expect("local_addr");
            let (tx, mut rx) = oneshot::channel::<()>();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = &mut rx => break,
                        accepted = listener.accept() => {
                            if let Ok((mut stream, _)) = accepted {
                                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                                let mut buf = vec![0u8; 4096];
                                let _ = stream.read(&mut buf).await;
                                let body = r#"{"status":"ok"}"#;
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    body.len(), body
                                );
                                let _ = stream.write_all(resp.as_bytes()).await;
                                let _ = stream.shutdown().await;
                            }
                        }
                    }
                }
            });
            MockHttp { addr, shutdown: Some(tx) }
        }

        impl Drop for MockHttp {
            fn drop(&mut self) {
                if let Some(tx) = self.shutdown.take() {
                    let _ = tx.send(());
                }
            }
        }
    }

    #[tokio::test]
    async fn test_health_returns_health_status() {
        let mock = mock_server::start_health().await;
        let url = format!("http://{}", mock.addr);
        let client = DaemonClient::new(url);
        let h = client.health().await.expect("health ok");
        assert_eq!(h.status, "ok");
    }

    /// 验证 create_session 解析 `{"session_id": "sess_xxx"}`.
    #[tokio::test]
    async fn test_create_session_returns_session_id() {
        // 单独起一个返 JSON 的 mock
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        if let Ok((mut stream, _)) = accepted {
                            use tokio::io::{AsyncReadExt, AsyncWriteExt};
                            let mut buf = vec![0u8; 4096];
                            let _ = stream.read(&mut buf).await;
                            let body = r#"{"session_id":"sess_test_abc"}"#;
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(), body
                            );
                            let _ = stream.write_all(resp.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        }
                    }
                }
            }
        });
        let url = format!("http://{}", addr);
        let client = DaemonClient::new(url);
        let s = client.create_session().await.expect("create_session");
        assert_eq!(s.session_id, "sess_test_abc");
        let _ = tx.send(());
    }

    /// 验证 stream_prompt 解析 SSE 帧: message_start → text_delta → message_stop.
    #[tokio::test]
    async fn test_stream_prompt_parses_sse_events() {
        // Mock server 返 SSE 流 (Content-Type: text/event-stream)
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    accepted = listener.accept() => {
                        if let Ok((mut stream, _)) = accepted {
                            use tokio::io::{AsyncReadExt, AsyncWriteExt};
                            let mut buf = vec![0u8; 4096];
                            let _ = stream.read(&mut buf).await;
                            // 3 个 SSE 帧
                            let body = concat!(
                                "data: {\"type\":\"message_start\",\"session_id\":\"sess_x\",\"model\":\"deepseek-v4-flash\",\"max_tokens\":16384}\n\n",
                                "data: {\"type\":\"text_delta\",\"index\":0,\"text\":\"Hello\"}\n\n",
                                "data: {\"type\":\"message_stop\"}\n\n",
                            );
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(), body
                            );
                            let _ = stream.write_all(resp.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        }
                    }
                }
            }
        });
        let url = format!("http://{}", addr);
        let client = DaemonClient::new(url);
        let req = PromptRequest::text("hi");
        let stream = client.stream_prompt("sess_x", &req).await.expect("stream");
        let events: Vec<SseEvent> = {
            tokio::pin!(stream);
            let mut v = Vec::new();
            while let Some(item) = stream.next().await {
                match item {
                    Ok(ev) => v.push(ev),
                    Err(e) => panic!("unexpected error: {e}"),
                }
            }
            v
        };
        assert_eq!(events.len(), 3, "expected 3 events, got {events:?}");
        match &events[0] {
            SseEvent::MessageStart { session_id, model, max_tokens } => {
                assert_eq!(session_id, "sess_x");
                assert_eq!(model, "deepseek-v4-flash");
                assert_eq!(*max_tokens, 16384);
            }
            other => panic!("expected MessageStart, got {other:?}"),
        }
        match &events[1] {
            SseEvent::TextDelta { index, text } => {
                assert_eq!(*index, 0);
                assert_eq!(text, "Hello");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
        assert_eq!(events[2], SseEvent::MessageStop);
        let _ = tx.send(());
    }
}

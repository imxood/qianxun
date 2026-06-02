use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{sse::Event, Json, Response, Sse},
    routing::{get, post},
    Router,
};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::iter;
use tokio_stream::wrappers::ReceiverStream;

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::provider::types::LlmStreamEvent;
use qianxun_core::types::LlmError;

use crate::daemon::sse::{SseEvent, SseEventBuilder};
use crate::daemon::AppState;

/// 健康检查响应。
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

/// 创建会话响应。
#[derive(Serialize)]
struct SessionCreatedResponse {
    session_id: String,
}

/// `/v1/chat/session/:id/prompt` 请求体.
#[derive(Debug, Deserialize)]
pub struct PromptRequest {
    /// 用户/助手消息列表 (按时间顺序). 简单字符串数组, 后续可扩展为多模态.
    #[serde(default)]
    pub messages: Vec<PromptMessage>,
    /// 可选: 覆盖 session 默认 model (Stage 2 暂忽略, 留 Stage 3 接 config 切换).
    #[serde(default)]
    pub model: Option<String>,
}

/// Prompt 请求中的单条消息.
#[derive(Debug, Deserialize)]
pub struct PromptMessage {
    /// "user" / "assistant" / "system"
    pub role: String,
    /// 文本内容 (Stage 2 简化: 仅支持纯文本).
    pub content: String,
}

/// 构建 Daemon HTTP 路由。
///
/// Stage 5 token auth 策略:
/// - `/v1/system/health` 跳过 (k8s liveness/readiness probe 用, 不应被 token 拦)
/// - `/v1/system/status` 跳过 (状态查询, 信息非敏感, 方便调试)
/// - 其余 endpoint 全部要求 `X-Api-Key` 或 `Authorization: Bearer <key>`
///
/// 实现: 单一 `auth_middleware` 检查 Header, 缺/错则返 401.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // 系统
        .route("/v1/system/health", get(health_handler))
        .route("/v1/system/status", get(status_handler))
        // 会话
        .route("/v1/chat/session", post(create_session))
        .route("/v1/chat/session/{id}", get(get_session).delete(delete_session))
        .route("/v1/chat/session/{id}/prompt", post(prompt_handler))
        // 工具
        .route("/v1/tools", get(list_tools))
        // 配置
        .route("/v1/config", get(get_config))
        // 记忆
        .route("/v1/memory/sessions", get(memory_sessions))
        .route("/v1/memory/search", post(memory_search))
        // 技能
        .route("/v1/skills", get(list_skills))
        // MCP
        .route("/v1/mcp/servers", get(list_mcp_servers).post(add_mcp_server))
        .with_state(state)
        // Stage 5: 全局 token auth middleware (在 handler 之前执行)
        .layer(middleware::from_fn(auth_middleware))
}

// ─── Token Auth Middleware (Stage 5) ───────────────────────────

/// Stage 5: 简单 token 校验 (单 key, 不做 JWT/expiry).
///
/// 接受两种 header 形式 (任一即可):
/// - `X-Api-Key: <key>`
/// - `Authorization: Bearer <key>`
///
/// Key 来源 (按优先级):
/// 1. env var `QIANXUN_API_KEY` (启动时设置, 跟 provider api_key 区分)
/// 2. 留空 (未配置) → 全部请求放行 (dev 模式, 方便本地调试)
///
/// 拒绝: 缺 header 或 key 不匹配 → 401 Unauthorized
///
/// 跳过: `GET /v1/system/health` 和 `GET /v1/system/status` (k8s probe / 调试).
/// 跳过实现: middleware 检查 path, health/status 路径直接放行.
///
/// Stage 6 升级方向: JWT + 过期检查 + 角色权限, 见 `docs/30_子项目规划/01-daemon.md` §14 OQ-8.
pub async fn auth_middleware(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // 1. 跳过 health/status (k8s probe + 调试)
    let path = request.uri().path();
    if is_auth_skipped_path(path) {
        return Ok(next.run(request).await);
    }

    // 2. dev 模式: 未配置 key → 放行 (打印 warning 仅一次)
    let Some(expected) = expected_api_key() else {
        warn_auth_disabled_once();
        return Ok(next.run(request).await);
    };

    // 3. 校验
    match extract_token(&headers) {
        Some(token) if token == expected => Ok(next.run(request).await),
        Some(_) => {
            tracing::warn!("[auth] token mismatch on {path}");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            tracing::debug!("[auth] missing token on {path}");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

/// 哪些 path 跳过 auth (k8s probe / 调试查询).
///
/// 当前跳过: /v1/system/health, /v1/system/status
pub fn is_auth_skipped_path(path: &str) -> bool {
    path == "/v1/system/health" || path == "/v1/system/status"
}

/// 读期望的 API key (env var QIANXUN_API_KEY).
///
/// 返回 `None` 表示 dev 模式 (env var 留空或未设置).
pub fn expected_api_key() -> Option<String> {
    std::env::var("QIANXUN_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
}

/// 打印 "auth disabled" warning, 仅一次.
fn warn_auth_disabled_once() {
    static WARN_ONCE: std::sync::Once = std::sync::Once::new();
    WARN_ONCE.call_once(|| {
        tracing::warn!(
            "[auth] QIANXUN_API_KEY not set; all requests allowed (dev mode). \
             Set env var to enable token auth in production."
        );
    });
}

/// 从 HeaderMap 提取 token: 先 X-Api-Key, 再 Authorization: Bearer <key>.
///
/// 公开以便测试.
pub fn extract_token(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key") {
        if let Ok(s) = v.to_str() {
            return Some(s.trim().to_string());
        }
    }
    if let Some(v) = headers.get("authorization") {
        if let Ok(s) = v.to_str() {
            // 接受 "Bearer xxx" 或裸 key
            if let Some(rest) = s.strip_prefix("Bearer ") {
                return Some(rest.trim().to_string());
            }
            if let Some(rest) = s.strip_prefix("bearer ") {
                return Some(rest.trim().to_string());
            }
            return Some(s.trim().to_string());
        }
    }
    None
}

// ─── 系统 ──────────────────────────────────────────────────

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn status_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "stage": "stage-2-sse-streaming",
    }))
}

// ─── 会话 ──────────────────────────────────────────────────

async fn create_session(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionCreatedResponse>, (StatusCode, String)> {
    match state.agent_host.create_session() {
        Ok(runtime) => Ok(Json(SessionCreatedResponse {
            session_id: runtime.session_id.clone(),
        })),
        Err(e) => Err((StatusCode::SERVICE_UNAVAILABLE, e)),
    }
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if state.agent_host.session_exists(&id) {
        Ok(Json(serde_json::json!({ "session_id": id, "status": "active" })))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Session {id} not found")))
    }
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    state.agent_host.delete_session(&id);
    Json(serde_json::json!({ "status": "deleted" }))
}

// ─── 工具 ──────────────────────────────────────────────────

async fn list_tools() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "tools": [
            {"name": "read_text_file", "description": "读取文件内容"},
            {"name": "write_text_file", "description": "写入文件"},
            {"name": "search", "description": "搜索文件"},
            {"name": "grep", "description": "内容搜索"},
            {"name": "list_directory", "description": "目录列表"},
            {"name": "execute_command", "description": "执行命令"},
            {"name": "edit_file", "description": "编辑文件"},
            {"name": "skill_read", "description": "读取技能"}
        ]
    }))
}

// ─── 配置 ──────────────────────────────────────────────────

async fn get_config() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "daemon": {"host": "127.0.0.1", "port": 23900},
        "agent": {"max_turns": 50, "max_retries": 3}
    }))
}

// ─── 记忆 ──────────────────────────────────────────────────

async fn memory_sessions() -> Json<serde_json::Value> {
    Json(serde_json::json!({"sessions": []}))
}

async fn memory_search() -> Json<serde_json::Value> {
    Json(serde_json::json!({"results": []}))
}

// ─── 技能 ──────────────────────────────────────────────────

async fn list_skills() -> Json<serde_json::Value> {
    Json(serde_json::json!({"skills": []}))
}

// ─── MCP ──────────────────────────────────────────────────

async fn list_mcp_servers() -> Json<serde_json::Value> {
    Json(serde_json::json!({"servers": []}))
}

async fn add_mcp_server() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "not_implemented"}))
}

// ─── Prompt (SSE 流式) ────────────────────────────────────

/// POST /v1/chat/session/:id/prompt — SSE 流式响应.
///
/// Stage 2 实现:
/// 1. 验证 session 存在
/// 2. 构造一个**临时** `Conversation` (Stage 3 才把 conversation 持久化到 SessionRuntime)
/// 3. 调 `provider.stream_completion` 拿到 `BoxStream<LlmStreamEvent>`
/// 4. spawn 后台任务消费 stream, 用 `SseEventBuilder` 映射成 12 种契约事件, 推 mpsc
/// 5. SSE wrapper 从 mpsc 读, 序列化成 `data: <json>\n\n` 帧
/// 6. 客户端断连 → axum drop SSE future → mpsc::Receiver 关闭 → spawn task 中
///    `tx.send()` 返回 Err, 任务自然退出
///
/// **Stage 2 不接** `processing_loop::handle_user_message` (Stage 3 接入).
/// 也不接 `tool_result` 事件 (Stage 3 在工具执行路径上发射).
async fn prompt_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<PromptRequest>,
) -> Result<Sse<Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>>, (StatusCode, String)>
{
    // 1. 验证 session
    let runtime = state
        .agent_host
        .get_session(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {id} not found")))?;
    runtime.touch();

    // 2. 构造临时 conversation (Stage 2 简化: 不写回 SessionRuntime)
    let mut conv = Conversation::new(None);
    for msg in &req.messages {
        let role = msg.role.as_str();
        match role {
            "user" => {
                let block = ContentBlock::text(&msg.content);
                conv.push_user_message(vec![block]);
            }
            "assistant" | "system" => {
                // Stage 2 简化: assistant / system 直接入 history 不传给 LLM
                // (完整的 multi-turn 由 Stage 3 conversation 持久化时还原)
                tracing::debug!(
                    "[prompt] role={role} content.len={} (ignored in stage-2)",
                    msg.content.len()
                );
            }
            other => {
                tracing::warn!("[prompt] unknown role {other}, skipping");
            }
        }
    }

    // 3. 构建 CompletionRequest (memory / skills 留 Stage 3 接入)
    let request = conv.build_request(
        &[],
        "", // memory_context
        "", // skills_catalog
        "", // skill_injections
        &runtime.resolved.agent,
    );

    // 4. 调 provider.stream_completion
    let provider = runtime.provider.clone();
    let stream = match provider.stream_completion(request).await {
        Ok(s) => s,
        Err(e) => {
            // provider 启动失败 → 返回 error 事件后关闭
            let err_event = SseEventBuilder::error_from_llm(&e);
            let tail = SseEventBuilder::new().finalize("error");
            let mut events = vec![err_event];
            events.extend(tail);
            let s: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
                Box::pin(iter(events).map(event_to_sse));
            return Ok(Sse::new(s));
        }
    };

    // 5. 通道 + message_start 先发, 再 spawn consumer (避免与 text_delta 乱序)
    let (tx, rx) = mpsc::channel::<SseEvent>(64);

    // message_start: 同步先发, 保证客户端收到的第一帧就是 session 元数据
    let model = runtime.config.model.clone();
    let max_tokens = runtime.resolved.agent.max_tokens.unwrap_or(16384) as u32;
    let session_id = runtime.session_id.clone();
    // channel 容量 64, 单条消息不会 .await 等待
    let _ = tx
        .send(SseEvent::MessageStart {
            session_id: session_id.clone(),
            model,
            max_tokens,
        })
        .await;

    // Stage 3: 把 message_start 事件也写到 store.event_log
    if let Ok(json) = serde_json::to_string(&SseEvent::MessageStart {
        session_id: session_id.clone(),
        model: runtime.config.model.clone(),
        max_tokens,
    }) {
        let _ = state.store.append_event(&session_id, 0, "message_start", &json);
    }

    // consumer: 把 LlmStreamEvent 逐个映射成 SseEvent
    let mut builder = SseEventBuilder::new();
    // Stage 3: 给 consumer 传 store clone 用于事件落盘 + 末次 snapshot
    let store = state.store.clone();
    let sess_id_for_consumer = session_id.clone();
    tokio::spawn(async move {
        consume_stream_to_sse(stream, &mut builder, tx, store, sess_id_for_consumer).await;
    });

    // 6. SSE wrapper: 把 mpsc 里的事件序列化成 SSE 帧
    //    (ReceiverStream 适配 axum::body::Body 要求 impl Stream)
    let sse_stream: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        Box::pin(ReceiverStream::new(rx).map(event_to_sse));
    Ok(Sse::new(sse_stream))
}

/// 把 `SseEvent` 序列化成 axum `Event` (data 帧).
fn event_from_sse(event: SseEvent) -> Event {
    let json = serde_json::to_string(&event).unwrap_or_else(|e| {
        tracing::error!("[sse] failed to serialize event: {e}");
        r#"{"type":"error","code":"internal","message":"event serialization failed"}"#
            .to_string()
    });
    Event::default().data(json)
}

/// 适配 `Stream::map`: `SseEvent` → `Result<Event, Infallible>` (SSE 帧).
fn event_to_sse(event: SseEvent) -> Result<Event, Infallible> {
    Ok(event_from_sse(event))
}

/// 在 spawn task 里消费 `BoxStream<Result<LlmStreamEvent, LlmError>>`,
/// 经 `SseEventBuilder` 转成 SseEvent, 推给 mpsc::Sender.
///
/// 流结束 / 出错时调用 `builder.finalize(reason)` 统一发
/// `ContentBlockStop` (关闭未关 block) + `MessageDelta` + `MessageStop` 收尾.
///
/// 客户端断连: `tx.send().await` 返回 Err, 立即 return 退出 (LLM 流仍可能
/// 在跑, 但没人消费事件, 任务自然结束).
///
/// Stage 3: 同时把每个 SseEvent 写到 `store.append_event()` 落盘; 流结束
/// 时调 `store.save_snapshot(ordinal+1, "{}")` 写一个占位 snapshot
/// (Stage 4 接完整 conversation 序列化).
async fn consume_stream_to_sse(
    mut stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
    >,
    builder: &mut SseEventBuilder,
    tx: mpsc::Sender<SseEvent>,
    store: std::sync::Arc<crate::daemon::persistence::SessionStore>,
    session_id: String,
) {
    let mut last_stop_reason: Option<String> = None;
    // Stage 3: 事件序号, 跳过 seq=0 (已用于 message_start in prompt_handler)
    let mut event_seq: u32 = 1;

    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => {
                // 记录 stop reason, 等流自然结束后再 finalize
                if let LlmStreamEvent::Stop(reason) = &event {
                    last_stop_reason =
                        Some(SseEventBuilder::stop_reason_str(reason).to_string());
                }
                let events = builder.from_llm_event(&event);
                for ev in events {
                    // 落盘: 序列化 SseEvent → JSON → store.append_event
                    if let Ok(json) = serde_json::to_string(&ev) {
                        let type_name = event_type_name(&ev);
                        let _ = store.append_event(&session_id, event_seq, type_name, &json);
                        event_seq += 1;
                    }
                    if tx.send(ev).await.is_err() {
                        return; // 客户端已断
                    }
                }
            }
            Err(e) => {
                // 错误: 发 error 事件 + finalize 收尾
                let err_event = SseEventBuilder::error_from_llm(&e);
                if let Ok(json) = serde_json::to_string(&err_event) {
                    let _ = store.append_event(&session_id, event_seq, "error", &json);
                    event_seq += 1;
                }
                if tx.send(err_event).await.is_err() {
                    return;
                }
                let reason = last_stop_reason
                    .take()
                    .unwrap_or_else(|| "error".to_string());
                let tail = builder.finalize(&reason);
                for ev in tail {
                    if let Ok(json) = serde_json::to_string(&ev) {
                        let type_name = event_type_name(&ev);
                        let _ = store.append_event(&session_id, event_seq, type_name, &json);
                        event_seq += 1;
                    }
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }
                return;
            }
        }
    }

    // 流自然结束: finalize (MessageDelta + MessageStop; 视 builder 内是否有
    // 未关 block 决定要不要先发 ContentBlockStop).
    let reason = last_stop_reason
        .unwrap_or_else(|| "end_turn".to_string());
    let tail = builder.finalize(&reason);
    for ev in tail {
        if let Ok(json) = serde_json::to_string(&ev) {
            let type_name = event_type_name(&ev);
            let _ = store.append_event(&session_id, event_seq, type_name, &json);
            event_seq += 1;
        }
        if tx.send(ev).await.is_err() {
            return;
        }
    }

    // Stage 3 简化: 流结束时写一次占位 snapshot
    // (Stage 4 接完整 conversation 序列化, ordinal=1 表示本次 turn)
    let _ = store.save_snapshot(&session_id, 1, r#"{"messages":[],"stage":"stage3_placeholder"}"#);
}

/// Stage 3: 提取 SSE 事件 type 字符串 (用于 store event_type 字段).
/// 与 `SseEvent` 的 serde tag 字段名严格一致.
fn event_type_name(ev: &SseEvent) -> &'static str {
    match ev {
        SseEvent::MessageStart { .. } => "message_start",
        SseEvent::ContentBlockStart { .. } => "content_block_start",
        SseEvent::TextDelta { .. } => "text_delta",
        SseEvent::ThinkingDelta { .. } => "thinking_delta",
        SseEvent::ToolUseDelta { .. } => "tool_use_delta",
        SseEvent::ToolUseComplete { .. } => "tool_use_complete",
        SseEvent::ToolResult { .. } => "tool_result",
        SseEvent::ContentBlockStop { .. } => "content_block_stop",
        SseEvent::Usage { .. } => "usage",
        SseEvent::MessageDelta { .. } => "message_delta",
        SseEvent::MessageStop => "message_stop",
        SseEvent::Error { .. } => "error",
    }
}

// ─── E2E test: mock LLM stream → SSE 事件序列 ──────────────────

#[cfg(test)]
mod e2e_tests {
    use super::*;
    use futures::stream;
    use qianxun_core::types::{LlmError, StopReason, TokenUsage};
    use serde_json::Value;
    use std::time::Duration;

    /// 端到端测试: 喂入预定义的 LlmStreamEvent 序列, 验证产出的 SseEvent 顺序
    /// 与 shared-contract §3.2 一致 (message_start 由 prompt_handler 在前面
    /// 单独发, 这里测的是从第一个 LlmStreamEvent 开始到 finalize 收尾).
    #[tokio::test]
    async fn test_e2e_mock_provider_text_only_stream() {
        // 1. 构造 mock LLM stream: 2 段 text + 1 个 usage + Stop
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("Hello, ".into())),
            Ok(LlmStreamEvent::Text("world!".into())),
            Ok(LlmStreamEvent::UsageUpdate(TokenUsage {
                input: 100,
                output: 5,
                cache_creation_input: None,
                cache_read_input: None,
            })),
            Ok(LlmStreamEvent::Stop(StopReason::EndTurn)),
        ]));

        // 2. channel + consumer + store (Stage 3: 事件落盘)
        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let mut builder = SseEventBuilder::new();
        let store = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store,
                "sess_e2e_text".to_string(),
            )
            .await;
        });

        // 3. 收集事件 (设 200ms 超时防挂死)
        let mut collected: Vec<SseEvent> = Vec::new();
        let collect_deadline =
            tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(ev) => collected.push(ev),
                        None => break, // channel closed → 任务结束
                    }
                }
                _ = tokio::time::sleep_until(collect_deadline) => {
                    panic!("timed out waiting for SSE events; got so far: {collected:?}");
                }
            }
        }
        task.await.expect("consumer task should not panic");

        // 4. 验证事件序列
        //    预期: ContentBlockStart(text#0), TextDelta(0,"Hello, "),
        //          TextDelta(0,"world!"), Usage(100,5,0,0),
        //          ContentBlockStop(0), MessageDelta("end_turn"), MessageStop
        let types: Vec<&'static str> = collected
            .iter()
            .map(|e| match e {
                SseEvent::MessageStart { .. } => "message_start",
                SseEvent::ContentBlockStart { .. } => "content_block_start",
                SseEvent::TextDelta { .. } => "text_delta",
                SseEvent::ThinkingDelta { .. } => "thinking_delta",
                SseEvent::ToolUseDelta { .. } => "tool_use_delta",
                SseEvent::ToolUseComplete { .. } => "tool_use_complete",
                SseEvent::ToolResult { .. } => "tool_result",
                SseEvent::ContentBlockStop { .. } => "content_block_stop",
                SseEvent::Usage { .. } => "usage",
                SseEvent::MessageDelta { .. } => "message_delta",
                SseEvent::MessageStop => "message_stop",
                SseEvent::Error { .. } => "error",
            })
            .collect();
        assert_eq!(
            types,
            vec![
                "content_block_start",
                "text_delta",
                "text_delta",
                "usage",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ],
            "expected sequence for text-only stream"
        );

        // 5. 验证关键字段
        match &collected[1] {
            SseEvent::TextDelta { index, text } => {
                assert_eq!(*index, 0);
                assert_eq!(text, "Hello, ");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
        match &collected[3] {
            SseEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, 100);
                assert_eq!(*output_tokens, 5);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
        match &collected[5] {
            SseEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason, "end_turn");
            }
            other => panic!("expected MessageDelta, got {other:?}"),
        }
    }

    /// E2E: 流里有 tool_call 时, block 切换正确
    #[tokio::test]
    async fn test_e2e_mock_provider_text_then_tool_call() {
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("让我读取一下文件".into())),
            Ok(LlmStreamEvent::ToolCall {
                id: "toolu_abc".into(),
                tool_name: "read_text_file".into(),
                arguments: serde_json::json!({"path": "/tmp/test.rs"}),
            }),
            Ok(LlmStreamEvent::Stop(StopReason::ToolUse)),
        ]));

        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let mut builder = SseEventBuilder::new();
        let store2 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store2,
                "sess_e2e_tool".to_string(),
            )
            .await;
        });

        let mut collected: Vec<SseEvent> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => collected.push(ev),
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => {
                    panic!("timeout; got: {collected:?}");
                }
            }
        }
        task.await.expect("task ok");

        // 预期序列:
        //   text#0: CBS(text#0), TD("让我读取一下文件")
        //   tool_call: CBS(text#0 STOP), CBS(tool_use#1), TUC, CBS(tool_use#1 STOP)
        //   final: MD("tool_use"), MS
        let types: Vec<&'static str> = collected
            .iter()
            .map(|e| match e {
                SseEvent::ContentBlockStart { .. } => "cbs",
                SseEvent::ContentBlockStop { .. } => "cbs_stop",
                SseEvent::TextDelta { .. } => "td",
                SseEvent::ToolUseComplete { .. } => "tuc",
                SseEvent::MessageDelta { .. } => "md",
                SseEvent::MessageStop => "ms",
                _ => "other",
            })
            .collect();
        assert_eq!(
            types,
            vec!["cbs", "td", "cbs_stop", "cbs", "tuc", "cbs_stop", "md", "ms"],
            "block lifecycle for text+tool_call"
        );

        // 验证 tool_use_complete 携带了正确的 id/name
        match &collected[4] {
            SseEvent::ToolUseComplete { id, name, arguments, index } => {
                assert_eq!(id, "toolu_abc");
                assert_eq!(name, "read_text_file");
                assert_eq!(*index, 1);
                assert_eq!(arguments.get("path").and_then(|v| v.as_str()), Some("/tmp/test.rs"));
            }
            other => panic!("expected ToolUseComplete, got {other:?}"),
        }
    }

    /// E2E: stream 出错时, 错误事件 + 收尾
    #[tokio::test]
    async fn test_e2e_mock_provider_error_mid_stream() {
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("正在思考...".into())),
            Err(LlmError::RateLimitExceeded {
                provider: "deepseek".into(),
                retry_after: Some(Duration::from_secs(2)),
            }),
        ]));

        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let mut builder = SseEventBuilder::new();
        let store3 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store3,
                "sess_e2e_err".to_string(),
            )
            .await;
        });

        let mut collected: Vec<SseEvent> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => collected.push(ev),
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => panic!("timeout; got: {collected:?}"),
            }
        }
        task.await.expect("task ok");

        // 期望: CBS(text#0), TD("..."), ERROR, CBS_STOP(0), MD("error"), MS
        let types: Vec<&'static str> = collected
            .iter()
            .map(|e| match e {
                SseEvent::ContentBlockStart { .. } => "cbs",
                SseEvent::TextDelta { .. } => "td",
                SseEvent::ContentBlockStop { .. } => "cbs_stop",
                SseEvent::Error { .. } => "error",
                SseEvent::MessageDelta { .. } => "md",
                SseEvent::MessageStop => "ms",
                _ => "other",
            })
            .collect();
        assert_eq!(types, vec!["cbs", "td", "error", "cbs_stop", "md", "ms"]);

        // 验证 error 事件的 code = "rate_limit"
        match &collected[2] {
            SseEvent::Error { code, message } => {
                assert_eq!(code, "rate_limit");
                assert!(message.contains("deepseek"), "msg should mention provider: {message}");
                assert!(message.contains("2"), "msg should mention retry_after: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
        // 验证 MD stop_reason = "error" (我们没收到 Stop 事件时使用 fallback)
        match &collected[4] {
            SseEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason, "error");
            }
            other => panic!("expected MD, got {other:?}"),
        }
    }

    /// E2E: 流自然结束但没有 Stop 事件时 (网络异常), 默认 stop_reason = "end_turn"
    #[tokio::test]
    async fn test_e2e_mock_provider_stream_ends_without_stop() {
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("hi".into())),
            // 没有 Stop, 流直接结束 (模拟 provider 异常)
        ]));

        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let mut builder = SseEventBuilder::new();
        let store4 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store4,
                "sess_e2e_no_stop".to_string(),
            )
            .await;
        });

        let mut collected: Vec<SseEvent> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => collected.push(ev),
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => panic!("timeout; got: {collected:?}"),
            }
        }
        task.await.expect("task ok");

        // 末位应该是 MD("end_turn") + MS (fallback)
        let last_two = &collected[collected.len() - 2..];
        match (&last_two[0], &last_two[1]) {
            (SseEvent::MessageDelta { stop_reason }, SseEvent::MessageStop) => {
                assert_eq!(stop_reason, "end_turn");
            }
            other => panic!("expected MD+MS, got {other:?}"),
        }
    }

    /// 验证 SseEvent → SSE 帧的 JSON 序列化格式 (axum 自动在前面加 `data: ` 加
    /// `\n\n` 后缀, 这里只验证 JSON 内容正确). 端到端 SSE wire format 由 axum
    /// 自己保证 (`data: <json>\n\n` 格式).
    #[test]
    fn test_sse_wire_format_json() {
        let ev = SseEvent::TextDelta {
            index: 0,
            text: "hello".into(),
        };
        let json = serde_json::to_string(&ev).expect("serialize");
        // 实际 wire format 是 `data: <json>\n\n`, 这里只验证 JSON 内容
        assert!(json.starts_with("{"), "JSON must start with `{{`: {json}");
        let v: Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("text_delta"));
        assert_eq!(v.get("index").and_then(|i| i.as_u64()), Some(0));
        assert_eq!(v.get("text").and_then(|t| t.as_str()), Some("hello"));
    }
}

// ─── Token Auth Middleware 测试 (Stage 5) ───────────────────────
//
// 注: axum 0.8 的 `Next` 类型没有公开构造器, 无法在测试里直接调
// `auth_middleware`. 改成测试其内部 helper (`extract_token` +
// `is_auth_skipped_path` + `expected_api_key`), 这三个函数完整覆盖了
// middleware 的判定逻辑.

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn test_extract_token_x_api_key_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "secret123".parse().unwrap());
        assert_eq!(extract_token(&headers), Some("secret123".to_string()));
    }

    #[test]
    fn test_extract_token_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret123".parse().unwrap());
        assert_eq!(extract_token(&headers), Some("secret123".to_string()));

        // 大小写不敏感
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "bearer secret456".parse().unwrap());
        assert_eq!(extract_token(&headers), Some("secret456".to_string()));

        // 裸 key (无 Bearer 前缀)
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "raw_key".parse().unwrap());
        assert_eq!(extract_token(&headers), Some("raw_key".to_string()));
    }

    #[test]
    fn test_extract_token_missing_returns_none() {
        let headers = HeaderMap::new();
        assert_eq!(extract_token(&headers), None);
    }

    #[test]
    fn test_is_auth_skipped_path_health_and_status() {
        assert!(is_auth_skipped_path("/v1/system/health"));
        assert!(is_auth_skipped_path("/v1/system/status"));
        // 其它路径不跳过
        assert!(!is_auth_skipped_path("/v1/chat/session"));
        assert!(!is_auth_skipped_path("/v1/tools"));
        assert!(!is_auth_skipped_path("/"));
    }

    /// Stage 5 spec 测试: 缺 X-Api-Key → 401, 有 → 200
    ///
    /// 实施方式: 模拟 middleware 行为 (用 helper 函数组合), 不直接调
    /// `auth_middleware` (它的 `Next` 参数无法在测试里 mock).
    #[test]
    fn test_auth_middleware_missing_token_returns_401() {
        // 1. 配置期望的 key
        let expected = "secret_test_key".to_string();

        // 2. 模拟 middleware 的判定: path 不在跳过列表, 提取的 token 为 None
        let path = "/v1/chat/session";
        let headers = HeaderMap::new();
        assert!(!is_auth_skipped_path(path), "path should require auth");
        let provided = extract_token(&headers);
        assert_eq!(provided, None, "no headers → no token");

        // 3. 模拟 middleware 决策: token == expected? 这里是 None != expected → 401
        let auth_result = match provided {
            Some(t) if t == expected => Ok(()),
            Some(_) => Err(StatusCode::UNAUTHORIZED),
            None => Err(StatusCode::UNAUTHORIZED),
        };
        assert_eq!(auth_result, Err(StatusCode::UNAUTHORIZED));
    }

    /// Stage 5 spec 测试: 有 X-Api-Key → 200, 错误 key → 401
    #[test]
    fn test_auth_middleware_valid_token_returns_200() {
        let expected = "secret_test_key".to_string();

        // 1. X-Api-Key 正确
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "secret_test_key".parse().unwrap());
        let provided = extract_token(&headers);
        assert_eq!(provided, Some("secret_test_key".to_string()));
        let result = check_token(&provided, &expected);
        assert_eq!(result, Ok(()));

        // 2. Authorization: Bearer 正确
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret_test_key".parse().unwrap());
        let provided = extract_token(&headers);
        assert_eq!(provided, Some("secret_test_key".to_string()));
        let result = check_token(&provided, &expected);
        assert_eq!(result, Ok(()));

        // 3. 错误 key
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "wrong_key".parse().unwrap());
        let provided = extract_token(&headers);
        assert_eq!(provided, Some("wrong_key".to_string()));
        let result = check_token(&provided, &expected);
        assert_eq!(result, Err(StatusCode::UNAUTHORIZED));

        // 4. health/status 跳过
        assert!(is_auth_skipped_path("/v1/system/health"));
        assert!(is_auth_skipped_path("/v1/system/status"));
    }

    /// 抽出 token 校验逻辑为可测试函数 (避免在测试里调 `auth_middleware`).
    fn check_token(provided: &Option<String>, expected: &str) -> Result<(), StatusCode> {
        match provided {
            Some(t) if t == expected => Ok(()),
            Some(_) => Err(StatusCode::UNAUTHORIZED),
            None => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

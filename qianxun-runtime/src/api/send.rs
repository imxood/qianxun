// qianxun-runtime/src/api/send.rs
// send_message — 推 user 消息 + 起后台 agent loop + 返 SSE 事件流.
//
// 业务逻辑 1:1 搬自 `qianxun/src/runtime/router.rs::prompt_handler` (Stage 2/3/4 完整版).
// 区别:
//   - 不再返 axum Sse, 返 mpsc::Receiver<SseEvent>
//   - HTTP layer 把 receiver 包成 Sse (BoxStream -> axum::body::Body)
//   - Tauri layer 把 receiver 通过 spawned task 包成 `emit("session_event", event)`
//   - 不需要 AppState (RuntimeState 已含全部依赖)

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{processing_loop, AgentLoop};
use qianxun_core::agent::message::{ContentBlock, Message};
use qianxun_core::context::MemoryObserver;
use qianxun_core::tools::ToolCategoryFilter;
use tokio::sync::mpsc;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{SendRequest, SendResponse};
use crate::output_sink::DaemonOutputSink;
use crate::sse::SseEvent;
use crate::RuntimeState;

/// send_message 业务实现 (供 trait + 单测共用).
///
/// 步骤 (跟 daemon prompt_handler 1:1):
/// 1. 验证 session 存在 (404 if not)
/// 2. 推 user/assistant/system 消息到 conversation
/// 3. 计算 memory_context / skills_catalog / skill_injections
/// 4. 构造 AgentLoop + DaemonOutputSink
/// 5. spawn task 跑 processing_loop::handle_user_message
/// 6. 返 (SendResponse, mpsc::Receiver<SseEvent>)
pub async fn send_message_impl(
    state: Arc<RuntimeState>,
    session_id: &str,
    req: SendRequest,
) -> RuntimeApiResult<(SendResponse, mpsc::Receiver<SseEvent>)> {
    // 1. 验证 session
    let runtime = state
        .agent_host
        .get_session(session_id)
        .ok_or_else(|| RuntimeApiError::NotFound(format!("session {session_id} not found")))?;
    runtime.touch();

    // 2. 推消息到 conversation
    {
        let mut conv_guard = runtime
            .conversation
            .lock()
            .expect("SessionRuntime conversation lock poisoned");
        for msg in &req.messages {
            let role = msg.role.as_str();
            match role {
                "user" => {
                    let block = ContentBlock::text(&msg.content);
                    conv_guard.push_user_message(vec![block]);
                }
                "assistant" | "system" => {
                    let block = ContentBlock::text(&msg.content);
                    conv_guard.push_message(match role {
                        "assistant" => Message::assistant(vec![block]),
                        _ => Message::user(vec![block]),
                    });
                }
                other => {
                    tracing::warn!("[api] send_message: unknown role {other}, skipping");
                }
            }
        }
    }

    // 3. 提取最后一条 user 消息 (作 memory/skills 注入的 query)
    let last_user_msg: String = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // 4. 计算 context 字符串
    let memory_context: String = state.memory.build_context(&last_user_msg, 2000).await;
    let skills_catalog: String = runtime.skills.build_catalog_prompt();
    let matched_skills: Vec<String> = runtime.skills.auto_select(&last_user_msg, &[]);
    let skill_injections: String = runtime.skills.build_injections(&matched_skills);

    // 5. 准备 AgentLoop + Conversation 快照
    let mut agent_loop = AgentLoop::new(runtime.resolved.agent.clone());
    let mut conv: Conversation = runtime
        .conversation
        .lock()
        .expect("SessionRuntime conversation lock poisoned")
        .clone();

    // 6. 通道 + DaemonOutputSink
    let (tx, rx) = mpsc::channel::<SseEvent>(64);
    let model = runtime.config.model.clone();
    let max_tokens = runtime.resolved.agent.max_tokens.unwrap_or(16384) as u32;
    let sink = DaemonOutputSink::new(
        tx,
        state.store.clone(),
        runtime.session_id.clone(),
        model,
        max_tokens,
        true, // emit_message_start
    );

    // 7. spawn 后台 task
    let provider = runtime.provider.clone();
    let tools = runtime.tools.clone();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    tokio::spawn(async move {
        sink.begin_message().await;
        processing_loop::handle_user_message(
            &mut agent_loop,
            &mut conv,
            provider.as_ref(),
            tools.as_ref(),
            ToolCategoryFilter::all(),
            &sink,
            &memory_context,
            &skills_catalog,
            &skill_injections,
            cancel_flag,
        )
        .await;
    });

    Ok((
        SendResponse {
            session_id: runtime.session_id.clone(),
            status: "streaming",
        },
        rx,
    ))
}

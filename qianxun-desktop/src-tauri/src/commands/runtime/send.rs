// qianxun-desktop/src-tauri/src/commands/runtime/send.rs
// Tauri command: send_message
//
// 设计: 跟 daemon HTTP 路径不同, Tauri 不能 streaming 返给前端, 改成:
//   1. command 立即返 SendResponse (含 session_id + "streaming" status)
//   2. spawn task 消费 mpsc::Receiver<SseEvent>, 逐个 emit Tauri event
//   3. 前端监听 "session_event" 事件, 按 session_id 过滤, 拼装到 chat 流
//
// emit schema 跟 events/state_changed.rs 统一 (同 app handle), 见 events/mod.rs.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;

use qianxun_runtime::api::types::{SendRequest, SendResponse};
use qianxun_runtime::api::RuntimeApi;
use qianxun_runtime::RuntimeState;

/// emit payload 包装 — 前端拿 `payload.event` 拿到 SseEvent.
#[derive(Debug, Serialize, Clone)]
pub struct SessionEventPayload {
    pub session_id: String,
    pub event: serde_json::Value,
}

pub const SESSION_EVENT: &str = "session_event";

/// Tauri command: 推 user 消息 + 异步发送 SSE 事件流 (emit 模式).
///
/// 步骤:
/// 1. 拿 AppHandle + session_id 准备 emit
/// 2. 调 RuntimeApi::send_message 拿 SendResponse + Receiver<SseEvent>
/// 3. spawn task 消费 receiver, 逐个 emit SESSION_EVENT
/// 4. 立即返 SendResponse 给前端 (前端不再 polling 状态)
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, Arc<RuntimeState>>,
    session_id: String,
    request: SendRequest,
) -> Result<SendResponse, String> {
    let (resp, rx) = state
        .send_message(&session_id, request)
        .await
        .map_err(|e| e.to_string())?;

    spawn_event_emitter(app, resp.session_id.clone(), rx);
    Ok(resp)
}

/// 消费 Receiver<SseEvent>, 逐个 emit Tauri event. 通道关闭后 task 自然退出.
fn spawn_event_emitter(app: AppHandle, session_id: String, mut rx: mpsc::Receiver<qianxun_runtime::SseEvent>) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let payload = SessionEventPayload {
                session_id: session_id.clone(),
                event: serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
            };
            if let Err(e) = app.emit(SESSION_EVENT, payload) {
                tracing::warn!("[tauri] emit {SESSION_EVENT} failed: {e}");
            }
        }
        tracing::debug!("[tauri] send_message stream closed for {session_id}");
    });
}

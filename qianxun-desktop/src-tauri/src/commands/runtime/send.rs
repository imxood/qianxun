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
    // 2026-06-09 L2: entry log (用户首次能在 stderr 看到 invoke 进来).
    let msg_count = request.messages.len();
    let last_user_chars = request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.len())
        .unwrap_or(0);
    tracing::info!(
        session = %session_id,
        msgs = msg_count,
        user_chars = last_user_chars,
        "[tauri] send_message entry"
    );

    let (resp, rx) = state
        .send_message(&session_id, request)
        .await
        .map_err(|e| {
            // 完整错误 (含 RuntimeApiError::Display 链) 写到 stderr.
            tracing::warn!(
                session = %session_id,
                error = %e,
                "[tauri] send_message rejected"
            );
            e.to_string()
        })?;

    // 2026-06-09 修: 桌面端启动时只同步返骨架, 真 init + restore 在后台跑.
    // 这里 await 一次确保 restore 完, 防止旧 session 找不到 in-memory HashMap.
    state.ensure_restored().await;

    spawn_event_emitter(app, resp.session_id.clone(), rx);
    Ok(resp)
}

/// Tauri command: 推消息到 sub_session (4a-2 P0-2 收尾).
///
/// 当前 (P0) 实现跟 `send_message` 同壳, 调 RuntimeApi::send_message_to_sub_session.
/// 前端 `chatStore.sendToSubSession` 解析 `sub_session_id → parent_session_id` 后传过来,
/// 走 send_message_to_sub_session alias 命中 in-memory runtime.
///
/// 后续 P1 sub_session 持久化缺口接时:
///   1. 后端 send_message_to_sub_session 内部查 sub_session store 拿 parent_session_id
///   2. 前端改成传真 sub_id
///   3. emit payload.session_id 仍用 parent_session_id (跟 in-memory runtime 一致)
#[tauri::command]
pub async fn send_to_sub_session(
    app: AppHandle,
    state: State<'_, Arc<RuntimeState>>,
    sub_session_id: String,
    request: SendRequest,
) -> Result<SendResponse, String> {
    let msg_count = request.messages.len();
    tracing::info!(
        sub_session = %sub_session_id,
        msgs = msg_count,
        "[tauri] send_to_sub_session entry"
    );

    let (resp, rx) = state
        .send_message_to_sub_session(&sub_session_id, request)
        .await
        .map_err(|e| {
            tracing::warn!(
                sub_session = %sub_session_id,
                error = %e,
                "[tauri] send_to_sub_session rejected"
            );
            e.to_string()
        })?;

    state.ensure_restored().await;
    spawn_event_emitter(app, resp.session_id.clone(), rx);
    Ok(resp)
}

/// 消费 Receiver<SseEvent>, 逐个 emit Tauri event. 通道关闭后 task 自然退出.
fn spawn_event_emitter(app: AppHandle, session_id: String, mut rx: mpsc::Receiver<qianxun_runtime::SseEvent>) {
    tauri::async_runtime::spawn(async move {
        let mut event_count = 0usize;
        while let Some(event) = rx.recv().await {
            event_count += 1;
            // 抽样: 每 50 条事件 info 一次 (高频热路径, 全打会刷屏).
            if event_count % 50 == 1 {
                tracing::info!(
                    session = %session_id,
                    events = event_count,
                    "[tauri] streaming progress (每 50 事件抽样)"
                );
            }
            let payload = SessionEventPayload {
                session_id: session_id.clone(),
                event: serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
            };
            if let Err(e) = app.emit(SESSION_EVENT, payload) {
                tracing::warn!(
                    session = %session_id,
                    events = event_count,
                    error = %e,
                    "[tauri] emit {SESSION_EVENT} failed"
                );
            }
        }
        tracing::info!(
            session = %session_id,
            total_events = event_count,
            "[tauri] send_message stream closed"
        );
    });
}

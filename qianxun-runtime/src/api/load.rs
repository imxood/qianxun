// qianxun-runtime/src/api/load.rs
// load_session — 从 store + 内存合并加载 session 完整状态.
//
// 业务:
//   1. 检查内存中 agent_host 有没有 (有 → 返 Active/Paused, 没有 → 查 store 返 Stored)
//   2. 从 store 拉 latest conversation snapshot (Optional, 没 snapshot 也能返)
//   3. 组装 SessionState 返回
//
// 用途: Tauri 端切 session 时拿历史消息; daemon 端 GET /v1/chat/session/{id} 也用同一个 impl.

use std::sync::Arc;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{SessionState, SessionStatus};
use crate::RuntimeState;

/// load_session 业务实现.
pub async fn load_session_impl(
    state: Arc<RuntimeState>,
    session_id: &str,
) -> RuntimeApiResult<SessionState> {
    // 1. 内存中是否存在
    let runtime = state.agent_host.get_session(session_id);
    let (exists_in_memory, status, message_count) = match runtime {
        Some(r) => {
            let is_paused = r.is_paused();
            let count = r
                .conversation
                .lock()
                .map(|c| c.messages().len() as u32)
                .unwrap_or(0);
            let status = if is_paused {
                SessionStatus::Paused
            } else {
                SessionStatus::Active
            };
            (true, status, count)
        }
        None => {
            // 内存中没有, 查 store
            let meta = state.store.list_active().map_err(|e| {
                RuntimeApiError::Internal(format!("store list_active failed: {e}"))
            })?;
            let meta = meta
                .iter()
                .find(|m| m.id == session_id)
                .ok_or_else(|| {
                    RuntimeApiError::NotFound(format!("session {session_id} not found"))
                })?;
            (false, SessionStatus::Stored, meta.message_count)
        }
    };

    // 2. 拉 latest conversation snapshot (Optional)
    let conversation_json = state
        .store
        .load_latest_snapshot(session_id)
        .map_err(|e| RuntimeApiError::Internal(format!("load_latest_snapshot failed: {e}")))?
        .map(|(_ordinal, json)| json);

    Ok(SessionState {
        session_id: session_id.to_string(),
        exists_in_memory,
        status,
        conversation_json,
        message_count,
    })
}

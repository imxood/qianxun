// qianxun-desktop/src-tauri/src/commands/runtime/cancel.rs
// Tauri command: cancel_session
//
// Thin adapter — 收 session_id, 调 RuntimeApi::cancel_session, 返 ().

use std::sync::Arc;

use tauri::State;

use qianxun_runtime::api::RuntimeApi;
use qianxun_runtime::RuntimeState;

/// Tauri command: 取消正在跑的 session (Chat 视图 stop 按钮用).
#[tauri::command]
pub async fn cancel_session(
    state: State<'_, Arc<RuntimeState>>,
    session_id: String,
) -> Result<(), String> {
    state
        .cancel_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

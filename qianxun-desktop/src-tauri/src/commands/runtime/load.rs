// qianxun-desktop/src-tauri/src/commands/runtime/load.rs
// Tauri command: load_session
//
// Thin adapter — 收 session_id, 调 RuntimeApi::load_session, 返 SessionState.
// SessionState 含 conversation snapshot JSON, 跟 Svelte 端 Session entity 1:1.

use std::sync::Arc;

use tauri::State;

use qianxun_runtime::api::types::SessionState;
use qianxun_runtime::api::RuntimeApi;
use qianxun_runtime::RuntimeState;

/// Tauri command: 加载 session 完整状态 (切 session 时调用).
#[tauri::command]
pub async fn load_session(
    state: State<'_, Arc<RuntimeState>>,
    session_id: String,
) -> Result<SessionState, String> {
    state
        .load_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

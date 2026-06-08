// qianxun-desktop/src-tauri/src/commands/runtime/sessions.rs
// Tauri command: list_sessions
//
// Thin adapter — 收 SessionFilter 入参, 调 RuntimeApi::list_sessions, 返 ListSessionsResponse.
// 错误统一成 String 给前端 (Tauri command 必须是 Serialize, RuntimeApiError 没 derive Serialize).

use std::sync::Arc;

use tauri::State;

use qianxun_runtime::api::types::{ListSessionsResponse, SessionFilter};
use qianxun_runtime::RuntimeState;

/// Tauri command: 列所有 session (Svelte 端 session list 视图用).
///
/// 入参: filter ("active" / "paused" / "stored" / "all", 默认 "all")
/// 返: ListSessionsResponse { sessions, total, active_in_memory, paused_in_memory }
#[tauri::command]
pub async fn list_sessions(
    state: State<'_, Arc<RuntimeState>>,
    filter: Option<String>,
) -> Result<ListSessionsResponse, String> {
    let filter = match filter.as_deref() {
        Some("active") => SessionFilter::Active,
        Some("paused") => SessionFilter::Paused,
        Some("stored") => SessionFilter::Stored,
        _ => SessionFilter::All,
    };
    use qianxun_runtime::api::RuntimeApi;
    state
        .list_sessions(filter)
        .await
        .map_err(|e| e.to_string())
}

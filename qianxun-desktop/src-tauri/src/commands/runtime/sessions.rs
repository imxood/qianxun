// qianxun-desktop/src-tauri/src/commands/runtime/sessions.rs
// Tauri command: list_sessions / create_session
//
// Thin adapter — 收 SessionFilter / CreateSessionRequest 入参, 调 RuntimeApi, 返响应.
// 错误统一成 String 给前端 (Tauri command 必须是 Serialize, RuntimeApiError 没 derive Serialize).

use std::sync::Arc;

use tauri::State;

use qianxun_runtime::api::types::{
    CreateSessionRequest, ListSessionsResponse, SessionFilter, SessionInfo, UpdateProviderRequest,
};
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

/// Tauri command: 创建新 session, 后端生成 sess_ 格式 ID, 返 SessionInfo.
///
/// 入参: CreateSessionRequest { model: Option<String>, project_root: Option<String> }
/// 返: SessionInfo { id, model, status, created_at, last_active_at, message_count }
///
/// 前端: NewTaskButton.onNewTask() 调 `invoke('create_session', { request: { project_root } })`,
/// 拿后端真 ID 后 push 到 sessionStore, 后续 send_message 不会 NotFound.
#[tauri::command]
pub async fn create_session(
    state: State<'_, Arc<RuntimeState>>,
    request: CreateSessionRequest,
) -> Result<SessionInfo, String> {
    use qianxun_runtime::api::RuntimeApi;
    state
        .create_session(request)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 删除 session (内存 + 持久化).
///
/// 错误: NotFound.
#[tauri::command]
pub async fn delete_session(
    state: State<'_, Arc<RuntimeState>>,
    session_id: String,
) -> Result<(), String> {
    use qianxun_runtime::api::RuntimeApi;
    state
        .delete_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 暂停 session (拒绝新 send_message, 返 InvalidRequest).
#[tauri::command]
pub async fn pause_session(
    state: State<'_, Arc<RuntimeState>>,
    session_id: String,
) -> Result<(), String> {
    use qianxun_runtime::api::RuntimeApi;
    state
        .pause_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 解除暂停.
#[tauri::command]
pub async fn resume_session(
    state: State<'_, Arc<RuntimeState>>,
    session_id: String,
) -> Result<(), String> {
    use qianxun_runtime::api::RuntimeApi;
    state
        .resume_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 更新 active provider + 可选 provider config (2026-06-09 加).
///
/// 入参: UpdateProviderRequest { active_provider, provider_config: Option<ProviderConfig> }
/// 行为: 写 `~/.qianxun/config.json` (原子). **不热替换** runtime.provider —
/// 调用方需提示用户重启 desktop 才能生效 (见 update_active_provider_impl 注释).
///
/// 错误:
/// - InvalidRequest — active_provider 为空
/// - Internal — 写 config.json 失败
#[tauri::command]
pub async fn update_active_provider(
    state: State<'_, Arc<RuntimeState>>,
    request: UpdateProviderRequest,
) -> Result<(), String> {
    use qianxun_runtime::api::RuntimeApi;
    state
        .update_active_provider(request)
        .await
        .map_err(|e| e.to_string())
}


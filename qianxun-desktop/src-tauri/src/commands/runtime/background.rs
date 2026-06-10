// qianxun-desktop/src-tauri/src/commands/runtime/background.rs
// Tauri commands: 后台异步任务 (缺口 05) — 4 个 thin adapter.
//
// 4 个 command (1:1 对应 RuntimeApi 5 个方法的子集, 跟 Stage 5.4 端点对齐):
//   - start_background_task   → RuntimeApi::start_background_task
//   - list_background_tasks   → RuntimeApi::list_background_tasks
//   - cancel_background_task  → RuntimeApi::cancel_background_task
//   - resume_background_task  → RuntimeApi::resume_background_task
//
// 业务 100% 在 qianxun-runtime/src/background_task.rs, 本文件是 thin adapter.
// get_background_task 留前端直接调 (轻量, 一次查 1 个 task).

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use qianxun_runtime::api::RuntimeApi;
use qianxun_runtime::background_task::TaskInfo;
use qianxun_runtime::RuntimeState;

/// 序列化 wrapper: 把 RuntimeApiError 变成 String 给前端.
#[derive(Debug, Serialize)]
pub struct BgtCommandError {
    pub message: String,
}

impl From<qianxun_runtime::api::RuntimeApiError> for BgtCommandError {
    fn from(e: qianxun_runtime::api::RuntimeApiError) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

/// Tauri command: 启动后台任务.
#[tauri::command]
pub async fn start_background_task(
    state: State<'_, Arc<RuntimeState>>,
    task_kind: String,
    opts: serde_json::Value,
) -> Result<TaskInfo, String> {
    state
        .start_background_task(task_kind, opts)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 列后台任务.
#[tauri::command]
pub async fn list_background_tasks(
    state: State<'_, Arc<RuntimeState>>,
    filter: Option<String>,
) -> Result<Vec<TaskInfo>, String> {
    use qianxun_runtime::background_task::TaskStatus;
    let parsed_filter = filter.as_deref().and_then(|s| match s {
        "pending" => Some(TaskStatus::Pending),
        "running" => Some(TaskStatus::Running),
        "paused" => Some(TaskStatus::Paused),
        "cancelled" => Some(TaskStatus::Cancelled),
        "done" => Some(TaskStatus::Done),
        _ => None,
    });
    state
        .list_background_tasks(parsed_filter)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 取消后台任务.
#[tauri::command]
pub async fn cancel_background_task(
    state: State<'_, Arc<RuntimeState>>,
    task_id: String,
    reason: Option<String>,
) -> Result<(), String> {
    state
        .cancel_background_task(&task_id, reason.unwrap_or_else(|| "user_cancelled".to_string()))
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 恢复 Paused 任务.
#[tauri::command]
pub async fn resume_background_task(
    state: State<'_, Arc<RuntimeState>>,
    task_id: String,
) -> Result<(), String> {
    state
        .resume_background_task(&task_id)
        .await
        .map_err(|e| e.to_string())
}

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

use tauri::State;

use qianxun_runtime::api::RuntimeApi;
use qianxun_runtime::background_task::TaskInfo;
use qianxun_runtime::RuntimeState;

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
    // 2026-06-12 (批次 2.6): filter 字符串解析改 fail-closed 穷举, 不再 `_ => None` 兜底.
    // 之前: "error" / "all" / 任何无效字符串都被静默吞成 "全部", 排查困惑; 同时如果后端
    // TaskStatus 加新变体会拿不到. 改穷举 match, 错就返 Err 提示 (规范 10 命名准确).
    let parsed_filter = match filter.as_deref() {
        None => None,
        Some("pending") => Some(TaskStatus::Pending),
        Some("running") => Some(TaskStatus::Running),
        Some("paused") => Some(TaskStatus::Paused),
        Some("cancelled") => Some(TaskStatus::Cancelled),
        Some("done") => Some(TaskStatus::Done),
        Some(other) => {
            return Err(format!(
                "list_background_tasks: unknown filter '{other}', valid: pending/running/paused/cancelled/done"
            ));
        }
    };
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

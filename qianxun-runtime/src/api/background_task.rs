// qianxun-runtime/src/api/background_task.rs
// 缺口 05: 后台异步任务 5 个 RuntimeApi 方法的 impl 委托.
//
// 业务逻辑全部在 `qianxun-runtime/src/background_task.rs` (BackgroundTaskManager),
// 本文件只是把 BackgroundTaskManager 的方法包装成 RuntimeApi trait 签名.
//
// 5 个方法 1:1 委托:
//   - start_background_task  → mgr.start(kind, opts)
//   - get_background_task    → mgr.get(task_id)
//   - cancel_background_task → mgr.cancel(task_id, reason)
//   - resume_background_task → mgr.resume(task_id)
//   - list_background_tasks  → mgr.list(filter)

use std::sync::Arc;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::background_task::{BgtError, TaskInfo, TaskKind, TaskStatus};
use crate::state::RuntimeState;

/// 启动后台任务.
pub async fn start_background_task_impl(
    state: Arc<RuntimeState>,
    task_kind: String,
    opts: serde_json::Value,
) -> RuntimeApiResult<TaskInfo> {
    let kind = parse_task_kind(&task_kind);
    let info = state.background_tasks.start(kind, opts).await;
    Ok(info)
}

/// 拿任务详情.
pub async fn get_background_task_impl(
    state: Arc<RuntimeState>,
    task_id: &str,
) -> RuntimeApiResult<TaskInfo> {
    state
        .background_tasks
        .get(task_id)
        .await
        .ok_or_else(|| RuntimeApiError::NotFound(format!("task {task_id} not found")))
}

/// 取消任务.
pub async fn cancel_background_task_impl(
    state: Arc<RuntimeState>,
    task_id: &str,
    reason: String,
) -> RuntimeApiResult<()> {
    state
        .background_tasks
        .cancel(task_id, &reason)
        .await
        .map_err(map_bgt_err)
}

/// 恢复 Paused 任务.
pub async fn resume_background_task_impl(
    state: Arc<RuntimeState>,
    task_id: &str,
) -> RuntimeApiResult<()> {
    state
        .background_tasks
        .resume(task_id)
        .await
        .map_err(map_bgt_err)
}

/// 列任务.
pub async fn list_background_tasks_impl(
    state: Arc<RuntimeState>,
    filter: Option<TaskStatus>,
) -> RuntimeApiResult<Vec<TaskInfo>> {
    Ok(state.background_tasks.list(filter).await)
}

// ─── helpers ───────────────────────────────────────────────

/// 解析 task_kind 字符串 → TaskKind enum.
fn parse_task_kind(s: &str) -> TaskKind {
    match s {
        "index_build" => TaskKind::IndexBuild,
        "memory_flush" => TaskKind::MemoryFlush,
        "skill_reload" => TaskKind::SkillReload,
        "long_prompt" => TaskKind::LongPrompt,
        other => TaskKind::Custom(other.to_string()),
    }
}

/// BgtError → RuntimeApiError 转换.
fn map_bgt_err(e: BgtError) -> RuntimeApiError {
    match e {
        BgtError::NotFound => RuntimeApiError::NotFound("task not found".into()),
        BgtError::AlreadyTerminal(s) => {
            RuntimeApiError::Conflict(format!("task already in terminal state: {s:?}"))
        }
        BgtError::InvalidStateTransition { from, to } => {
            RuntimeApiError::InvalidRequest(format!(
                "invalid state transition: {from:?} → {to:?}"
            ))
        }
    }
}

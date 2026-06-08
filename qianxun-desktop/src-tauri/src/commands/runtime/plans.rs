// qianxun-desktop/src-tauri/src/commands/runtime/plans.rs
// Tauri command: create_plan / cancel_plan
//
// Thin adapter — 收 PlanInput, 调 RuntimeApi::create_plan, 返 PlanInfo.
// 业务在 qianxun-runtime/api/plans.rs (in-memory store + 后台 task 顺序执行).
//
// Phase D 收尾: 加 `cancel_plan` Tauri command (后端 RuntimeApi 5→6 方法).

use std::sync::Arc;

use tauri::State;

use qianxun_runtime::api::types::{PlanInfo, PlanInput};
use qianxun_runtime::api::RuntimeApi;
use qianxun_runtime::RuntimeState;

/// Tauri command: 在指定 session 上创建 plan (Kanban 用).
#[tauri::command]
pub async fn create_plan(
    state: State<'_, Arc<RuntimeState>>,
    input: PlanInput,
) -> Result<PlanInfo, String> {
    state.create_plan(input).await.map_err(|e| e.to_string())
}

/// Tauri command: 取消 plan (把 status 置 Aborted).
///
/// Phase D 收尾: 后端 RuntimeApi 加 `cancel_plan` (之前用 cancel_session 走
/// session 路径间接取消 plan, 现在有 plan 级别的取消接口).
#[tauri::command]
pub async fn cancel_plan(
    state: State<'_, Arc<RuntimeState>>,
    plan_id: String,
) -> Result<(), String> {
    state.cancel_plan(&plan_id).await.map_err(|e| e.to_string())
}

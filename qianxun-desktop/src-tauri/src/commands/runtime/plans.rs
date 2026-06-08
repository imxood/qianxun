// qianxun-desktop/src-tauri/src/commands/runtime/plans.rs
// Tauri command: create_plan
//
// Thin adapter — 收 PlanInput, 调 RuntimeApi::create_plan, 返 PlanInfo.
// 业务在 qianxun-runtime/api/plans.rs (in-memory store).

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

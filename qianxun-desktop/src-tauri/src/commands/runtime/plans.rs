// qianxun-desktop/src-tauri/src/commands/runtime/plans.rs
// Tauri command: create_plan / cancel_plan / list_plans / subscribe_plan_events
//
// Thin adapter — 收 PlanInput, 调 RuntimeApi::create_plan, 返 PlanInfo.
// 业务在 qianxun-runtime/api/plans.rs (SQLite store + 后台 task 顺序执行).
//
// Phase D 收尾: 加 `cancel_plan` Tauri command (后端 RuntimeApi 5→6 方法).
// P1-3 收尾 (2026-06-12): 加 `subscribe_plan_events` 订阅 broadcast bus,
// 把 SseEvent::PlanUpdate 透传给前端 (走 `app.emit("plan_event", ...)`).

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use tauri::async_runtime::spawn;

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

/// Tauri command: 列所有 plan (Settings 面板用).
///
/// 2026-06-12 4a-2 P0 收尾: 后端 RuntimeApi 一直有 `list_plans`, 但 Tauri 端
/// 漏接, 前端无法拉取. 现在补 command + invoke 包装.
#[tauri::command]
pub async fn list_plans(
    state: State<'_, Arc<RuntimeState>>,
) -> Result<Vec<PlanInfo>, String> {
    state.list_plans().await.map_err(|e| e.to_string())
}

/// Tauri command: 订阅 plan 事件 (P1-3 收尾, 2026-06-12).
///
/// 启动后端长连接: 拿 `state.subscribe_plan_events()` 返回的 `broadcast::Receiver`,
/// spawn 一个后台 task 持续 `rx.recv()` 收 `SseEvent::PlanUpdate`, 然后
/// `app.emit("plan_event", payload)` 透传给前端.
///
/// 调用方: 前端 store 在启动时 invoke 一次即可. 生命周期跟 app 同步 (关窗即停).
/// 不传 plan_id = 订阅所有 plan 事件 (Tauri 端没必要按 plan_id 过滤, 事件稀少).
#[tauri::command]
pub async fn subscribe_plan_events(
    app: AppHandle,
    state: State<'_, Arc<RuntimeState>>,
) -> Result<(), String> {
    let mut rx = state.subscribe_plan_events();
    spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    // SseEvent 已经 #[derive(Serialize)], 直接 emit (前端收到完整 JSON).
                    if let Err(e) = app.emit("plan_event", &event) {
                        tracing::warn!("[plan] emit plan_event failed: {e}");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // 订阅者消费慢导致 buffer 覆盖, warn 不中断.
                    tracing::warn!("[plan] plan event receiver lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!("[plan] plan event channel closed, stopping subscriber");
                    break;
                }
            }
        }
    });
    Ok(())
}

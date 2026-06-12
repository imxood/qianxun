// qianxun-desktop/src-tauri/src/commands/runtime/sub_sessions.rs
// Tauri command: sub_session 4 个 RuntimeApi 方法 + 1 个事件订阅.
//
// Thin adapter — 跟 plans 1:1 模式: 收 SubSessionInput, 调 RuntimeApi, 返 SubSessionInfo.
// 业务在 qianxun-runtime/api/sub_sessions.rs (SQLite store + SseEvent emit).
//
// 2026-06-12 收尾: E2E Round 1 反馈 "plan 列表点击打开子会话无法打开" 根因 — 后端
// 没建 sub_session 实体, 前端 byPlan() 永远 []. 本次补: execute_one_task 启动
// 时建 sub_session, 完成 / 失败时更新 + emit SubSessionUpdate, 前端 init
// listSubSessions 拉全量 + onSubSessionEvent 增量.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use tauri::async_runtime::spawn;

use qianxun_runtime::api::types::{SubSessionInfo, SubSessionInput, SubSessionStatus};
use qianxun_runtime::api::RuntimeApi;
use qianxun_runtime::RuntimeState;

/// Tauri command: 列 sub_session. plan_id=None 列所有 (init 用),
/// plan_id=Some 列某 plan 下所有 (前端 PlanBlock.byPlan 等价).
#[tauri::command]
pub async fn list_sub_sessions(
    state: State<'_, Arc<RuntimeState>>,
    plan_id: Option<String>,
) -> Result<Vec<SubSessionInfo>, String> {
    state
        .list_sub_sessions(plan_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 按 id 拿单个 sub_session (前端点击"打开子会话"调).
#[tauri::command]
pub async fn get_sub_session(
    state: State<'_, Arc<RuntimeState>>,
    sub_session_id: String,
) -> Result<SubSessionInfo, String> {
    state
        .get_sub_session(&sub_session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 内部建 sub_session (execute_one_task 内部走 RuntimeApi,
/// 不经 Tauri; 但 Tauri 端留入口以便 integration test / 调试).
#[tauri::command]
pub async fn create_sub_session(
    state: State<'_, Arc<RuntimeState>>,
    input: SubSessionInput,
) -> Result<(), String> {
    state
        .create_sub_session(input)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 内部更新 sub_session (task 完成 / 失败).
#[tauri::command]
pub async fn update_sub_session(
    state: State<'_, Arc<RuntimeState>>,
    sub_session_id: String,
    status: SubSessionStatus,
    output: Option<String>,
) -> Result<(), String> {
    state
        .update_sub_session(&sub_session_id, status, output.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: 订阅 sub_session 事件 (execute_one_task 启动 / 完成 / 失败时
/// emit SseEvent::SubSessionUpdate).
///
/// 模式跟 subscribe_plan_events 1:1 — 拿 broadcast::Receiver, spawn 后台 task
/// 持续 recv, 然后 `app.emit("sub_session_event", payload)` 透传给前端.
#[tauri::command]
pub async fn subscribe_sub_session_events(
    app: AppHandle,
    state: State<'_, Arc<RuntimeState>>,
) -> Result<(), String> {
    let mut rx = state.subscribe_sub_session_events();
    spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = app.emit("sub_session_event", &event) {
                        tracing::warn!("[sub_session] emit sub_session_event failed: {e}");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[sub_session] sub_session event receiver lagged, skipped {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!("[sub_session] sub_session event channel closed, stopping subscriber");
                    break;
                }
            }
        }
    });
    Ok(())
}

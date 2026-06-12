// Stage 2 mock: 本地 health, 不走网络, 直接返回 connected.
//
// 2026-06-12 收尾 (Phase B.1): 注入 RuntimeState, session_count 走 list_sessions 真实值.

use std::sync::Arc;

use tauri::State;

use qianxun_runtime::RuntimeState;

use super::types::{DaemonState, HealthStatus};

/// Tauri command: 报告 desktop 端 health 状态.
///
/// 2026-06-12 之前: hardcoded session_count=0 / mcp_online=0 (mock 阶段残留).
/// 现在: 从 RuntimeState.list_sessions 拿真实 session 数, UI 显示与 sidebar 列表一致.
///
/// 注: async + `State<'_>` 注入要求返回 `Result<_, _>` (Tauri 2.x 约束).
/// list_sessions 失败时回退 0 (UI 仍显示 connected, 数字偏差可接受).
#[tauri::command]
pub async fn health_check(
    state: State<'_, Arc<RuntimeState>>,
) -> Result<HealthStatus, String> {
    use qianxun_runtime::api::RuntimeApi;
    let session_count = state
        .list_sessions(qianxun_runtime::api::types::SessionFilter::All)
        .await
        .map(|r| r.sessions.len() as u32)
        .unwrap_or(0);
    Ok(HealthStatus {
        status: DaemonState::Connected,
        version: format!("desktop-{}", env!("CARGO_PKG_VERSION")),
        uptime_sec: 0, // Tauri 端无启动时间跟踪, 留 0
        session_count,
        mcp_online: 0, // 暂未接 MCP 状态, 留 0
        provider_status: serde_json::json!({}),
    })
}

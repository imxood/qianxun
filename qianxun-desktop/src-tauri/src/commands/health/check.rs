// Stage 2 mock: 本地 health, 不走网络, 直接返回 connected.

use super::types::{DaemonState, HealthStatus};

#[tauri::command]
pub async fn health_check() -> HealthStatus {
    HealthStatus {
        status: DaemonState::Connected,
        version: format!("desktop-stage2-{}", env!("CARGO_PKG_VERSION")),
        uptime_sec: 0,
        session_count: 0,
        mcp_online: 0,
        provider_status: serde_json::json!({}),
    }
}
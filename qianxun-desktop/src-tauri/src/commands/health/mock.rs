// offline / connected mock helper (网络错误 / 解析失败降级用)
// Stage 2 最小集, 真状态机留 4a 后续

use super::types::{DaemonState, HealthStatus};

pub fn offline_status() -> HealthStatus {
    HealthStatus {
        status: DaemonState::Offline,
        version: "unknown".to_string(),
        uptime_sec: 0,
        session_count: 0,
        mcp_online: 0,
        provider_status: serde_json::json!({}),
    }
}
// DaemonState (4 态) + HealthStatus (7 字段)
// 与 qianxun-desktop/src/lib/types/ipc.ts 完全对齐
// 跟 docs/30_子项目规划/03-tauri-desktop.md §4.1.2 / §10.1 完全统一.

use serde::{Deserialize, Serialize};

/// Daemon 健康状态 (4 态).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DaemonState {
    Offline,
    Reconnecting,
    Degraded,
    Connected,
}

/// 与 `qianxun-desktop/src/lib/types/ipc.ts` `HealthStatus` 字段一一对应.
/// provider_status 简化为 `serde_json::Value` 以匹配 TS 端的 `Record<string, ...>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: DaemonState,
    pub version: String,
    pub uptime_sec: u64,
    pub session_count: u32,
    pub mcp_online: u32,
    pub provider_status: serde_json::Value,
}
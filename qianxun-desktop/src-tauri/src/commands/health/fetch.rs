// 真实 fetch: GET {url}/v1/system/health (与 _shared-contract.md §3.1 REST 一致).
// 3s 超时. 失败时返回 `offline` 状态, 不抛异常 — 让前端能根据 status 字段继续走降级 UI (§10).

use std::time::Duration;

use super::mock::offline_status;
use super::types::{DaemonState, HealthStatus};

#[tauri::command]
pub async fn daemon_health_fetch(url: String) -> Result<HealthStatus, String> {
    let endpoint = format!("{}/v1/system/health", url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let response = match client.get(&endpoint).send().await {
        Ok(r) => r,
        Err(e) => {
            // 网络层错误 → 返回 offline (前端继续显示降级 UI)
            tracing::warn!(error = %e, endpoint = %endpoint, "daemon_health_fetch: network error");
            return Ok(offline_status());
        }
    };

    if !response.status().is_success() {
        tracing::warn!(
            status = %response.status(),
            endpoint = %endpoint,
            "daemon_health_fetch: non-2xx response"
        );
        return Ok(offline_status());
    }

    // 尝试解析后端 HealthStatus 格式; 解析失败也降级为 offline
    let raw: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("invalid JSON from {endpoint}: {e}"))?;

    let status_str = raw
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("offline");
    let state = match status_str {
        "connected" | "reconnecting" | "degraded" | "offline" => {
            serde_json::from_value(serde_json::Value::String(status_str.to_string()))
                .unwrap_or(DaemonState::Offline)
        }
        // 后端旧版可能返回 "ok" / "starting" / "down", 统一映射
        "ok" | "running" => DaemonState::Connected,
        "starting" => DaemonState::Reconnecting,
        "down" => DaemonState::Degraded,
        _ => DaemonState::Offline,
    };

    Ok(HealthStatus {
        status: state,
        version: raw
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        uptime_sec: raw
            .get("uptime_sec")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        session_count: raw
            .get("session_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        mcp_online: raw
            .get("mcp_online")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        provider_status: raw
            .get("provider_status")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    })
}
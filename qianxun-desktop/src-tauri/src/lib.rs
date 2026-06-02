// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — Stage 2: Tauri 2.0 集成 + 真实 IPC 桥接骨架
//
// 范围 (Stage 2 最小集):
//   - 2 个 invoke command:
//       1. health_check()             — 本地 mock, 不走网络
//       2. daemon_health_fetch(url)   — 真实 fetch GET {url}/v1/system/health
//   - 1 个 emit event:
//       daemon://state-changed        — setup() 阶段立即发 'connected' (Stage 3 接真实状态机)
//   - 直接复用 qianxun-core (path dep), 避免重复实现核心类型
//
// 不做 (留给后续 Stage):
//   - SSE 消费 (§8, Stage 3)
//   - Team / Project / Session 真实 command (§7.1, Stage 3+)
//   - SQLite 缓存 / keyring / app://* (后续)
// ───────────────────────────────────────────────────────────────────────────

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

// 显式 use 一次 qianxun-core, 证明 path dep 接进来了 (后续 Stage 会扩展使用).
#[allow(unused_imports)]
use qianxun_core as _qianxun_core;

// ─── 数据模型 (与 qianxun-desktop/src/lib/types/ipc.ts §4.1.2 完全对齐) ──

/// Daemon 健康状态 (4 态).
/// 与 docs/30_子项目规划/03-tauri-desktop.md §4.1.2 / §10.1 完全统一.
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

// ─── Tauri commands ─────────────────────────────────────────────────────

/// Stage 2 mock: 本地 health, 不走网络, 直接返回 connected.
#[tauri::command]
async fn health_check() -> HealthStatus {
    HealthStatus {
        status: DaemonState::Connected,
        version: format!("desktop-stage2-{}", env!("CARGO_PKG_VERSION")),
        uptime_sec: 0,
        session_count: 0,
        mcp_online: 0,
        provider_status: serde_json::json!({}),
    }
}

/// 真实 fetch: GET {url}/v1/system/health (与 _shared-contract.md §3.1 REST 一致).
/// 3s 超时. 失败时返回 `offline` 状态, 不抛异常 — 让前端能根据 status 字段继续走降级 UI (§10).
#[tauri::command]
async fn daemon_health_fetch(url: String) -> Result<HealthStatus, String> {
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

fn offline_status() -> HealthStatus {
    HealthStatus {
        status: DaemonState::Offline,
        version: "unknown".to_string(),
        uptime_sec: 0,
        session_count: 0,
        mcp_online: 0,
        provider_status: serde_json::json!({}),
    }
}

// ─── App entry ──────────────────────────────────────────────────────────

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_http::init())
        .setup(|app| {
            // Stage 2: setup 阶段立即发一次 'connected', 让前端能验证 IPC 桥接通.
            // 真实 health check + 状态机留 Stage 3 (与 daemon-stage2-sse-stream 对齐).
            let handle: AppHandle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // 等 UI 加载完再 emit, 避免 listener 还没注册
                tokio::time::sleep(Duration::from_millis(500)).await;
                emit_state_changed(&handle, "connected");
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![health_check, daemon_health_fetch])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn emit_state_changed<R: Runtime>(app: &AppHandle<R>, state: &str) {
    if let Err(e) = app.emit("daemon://state-changed", state) {
        tracing::warn!(error = %e, "failed to emit daemon://state-changed");
    }
}

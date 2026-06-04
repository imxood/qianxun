
// ─── Daemon 探测 (默认 127.0.0.1:23900, 3s 超时) ─────────────

use std::time::Duration;

use tracing::debug;

use super::daemon_client::DaemonClient;

/// 探测本地 daemon 是否在运行. 成功返回 `Some(base_url)`, 失败返回 `None`.
///
/// Stage 4 简化:
/// - 优先 `QIANXUN_DAEMON_URL` env var
/// - 回退 `http://127.0.0.1:23900` (默认 daemon 端口)
/// - 3s 超时
pub async fn detect_local_daemon() -> Option<String> {
    let base_url = std::env::var("QIANXUN_DAEMON_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:23900".to_string());
    let client = DaemonClient::new(base_url.clone());
    match tokio::time::timeout(Duration::from_secs(3), client.health()).await {
        Ok(Ok(h)) if h.status == "ok" => {
            debug!("[client] daemon detected at {base_url}");
            Some(base_url)
        }
        Ok(Ok(h)) => {
            debug!("[client] daemon health non-ok: {h:?}");
            None
        }
        Ok(Err(e)) => {
            debug!("[client] daemon probe error: {e}");
            None
        }
        Err(_) => {
            debug!("[client] daemon probe timeout (>3s)");
            None
        }
    }
}

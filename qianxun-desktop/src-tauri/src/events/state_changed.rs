// daemon://state-changed — setup 阶段立即发 'connected' (Stage 2 mock)
// 4a 后续接真状态机, 等连接 / 重连 / 断开时 emit 真实状态.

use tauri::{AppHandle, Emitter, Runtime};

/// 事件名常量, 跨后端/前端共享 (前端 `lib/stores/connection.svelte.ts` listen).
pub const STATE_CHANGED_EVENT: &str = "daemon://state-changed";

/// Emit daemon health state 变更事件.
/// payload: `"connected"` | `"offline"` | `"reconnecting"` | `"degraded"`
pub fn emit<R: Runtime>(app: &AppHandle<R>, state: &str) {
    if let Err(e) = app.emit(STATE_CHANGED_EVENT, state) {
        tracing::warn!(error = %e, "failed to emit {STATE_CHANGED_EVENT}");
    }
}
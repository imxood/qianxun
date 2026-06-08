// qianxun-runtime/src/api/cancel.rs
// cancel_session — 软取消正在跑的 session.
//
// 业务 1:1 搬自 `qianxun/src/runtime/router.rs::cancel_session`:
//   - agent_host.cancel_session() 设置 paused flag
//   - 不存在 → NotFound
//   - 已 paused 也接受 (幂等)
//
// Stage 7b 简化: 软信号 (paused flag) 替代 tokio CancellationToken.
// 后续 Stage 接完整 CancellationToken 时, impl 替换 cancel_flag 的来源, 业务 0 改动.

use std::sync::Arc;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::RuntimeState;

/// cancel_session 业务实现.
pub async fn cancel_session_impl(
    state: Arc<RuntimeState>,
    session_id: &str,
) -> RuntimeApiResult<()> {
    state
        .agent_host
        .cancel_session(session_id)
        .await
        .map_err(RuntimeApiError::NotFound)?;
    tracing::info!("[api] cancel_session: id={session_id}");
    Ok(())
}

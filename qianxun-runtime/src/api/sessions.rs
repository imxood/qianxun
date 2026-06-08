// qianxun-runtime/src/api/sessions.rs
// list_sessions — 合并 store metadata + 内存 agent_host 状态.
//
// 业务逻辑 1:1 搬自 `qianxun/src/runtime/router.rs::list_sessions` (Stage 7b).
// 区别: 不再返 Json<serde_json::Value>, 返结构化 ListSessionsResponse.
// HTTP layer (daemon router) 跟 Tauri layer (command) 都用同一个返回类型.

use std::sync::Arc;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{
    ListSessionsResponse, SessionFilter, SessionInfo, SessionStatus,
};
use crate::RuntimeState;

/// list_sessions 业务实现 (供 trait + 单测共用).
///
/// 入参: filter (Active / Paused / Stored / All)
/// 返回: sessions 数组 + 内存中 active/paused 计数
/// 错误: store.list_active 失败返 Internal
pub async fn list_sessions_impl(
    state: Arc<RuntimeState>,
    filter: SessionFilter,
) -> RuntimeApiResult<ListSessionsResponse> {
    // 1. 拉 store 元数据 (内存 SQLite, 不阻塞)
    let metas = state.store.list_active().map_err(|e| {
        tracing::error!("[api] list_sessions: store.list_active failed: {e}");
        RuntimeApiError::Internal(format!("store error: {e}"))
    })?;

    // 2. 内存计数
    let active_in_mem = state.agent_host.session_count();
    let paused_in_mem = state.agent_host.paused_count();

    // 3. 合并 + 过滤
    let mut sessions = Vec::with_capacity(metas.len());
    for meta in metas {
        let runtime = state.agent_host.get_session(&meta.id);
        let is_paused = runtime.as_ref().is_some_and(|r| r.is_paused());
        let in_memory = runtime.is_some();

        // filter
        let include = match filter {
            SessionFilter::Active => in_memory && !is_paused,
            SessionFilter::Paused => in_memory && is_paused,
            SessionFilter::Stored => !in_memory,
            SessionFilter::All => true,
        };
        if !include {
            continue;
        }

        let model = runtime
            .as_ref()
            .map(|r| r.config.model.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let status = if !in_memory {
            SessionStatus::Stored
        } else if is_paused {
            SessionStatus::Paused
        } else {
            SessionStatus::Active
        };

        sessions.push(SessionInfo {
            id: meta.id,
            model,
            status,
            created_at: meta.created_at,
            last_active_at: meta.last_active_at,
            message_count: meta.message_count,
        });
    }

    Ok(ListSessionsResponse {
        total: sessions.len(),
        sessions,
        filter: filter_label(&filter),
        active_in_memory: active_in_mem,
        paused_in_memory: paused_in_mem,
    })
}

/// SessionFilter → "active" / "paused" / "stored" / "all" 字符串 (回显给前端用).
fn filter_label(filter: &SessionFilter) -> String {
    match filter {
        SessionFilter::Active => "active".to_string(),
        SessionFilter::Paused => "paused".to_string(),
        SessionFilter::Stored => "stored".to_string(),
        SessionFilter::All => "all".to_string(),
    }
}

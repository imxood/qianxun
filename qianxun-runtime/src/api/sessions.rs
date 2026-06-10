// qianxun-runtime/src/api/sessions.rs
// list_sessions — 合并 store metadata + 内存 agent_host 状态.
//
// 业务逻辑 1:1 搬自 `qianxun/src/runtime/router.rs::list_sessions` (Stage 7b).
// 区别: 不再返 Json<serde_json::Value>, 返结构化 ListSessionsResponse.
// HTTP layer (daemon router) 跟 Tauri layer (command) 都用同一个返回类型.

use std::sync::Arc;

use crate::agent_host::CreateSessionOpts;
use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{
    CreateSessionRequest, ListSessionsResponse, SessionFilter, SessionInfo, SessionStatus,
    UpdateProviderRequest,
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
    // 2026-06-09 修: 不在 list_sessions_impl 调 ensure_restored!
    // 原因: restore_from_disk 内部用 store 数据**新建** SessionRuntime 覆盖 agent_host 已有
    // 实例, 会丢失 cancel_session_impl 设的 paused=true 状态. 客户端调 cancel 后立刻 list_sessions
    // 验 paused 就会失败. 改成: ensure_restored 只在 send_message 入口调 (Tauri command 层),
    // 那时 cancel 还没发生. list_sessions 走 in-memory 当前状态, 不重新 load.
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
            project_root: meta.project_root,
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

/// create_session 业务实现 (供 trait + 单测共用).
///
/// 1. 构造 `CreateSessionOpts` (从 `CreateSessionRequest.project_root` 透传)
/// 2. 调 `state.agent_host.create_session(opts)` (后端生成 session_id, 持久化)
/// 3. 构造 `SessionInfo` 返前端
///
/// 错误:
/// - `RuntimeApiError::Internal` — store.create 失败 / agent_host panic
/// - `RuntimeApiError::Unavailable` — max_sessions 满
pub async fn create_session_impl(
    state: Arc<RuntimeState>,
    req: CreateSessionRequest,
) -> RuntimeApiResult<SessionInfo> {
    let opts = CreateSessionOpts {
        project_root: req.project_root,
        model: req.model,
    };

    let runtime = state.agent_host.create_session(opts).map_err(|e| {
        if e.contains("Max sessions reached") {
            RuntimeApiError::Unavailable(e)
        } else {
            tracing::error!("[api] create_session: agent_host.create_session failed: {e}");
            RuntimeApiError::Internal(e)
        }
    })?;

    // 构造 SessionInfo 返前端
    Ok(SessionInfo {
        id: runtime.session_id.clone(),
        model: runtime.config.model.clone(),
        status: SessionStatus::Active,
        created_at: chrono::Utc::now().to_rfc3339(),
        last_active_at: chrono::Utc::now().to_rfc3339(),
        message_count: 0,
        project_root: runtime.project_root.clone(),
    })
}

/// delete_session 业务实现. 同步删除内存 + 持久化记录 (FK CASCADE).
pub async fn delete_session_impl(
    state: Arc<RuntimeState>,
    session_id: &str,
) -> RuntimeApiResult<()> {
    if !state.agent_host.delete_session(session_id) {
        return Err(RuntimeApiError::NotFound(format!(
            "session {session_id} not found"
        )));
    }
    tracing::info!("[api] delete_session: id={session_id}");
    Ok(())
}

/// pause_session 业务实现. 同步设置 paused flag.
pub async fn pause_session_impl(
    state: Arc<RuntimeState>,
    session_id: &str,
) -> RuntimeApiResult<()> {
    state
        .agent_host
        .pause_session(session_id)
        .map_err(|e| {
            if e.contains("not found") {
                RuntimeApiError::NotFound(e)
            } else if e.contains("already paused") {
                RuntimeApiError::Conflict(e)
            } else {
                RuntimeApiError::Internal(e)
            }
        })?;
    tracing::info!("[api] pause_session: id={session_id}");
    Ok(())
}

/// resume_session 业务实现. 同步清除 paused flag.
pub async fn resume_session_impl(
    state: Arc<RuntimeState>,
    session_id: &str,
) -> RuntimeApiResult<()> {
    state
        .agent_host
        .resume_session(session_id)
        .map_err(|e| {
            if e.contains("not found") {
                RuntimeApiError::NotFound(e)
            } else if e.contains("is not paused") {
                RuntimeApiError::Conflict(e)
            } else {
                RuntimeApiError::Internal(e)
            }
        })?;
    tracing::info!("[api] resume_session: id={session_id}");
    Ok(())
}

/// update_active_provider 业务实现 (2026-06-09 加).
///
/// 步骤:
/// 1. 校验 active_provider 名字合法 (非空 + ASCII)
/// 2. 读 `~/.qianxun/config.json` 旧内容
/// 3. 改 active_provider 字段 (+ 可选 provider_config 改对应 entry)
/// 4. 写回 config.json (原子, 通过 Config::save_to_file)
///
/// 错误:
/// - InvalidRequest — active_provider 为空
/// - Internal — 读 / 写 config.json 失败
pub async fn update_active_provider_impl(
    state: Arc<RuntimeState>,
    req: UpdateProviderRequest,
) -> RuntimeApiResult<()> {
    use qianxun_core::config::Config;

    // 1. 校验
    if req.active_provider.trim().is_empty() {
        return Err(RuntimeApiError::InvalidRequest(
            "active_provider must not be empty".to_string(),
        ));
    }
    let name = req.active_provider.trim().to_string();

    // 2. 算 config.json 路径 (跟 state/runtime.rs::try_load_config 一致)
    let path = qianxun_core::workspace::qianxun_dir()
        .map(|d| d.join("config.json"))
        .ok_or_else(|| {
            RuntimeApiError::Internal("cannot determine ~/.qianxun home dir".to_string())
        })?;

    // 3. 读旧 config
    let mut config = Config::from_file(&path)
        .map_err(|e| RuntimeApiError::Internal(format!("read config: {e}")))?;

    // 4. 改 active_provider 字段
    config.active_provider = Some(name.clone());

    // 5. 可选: 改 provider config entry
    if let Some(pcfg) = req.provider_config {
        let providers = config.providers.get_or_insert_with(Default::default);
        providers.insert(name.clone(), pcfg);
    }

    // 6. 写回 (原子, 走 Config::save_to_file)
    config
        .save_to_file(&path)
        .map_err(|e| RuntimeApiError::Internal(format!("save config: {e}")))?;

    // 7. 提示 (用 tracing info, 桌面端会从 state 知道 — 但 runtime 不热替换)
    tracing::warn!(
        "[api] update_active_provider: id={} (重启 desktop 生效)",
        name
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_host::CreateSessionOpts;

    /// P1-5 收尾 (2026-06-12): list_sessions(filter=Paused).paused_in_memory
    /// 精确反映 cancel_session 触发的 paused 状态. router.rs::system_metrics
    /// 走这个 API 拿 paused_count, 之前是 hardcoded 0.
    ///
    /// 验证:
    ///   1. 创 2 session (都 active, paused_in_memory=0)
    ///   2. cancel 1 个 (paused_in_memory=1)
    ///   3. list_sessions(filter=Paused) 返 1 个 + paused_in_memory=1
    ///   4. list_sessions(filter=Active) 返 1 个 + paused_in_memory=1 (总数不变)
    #[tokio::test]
    async fn list_sessions_paused_reflects_cancel_state() {
        let state = RuntimeState::new_in_memory_with_config(
            qianxun_core::config::ResolvedConfig::default(),
        )
        .await
        .expect("RuntimeState init");

        // 1. 创 2 session
        let s1 = state
            .agent_host
            .create_session(CreateSessionOpts::default())
            .expect("create s1");
        let s2 = state
            .agent_host
            .create_session(CreateSessionOpts::default())
            .expect("create s2");
        // 初始: 都 active, paused_in_memory=0
        let resp = list_sessions_impl(state.clone(), SessionFilter::All)
            .await
            .expect("list All");
        assert_eq!(resp.paused_in_memory, 0);
        assert_eq!(resp.active_in_memory, 2);

        // 2. cancel s1 (P1-4 收尾: 触发 paused + cancel_flag)
        state
            .agent_host
            .cancel_session(&s1.session_id)
            .await
            .expect("cancel s1");

        // 3. list_sessions(Paused) 返 1 个 + paused_in_memory=1
        let resp = list_sessions_impl(state.clone(), SessionFilter::Paused)
            .await
            .expect("list Paused");
        assert_eq!(resp.paused_in_memory, 1, "P1-5: paused_in_memory 必须反映 cancel 状态");
        assert_eq!(resp.sessions.len(), 1, "filter=Paused 应返 1 个 session");
        assert_eq!(resp.sessions[0].id, s1.session_id);

        // 4. list_sessions(Active) 返 s2 + paused_in_memory=1 (总数不变)
        let resp = list_sessions_impl(state.clone(), SessionFilter::Active)
            .await
            .expect("list Active");
        assert_eq!(resp.paused_in_memory, 1);
        assert_eq!(resp.sessions.len(), 1);
        assert_eq!(resp.sessions[0].id, s2.session_id);
    }
}

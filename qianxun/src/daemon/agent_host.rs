//! AgentLoopHost — 管理多个 session 生命周期 + 共享子系统引用.
//!
//! Stage 1 范围 (见 docs/30_子项目规划/01-daemon.md §4):
//! - `SessionRuntime` 完整构造 (provider/tools/memory/skills 注入)
//! - `create_session` 真的能产出 `Arc<SessionRuntime>`
//! - `delete_session` / `reap_stale` 保留
//! - **不** 启动 processing_loop (Stage 2 SSE 接入)
//! - **不** 持久化 session (Stage 3 接入)
//!
//! Stage 3 范围:
//! - 持有 `Arc<SessionStore>` (持久化层)
//! - `create_session` 末尾调 `store.create()`
//! - `restore_from_disk()` 启动时加载所有 active session 的 conversation
//! - `delete_session` 同步调 `store` 的级联删除 (FK CASCADE 已自动处理)

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::Utc;

use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;

use qianxun_memory::MemoryCore;

use crate::daemon::persistence::SessionStore;
use crate::daemon::session_runtime::{SessionId, SessionRuntime};

/// 共享子系统集合, 由 `AppState` 持有, 注入到 `AgentLoopHost`.
///
/// 在 Stage 1, 这是构造 SessionRuntime 所需全部依赖的**最小集**:
/// - provider: LLM 端点 (来自 ResolvedConfig.active_provider)
/// - tools: builtin + MCP 工具 (Stage 1 用空, builtin register_all 留 Stage 2/3)
/// - memory: SQLite 记忆 (Stage 1 用 in_memory 占位)
/// - skills: 技能目录 (Stage 1 用空 manager, 真实加载留 Stage 2/3)
/// - resolved: AgentConfig / Compaction / Budget 等配置
///
/// `tools` / `memory` / `skills` 设计上每个 session 共享 (Arc),
/// 但 `tools` / `skills` 在 Stage 2/3 接入 builtin 注册 + skill 加载后才有真实内容.
pub struct SharedState {
    pub resolved: Arc<ResolvedConfig>,
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub memory: Arc<MemoryCore>,
    pub skills: SkillManager,
}

impl SharedState {
    /// 构造共享状态 (Stage 1 最小集: 真实 provider + 空 tools/memory/skills).
    pub fn new(
        resolved: ResolvedConfig,
        provider: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        memory: Arc<MemoryCore>,
        skills: SkillManager,
    ) -> Self {
        Self {
            resolved: Arc::new(resolved),
            provider,
            tools,
            memory,
            skills,
        }
    }
}

/// AgentLoop 会话宿主 —— 管理多个会话的生命周期.
///
/// Stage 1 持有 `SharedState` 引用, 每个 session 通过 `create_session` 拿到
/// 自己的一份 `SessionRuntime` (Arc 共享).
pub struct AgentLoopHost {
    sessions: Arc<RwLock<HashMap<SessionId, Arc<SessionRuntime>>>>,
    max_sessions: usize,
    state: Arc<SharedState>,
    /// Stage 3: session 持久化 (3 张 daemon_ 表).
    pub store: Arc<SessionStore>,
}

impl AgentLoopHost {
    pub fn new(
        max_sessions: usize,
        state: Arc<SharedState>,
        store: Arc<SessionStore>,
    ) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            max_sessions,
            state,
            store,
        }
    }

    /// 创建一个新 session, 注入全部共享依赖.
    ///
    /// Stage 3: 末尾调 `self.store.create()` 持久化元数据 + 空 snapshot.
    /// `project_root` 暂时为 None (Stage 2 在 router 层根据请求 body 传入).
    pub fn create_session(
        &self,
    ) -> Result<Arc<SessionRuntime>, String> {
        // 1. 上限检查
        {
            let sessions = self.sessions.read().expect("AgentLoopHost lock poisoned");
            if sessions.len() >= self.max_sessions {
                return Err(format!(
                    "Max sessions reached ({} / {})",
                    sessions.len(),
                    self.max_sessions
                ));
            }
        }

        // 2. 生成 session_id
        let now = Utc::now();
        let session_id = format!("sess_{}", now.format("%Y%m%d_%H%M%S_%6f"));

        // 3. 构造 SessionRuntime (注入共享子系统)
        let runtime = Arc::new(SessionRuntime::new(
            session_id.clone(),
            None, // project_root: Stage 1 暂不传
            self.state.resolved.clone(),
            self.state.provider.clone(),
            self.state.tools.clone(),
            self.state.memory.clone(),
            self.state.skills.clone(),
        ));

        // 4. 二次检查后插入 HashMap
        let mut sessions = self.sessions.write().expect("AgentLoopHost lock poisoned");
        if sessions.len() >= self.max_sessions {
            return Err(format!(
                "Max sessions reached ({} / {})",
                sessions.len(),
                self.max_sessions
            ));
        }
        sessions.insert(session_id.clone(), runtime.clone());
        drop(sessions);

        // 5. Stage 3: 持久化 session 元数据 + 空 snapshot.
        //    config_json: 序列化 ResolvedProviderConfig (model / base_url 等).
        //    注: ResolvedProviderConfig 暂未 derive Serialize (qianxun-core),
        //    这里手动构造 JSON, 后续 Stage 4 给 ResolvedProviderConfig 加 Serialize derive.
        let config_json = serde_json::json!({
            "model": runtime.config.model,
            "base_url": runtime.config.base_url,
            "temperature": runtime.config.temperature,
            "max_tokens": runtime.config.max_tokens,
        })
        .to_string();
        if let Err(e) = self.store.create(&session_id, None, &config_json) {
            tracing::error!("[daemon] session store.create failed: {e}");
            return Err(format!("session persistence failed: {e}"));
        }

        tracing::info!(
            "[daemon] created session {session_id} (total: {})",
            self.session_count()
        );
        Ok(runtime)
    }

    /// 检查会话是否存在.
    pub fn session_exists(&self, id: &str) -> bool {
        self.sessions
            .read()
            .expect("AgentLoopHost lock poisoned")
            .contains_key(id)
    }

    /// 取 session 引用.
    pub fn get_session(&self, id: &str) -> Option<Arc<SessionRuntime>> {
        self.sessions
            .read()
            .expect("AgentLoopHost lock poisoned")
            .get(id)
            .cloned()
    }

    /// 删除会话.
    ///
    /// Stage 3: 也从 SessionStore 删除 (FK CASCADE 自动清理
    /// daemon_conversation_snapshots 和 daemon_event_log).
    pub fn delete_session(&self, id: &str) -> bool {
        let removed = self
            .sessions
            .write()
            .expect("AgentLoopHost lock poisoned")
            .remove(id)
            .is_some();

        // Stage 3: 同步删除持久化记录
        if removed {
            if let Err(e) = self.store.delete(id) {
                tracing::warn!("[daemon] session store.delete failed: {e}");
            }
            tracing::info!("[daemon] deleted session {id}");
        }
        removed
    }

    /// Stage 7b: 取消正在跑的 prompt.
    ///
    /// 简化实现: 设置 `runtime.cancelled` 标志, 现有 SSE 流消费 task 会在下次
    /// `tx.send()` 时返回 Err, 自然退出. Stage 7c 接入完整 AgentLoop 后会
    /// 把 cancel 信号发到 LLM provider (HTTP request abort).
    ///
    /// 错误: session 不存在 → `Err("session not found")`.
    pub async fn cancel_session(&self, id: &str) -> Result<(), String> {
        let runtime = self
            .get_session(id)
            .ok_or_else(|| format!("session {id} not found"))?;
        tracing::info!("[daemon] cancel session {id}");
        // 当前无活跃 stream (Stage 2 prompt_handler spawn 的 task 不可直接
        // 引用). 我们设置 paused = true 作为软信号; Stage 7c 接入完整
        // 取消令牌 (tokio CancellationToken) 后, 这里调 token.cancel().
        runtime.set_paused(true);
        runtime.touch();
        Ok(())
    }

    /// Stage 7b: 暂停 session. 标记后新 prompt 返 409 Conflict.
    ///
    /// 已 paused 时再调返 `Err("already paused")`, 不改变状态.
    /// Stage 7c 实现完整 resume 语义.
    pub fn pause_session(&self, id: &str) -> Result<(), String> {
        let runtime = self
            .get_session(id)
            .ok_or_else(|| format!("session {id} not found"))?;
        if runtime.is_paused() {
            return Err(format!("session {id} already paused"));
        }
        runtime.set_paused(true);
        runtime.touch();
        tracing::info!("[daemon] paused session {id}");
        Ok(())
    }

    /// Stage 7b: 解除暂停 (Stage 7c 完善). 已在 active 时返 Err.
    #[allow(dead_code)] // Stage 7b 简化: 不接 endpoint, 留 7c
    pub fn resume_session(&self, id: &str) -> Result<(), String> {
        let runtime = self
            .get_session(id)
            .ok_or_else(|| format!("session {id} not found"))?;
        if !runtime.is_paused() {
            return Err(format!("session {id} is not paused"));
        }
        runtime.set_paused(false);
        runtime.touch();
        tracing::info!("[daemon] resumed session {id}");
        Ok(())
    }

    /// Stage 7b: 统计 paused session 数. 用于 /v1/system/metrics.
    pub fn paused_count(&self) -> usize {
        self.sessions
            .read()
            .expect("AgentLoopHost lock poisoned")
            .values()
            .filter(|r| r.is_paused())
            .count()
    }

    /// Stage 10b: 优雅关闭所有活跃 session.
    ///
    /// 行为: 遍历 in-memory `sessions` HashMap, 对每个**未 paused**的 runtime
    /// 设 `paused = true` (跟 `cancel_session` 同效, 触发 SSE 流的 stop signal).
    /// 已经 paused 的不动 (避免误覆盖).
    ///
    /// 返回: 实际被 mark 为 cancelled 的 session 数 (含已经 paused 的 — 因为
    /// 这次 mark "cancelled" 仍要 touch 一下, 让 last_active 更新).
    ///
    /// 错误: lock poison 会 panic (跟其他方法一致, 不返 Result).
    pub fn shutdown_all(&self) -> usize {
        let sessions: Vec<Arc<SessionRuntime>> = {
            let map = self.sessions.read().expect("AgentLoopHost lock poisoned");
            map.values().cloned().collect()
        };
        let total = sessions.len();
        let mut cancelled = 0usize;
        for runtime in sessions {
            runtime.set_paused(true);
            runtime.touch();
            cancelled += 1;
        }
        if cancelled > 0 {
            tracing::info!(
                "[daemon] shutdown_all: marked {cancelled}/{total} sessions as paused (cancelled)"
            );
        }
        cancelled
    }

    /// Stage 3: 启动恢复 — 加载所有 active session 的 conversation.
    ///
    /// 对 `store.list_active()` 的每个 session:
    /// 1. 调 `store.load_latest_snapshot()` 拿到 conversation_json + ordinal
    /// 2. 构造 SessionRuntime (不调 AgentLoop, 只装 conversation 状态)
    /// 3. 插入 in-memory HashMap
    ///
    /// 返回成功恢复的 session 数.
    pub async fn restore_from_disk(&self) -> Result<usize, String> {
        // 1. 列出所有 active session
        let metas = self.store.list_active().map_err(|e| e.to_string())?;
        if metas.is_empty() {
            return Ok(0);
        }

        let mut restored = 0;
        for meta in metas {
            // 2. 加载最新 snapshot
            let snap = match self
                .store
                .load_latest_snapshot(&meta.id)
                .map_err(|e| e.to_string())?
            {
                Some(s) => s,
                None => {
                    tracing::warn!(
                        "[daemon] session {id} has no snapshot, skipping",
                        id = meta.id
                    );
                    continue;
                }
            };

            let (ordinal, conversation_json) = snap;
            tracing::info!(
                "[daemon] restoring session {id} from snapshot ordinal={ord}",
                id = meta.id,
                ord = ordinal
            );

            // 3. 构造 SessionRuntime (空 AgentLoop, 还原 conversation).
            //    注意: Stage 3 简化, conversation 字段无法从 JSON 还原
            //    (Conversation 持有 Vec<Message>, 反序列化需要 Message
            //    全字段支持; 后续 Stage 4 接完整 restore). 这里保持空
            //    conversation, 但元数据 + last_active 已恢复, 客户端可
            //    看到 session 列表.
            let runtime = Arc::new(SessionRuntime::new(
                meta.id.clone(),
                meta.project_root.clone(),
                self.state.resolved.clone(),
                self.state.provider.clone(),
                self.state.tools.clone(),
                self.state.memory.clone(),
                self.state.skills.clone(),
            ));

            // 4. 插入 HashMap
            let mut sessions = self.sessions.write().expect("AgentLoopHost lock poisoned");
            if sessions.len() >= self.max_sessions {
                tracing::warn!(
                    "[daemon] max_sessions reached while restoring, dropping {id}",
                    id = meta.id
                );
                break;
            }
            sessions.insert(meta.id.clone(), runtime);

            // 5. conversation_json 留在 store 里, 客户端 connect 后按需 replay
            let _ = conversation_json; // 暂未反序列化 (Stage 4 完整恢复)
            restored += 1;
        }

        Ok(restored)
    }

    /// 当前 session 数.
    pub fn session_count(&self) -> usize {
        self.sessions
            .read()
            .expect("AgentLoopHost lock poisoned")
            .len()
    }

    /// 清理过期会话 (后台任务, 60s tick).
    pub async fn reap_stale(&self) {
        let timeout = Duration::from_secs(3600);
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let now = Utc::now();
            let mut sessions = self.sessions.write().expect("AgentLoopHost lock poisoned");
            let before = sessions.len();
            sessions.retain(|id, runtime| {
                let elapsed = now.signed_duration_since(runtime.last_active()).to_std().unwrap_or(timeout);
                let keep = elapsed < timeout;
                if !keep {
                    tracing::info!("[daemon] reaping stale session {id}");
                }
                keep
            });
            let reaped = before - sessions.len();
            if reaped > 0 {
                tracing::info!("[daemon] reaped {reaped} stale sessions");
            }
        }
    }

    /// 构造测试用 host (不连真实 provider, 用空 in-memory 依赖).
    #[cfg(test)]
    pub fn for_test(max_sessions: usize, resolved: ResolvedConfig) -> Self {
        use qianxun_core::provider::create_provider;
        use qianxun_core::tools::ToolRegistry;
        use qianxun_core::skills::SkillManager;
        use qianxun_memory::MemoryCore;

        let provider: Arc<dyn LlmProvider> = create_provider(
            &resolved.active_provider,
            &resolved.active_provider_config(),
        )
        .into();
        let tools = Arc::new(ToolRegistry::new());
        let memory = Arc::new(
            MemoryCore::open_in_memory().expect("MemoryCore::open_in_memory failed"),
        );
        let skills = SkillManager::new();
        let state = Arc::new(SharedState::new(
            resolved,
            provider,
            tools,
            memory,
            skills,
        ));
        // Stage 3: 测试用 in-memory store
        let store = Arc::new(
            SessionStore::in_memory().expect("SessionStore::in_memory failed"),
        );
        Self::new(max_sessions, state, store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qianxun_core::config::ResolvedConfig;

    #[test]
    fn test_create_session_yields_runtime() {
        let host = AgentLoopHost::for_test(10, ResolvedConfig::default());

        // 创建前 HashMap 为空
        assert_eq!(host.session_count(), 0);
        assert!(!host.session_exists("sess_does_not_exist"));

        // 创建一个 session
        let runtime = host.create_session().expect("create_session should succeed");

        // 字段正确
        assert!(runtime.session_id.starts_with("sess_"));
        assert!(runtime.project_root.is_none()); // Stage 1 简化
        assert!(runtime.config.model.contains("deepseek")
            || runtime.config.model.contains("MiniMax")
            || !runtime.config.model.is_empty());
        assert_eq!(runtime.agent_loop.turn_count, 0);
        assert_eq!(runtime.agent_loop.retry_count, 0);
        assert_eq!(runtime.conversation.messages().len(), 0);

        // HashMap 里现在有 1 个
        assert_eq!(host.session_count(), 1);
        assert!(host.session_exists(&runtime.session_id));
        assert!(host.get_session(&runtime.session_id).is_some());
    }

    #[test]
    fn test_delete_session_removes_runtime() {
        let host = AgentLoopHost::for_test(10, ResolvedConfig::default());

        let runtime = host.create_session().expect("create_session should succeed");
        assert_eq!(host.session_count(), 1);

        let id = runtime.session_id.clone();
        let removed = host.delete_session(&id);
        assert!(removed, "delete_session should return true for existing session");
        assert_eq!(host.session_count(), 0);
        assert!(!host.session_exists(&id));
        assert!(host.get_session(&id).is_none());
    }

    #[test]
    fn test_max_sessions_limit_enforced() {
        let host = AgentLoopHost::for_test(2, ResolvedConfig::default());

        let _r1 = host.create_session().expect("first");
        let _r2 = host.create_session().expect("second");
        let r3 = host.create_session();
        // SessionRuntime 没有 Debug (dyn LlmProvider 没法 derive), 用 match 替代 unwrap_err
        match r3 {
            Err(msg) => assert!(msg.contains("Max sessions reached"), "unexpected error: {msg}"),
            Ok(_) => panic!("third session should be rejected"),
        }
    }
}

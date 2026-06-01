//! AgentLoopHost — 管理多个 session 生命周期 + 共享子系统引用.
//!
//! Stage 1 范围 (见 docs/30_子项目规划/01-daemon.md §4):
//! - `SessionRuntime` 完整构造 (provider/tools/memory/skills 注入)
//! - `create_session` 真的能产出 `Arc<SessionRuntime>`
//! - `delete_session` / `reap_stale` 保留
//! - **不** 启动 processing_loop (Stage 2 SSE 接入)
//! - **不** 持久化 session (Stage 3 接入)

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::Utc;

use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;

use qianxun_memory::MemoryCore;

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
}

impl AgentLoopHost {
    pub fn new(max_sessions: usize, state: Arc<SharedState>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            max_sessions,
            state,
        }
    }

    /// 创建一个新 session, 注入全部共享依赖.
    ///
    /// Stage 1 简化: `project_root` 暂时为 None (Stage 2 在 router 层
    /// 根据请求 body 传入). Session 真实工作目录由 Stage 3 持久化时记录.
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

        tracing::info!(
            "[daemon] created session {session_id} (total: {})",
            sessions.len()
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
    pub fn delete_session(&self, id: &str) -> bool {
        let removed = self
            .sessions
            .write()
            .expect("AgentLoopHost lock poisoned")
            .remove(id)
            .is_some();
        if removed {
            tracing::info!("[daemon] deleted session {id}");
        }
        removed
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
        Self::new(max_sessions, state)
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

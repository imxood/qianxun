//! SessionRuntime — 每个 session 的全部运行时状态。
//!
//! ToolUseDelta/ToolResult variant 留 Phase 4 接 streaming tool call.
#![allow(dead_code)]
//!
//! Stage 1 范围: 仅持有引用, 不启动 AgentLoop. 真正的 processing_loop
//! 调度 (Stage 2 SSE 流式) 在 router 层 spawn task 后调用.
//!
//! 字段遵循 docs/30_子项目规划/01-daemon.md §3.2 / §4.2 契约, 但 Stage 1
//! 实际只需要把核心依赖聚合在一起, 一些锁/取消/状态字段暂用简化版.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::AgentLoop;
use qianxun_core::config::{ResolvedConfig, ResolvedProviderConfig};
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;

use qianxun_memory::MemoryCore;

/// Session 唯一标识 (`sess_YYYYMMDD_HHMMSS_microsec`).
pub type SessionId = String;

/// 单个 session 的运行时状态.
///
/// 所有 AgentLoop 运行所需的依赖都聚合在这里, 由 `AgentLoopHost` 持有.
///
/// # Stage 1 备注
///
/// - `agent_loop` / `conversation` 当前**直接**持有, 因为 Stage 1 不会真的
///   调用 `processing_loop`. Stage 2 接入时, 按设计文档 §4.3 的决策, 会改成
///   `tokio::sync::Mutex<AgentLoop>` / `tokio::sync::Mutex<Conversation>`
///   以支持跨 await 持锁.
/// - `cancel_flag` 暂未启用 (Stage 2 接入 SSE 后才用).
/// - `status` 暂用 `String` 简化版, Stage 2 替换为 `SessionStatus` 枚举.
pub struct SessionRuntime {
    /// Session ID (`sess_...`).
    pub session_id: SessionId,

    /// 创建时从 .qianxun/ 向上查找得到的项目根, 用于工作目录定位.
    pub project_root: Option<String>,

    /// 当前激活 provider 的解析后配置 (api_key/model/base_url/...).
    /// 注意: api_key 在 Stage 1 走明文 (沿用 Phase 3 行为), Stage 4 接入 keyring.
    pub config: ResolvedProviderConfig,

    /// 完整 ResolvedConfig 引用, 便于在 handler 里读 compaction / budget / 等.
    pub resolved: Arc<ResolvedConfig>,

    /// per-session AgentLoop 状态 (turn_count / retry_count / accumulated_usage).
    /// Stage 1 不启动, Stage 2 SSE 调 `processing_loop::handle_user_message` 时用.
    pub agent_loop: AgentLoop,

    /// per-session 会话历史.
    /// Stage 1 留空, Stage 2 每次 prompt 起始时 `push_user_message`.
    pub conversation: Conversation,

    /// 共享 LLM provider (来自 AppState, Arc 引用以避免复制).
    pub provider: Arc<dyn LlmProvider>,

    /// 共享 ToolRegistry (builtin + MCP + skill, 来自 AppState).
    pub tools: Arc<ToolRegistry>,

    /// 共享 MemoryCore (来自 AppState).
    pub memory: Arc<MemoryCore>,

    /// 共享 SkillManager (来自 AppState).
    pub skills: SkillManager,

    /// 创建时间.
    pub created_at: DateTime<Utc>,

    /// 最后活跃时间. `RwLock` 是为 Stage 2 异步更新 last_active 准备.
    pub last_active_at: RwLock<DateTime<Utc>>,

    /// Stage 7b: 暂停标志. `pause_session` 调后切 true, 后续 prompt 拒绝
    /// 接收 (返 409). 完整 resume 语义留给 Stage 7c/8.
    pub paused: AtomicBool,
}

impl SessionRuntime {
    /// 构造新 SessionRuntime.
    ///
    /// 所有 Arc 字段由 `state` 共享注入 (Stage 1 最小集, 见 `AppState`).
    pub fn new(
        session_id: SessionId,
        project_root: Option<String>,
        resolved: Arc<ResolvedConfig>,
        provider: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        memory: Arc<MemoryCore>,
        skills: SkillManager,
    ) -> Self {
        let now = Utc::now();
        // AgentLoop 持有 resolved.agent (AgentConfig) — Stage 2 处理 user message 时用.
        let agent_loop = AgentLoop::new(resolved.agent.clone());
        // Conversation 暂用 None system_prompt; Stage 2 接入 system_prompt.rs.
        // TODO (Stage 2): 用 system_prompt::build_system_prompt 构造.
        let conversation = Conversation::new(None);

        Self {
            session_id,
            project_root,
            config: resolved.active_provider_config(),
            resolved,
            agent_loop,
            conversation,
            provider,
            tools,
            memory,
            skills,
            created_at: now,
            last_active_at: RwLock::new(now),
            paused: AtomicBool::new(false),
        }
    }

    /// 读取最后活跃时间 (拷贝).
    pub fn last_active(&self) -> DateTime<Utc> {
        *self.last_active_at.read().expect("SessionRuntime last_active lock poisoned")
    }

    /// 更新最后活跃时间到 now.
    pub fn touch(&self) {
        let now = Utc::now();
        *self.last_active_at.write().expect("SessionRuntime last_active lock poisoned") = now;
    }

    /// Stage 7b: 检查 session 是否被暂停.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Stage 7b: 切换暂停状态. 返切换后状态 (true = paused).
    pub fn set_paused(&self, paused: bool) -> bool {
        self.paused.store(paused, Ordering::Relaxed);
        paused
    }
}

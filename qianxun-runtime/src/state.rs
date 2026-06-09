// qianxun-runtime/src/state.rs
// RuntimeState — 抽自 qianxun/src/runtime/mod.rs (原 daemon 启动逻辑 1:1)
// 复用给 qianxun binary (daemon / cli / tui / server) 跟 qianxun-desktop (Tauri webview)
// 跟 ADR-0003 (合并 desktop + 2-mode 互斥) 一致
//
// 字段: 9 核心 (provider / config / tools / memory / skills / shared / agent_host / store / shutdown_tx)
// 6 daemon-specific 字段 (admin / llm_providers / active_conns / log_ring / started_at / processing_loop_enabled)
// 留在 qianxun binary 内的 AppState, 嵌入 `Arc<RuntimeState>`

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::{create_provider, LlmProvider};
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;
use qianxun_memory::MemoryCore;

use crate::agent_host::{AgentLoopHost, SharedState};
use crate::api::plans::PlanStore;
use crate::persistence::SessionStore;

/// 千寻运行时核心状态 — 10 核心字段, 跨桌面 / daemon 共享.
///
/// 字段变更 (Stage 4a sub-task #3):
///   - 新增 `plans: Arc<PlanStore>` — in-memory plan store (RuntimeApi plans impl 用)
///   - 之前 9 字段不变
pub struct RuntimeState {
    /// 内部 session 生命周期管理. **pub(crate)** — 外部禁止直接访问, 必须走 RuntimeApi trait.
    /// 内部 6 个字段 (provider / config / tools / memory / skills / shared) 仍 pub,
    /// 因为 RuntimeApi impl + DaemonOutputSink 等内部组件需要读它们.
    pub(crate) agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub memory: Arc<MemoryCore>,
    pub skills: SkillManager,
    pub shared: Arc<SharedState>,
    pub store: Arc<SessionStore>,
    pub plans: Arc<PlanStore>,
    pub shutdown_tx: watch::Sender<()>,
    /// 2026-06-09 加: restore_from_disk 懒初始化标志 (OnceCell 模式).
    /// build() 同步不调 restore_from_disk (它跑 5-15s 同步 SQLite, 阻塞 webview 启动).
    /// 后台 task 调 ensure_restored() 异步跑, 完成后置 true. 首次 send_message / list_sessions
    /// 前若没 restored, 自动调一次.
    restored: Arc<AtomicBool>,
}

impl RuntimeState {
    /// 内部 session 数 (供 graceful shutdown / metrics 用).
    /// 外部不应直接调 — 用 RuntimeApi::list_sessions 返 ListSessionsResponse.active_in_memory.
    pub fn session_count(&self) -> usize {
        self.agent_host.session_count()
    }

    /// 2026-06-09 加: 懒恢复 session (后台首次调, 之后 idempotent).
    ///
    /// 桌面端启动不再 block_on 等 restore, 而是:
    /// 1. build() 同步返骨架 (new_for_test, in-memory, <100ms)
    /// 2. 后台 task 调 ensure_restored() 异步跑, 加载 store 里的 session 到 agent_host
    /// 3. 首次 send_message / list_sessions 前若没 restored, 同步等一次
    ///
    /// 优点: desktop 启动从 5-15s 降到 <1s.
    pub async fn ensure_restored(&self) {
        if self.restored.load(Ordering::Acquire) {
            return;
        }
        match self.agent_host.restore_from_disk().await {
            Ok(n) => {
                tracing::info!("[runtime] ensure_restored: loaded {n} sessions from disk");
                self.restored.store(true, Ordering::Release);
            }
            Err(e) => {
                tracing::warn!("[runtime] ensure_restored failed: {e} (will retry on next call)");
                // 不置 true, 下次 send_message 还会再试
            }
        }
    }

    /// 优雅关闭所有 in-memory session (graceful shutdown 用).
    /// 标记每个未 paused 的 runtime 为 paused, 触发 SSE 流 stop signal.
    pub fn shutdown_all_sessions(&self) -> usize {
        self.agent_host.shutdown_all()
    }

    /// 启动后台 reap_stale 任务 (清理 1 小时未活跃的 session).
    /// RuntimeState::new 自动调用, qianxun binary 不需要手动启动.
    /// 暴露 pub 是给 mod.rs 显式触发的 fallback (Stage 4 之前), 后续可改 private.
    pub fn spawn_reap_stale(&self) {
        let host = self.agent_host.clone();
        tokio::spawn(async move {
            host.reap_stale().await;
        });
    }
    /// 完整初始化: provider / tools / memory / skills / SessionStore / AgentLoopHost
    /// 跟 `qianxun/src/runtime/mod.rs::run()` 初始化逻辑 1:1
    /// (Stage 1 最小集, 真实 provider + builtin tools + 真 SQLite memory/skills)
    pub async fn new(config: ResolvedConfig) -> anyhow::Result<Arc<Self>> {
        let mem_path = qianxun_core::workspace::qianxun_dir()
            .map(|d| d.join("mem.db"))
            .unwrap_or_else(|| PathBuf::from("./mem.db"));
        let store_path = qianxun_core::workspace::qianxun_dir()
            .map(|d| d.join("daemon.db"))
            .ok_or_else(|| anyhow::anyhow!("cannot determine ~/.qianxun home dir"))?;
        Self::build(
            config,
            mem_path.to_str().unwrap_or("./mem.db"),
            store_path.to_str().unwrap_or("./daemon.db"),
        )
        .await
    }

    /// 集成测试用: 真 config + 真 provider, 但 store 走 in_memory 避免污染 ~/.qianxun/daemon.db
    /// memory 也走 in_memory (跟 llm_integration_tests 旧实现 1:1)
    pub async fn new_in_memory_with_config(config: ResolvedConfig) -> anyhow::Result<Arc<Self>> {
        Self::build(config, ":memory:", ":memory:").await
    }

    async fn build(
        config: ResolvedConfig,
        mem_path: &str,
        store_path: &str,
    ) -> anyhow::Result<Arc<Self>> {
        // provider: 来自 config.active_provider
        let provider: Arc<dyn LlmProvider> = create_provider(
            &config.active_provider,
            &config.active_provider_config(),
        )
        .into();

        // tools: 空 registry + register_all_builtin (失败 fallback 空)
        let mut tools = ToolRegistry::new();
        let _ = tools.register_all_builtin();
        let tools = Arc::new(tools);

        // memory: SQLite (mem_path = ":memory:" 走 in_memory)
        let memory = if mem_path == ":memory:" {
            Arc::new(MemoryCore::open_in_memory()?)
        } else {
            MemoryCore::open(PathBuf::from(mem_path))
                .map(Arc::new)
                .unwrap_or_else(|_| Arc::new(MemoryCore::open_in_memory().expect("in_memory fallback")))
        };

        // skills: 加载 (空目录静默 OK)
        let skills = SkillManager::load_all(None);

        // store: SQLite (store_path = ":memory:" 走 in_memory)
        let store = if store_path == ":memory:" {
            Arc::new(SessionStore::in_memory()?)
        } else {
            Arc::new(SessionStore::new(&PathBuf::from(store_path))?)
        };

        // shared: 包共享 provider/tools/memory/skills
        let shared = Arc::new(SharedState::new(
            config.clone(),
            provider.clone(),
            tools.clone(),
            memory.clone(),
            skills.clone(),
        ));

        // agent_host: usize::MAX sessions (无上限, 2026-06-09 修正)
        // session 是持久化状态, 不限制数量. 实际"运行中 LLM 调用"由
        // processing_loop 内的并发控制 (后续 PR 加 Semaphore).
        let agent_host = Arc::new(AgentLoopHost::new(usize::MAX, shared.clone(), store.clone()));

        // 启动恢复: 加载上次未关闭的 session (失败 warn 继续)
        let _ = agent_host.restore_from_disk().await;

        let (shutdown_tx, _) = watch::channel(());

        Ok(Arc::new(Self {
            agent_host,
            config: Arc::new(config),
            provider,
            tools,
            memory,
            skills,
            shared,
            store,
            plans: Arc::new(Mutex::new(std::collections::HashMap::new())),
            shutdown_tx,
            restored: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// 测试用: in-memory provider + in-memory memory + tmp dir store
    /// 跟 daemon `mod.rs::make_test_state()` 1:1
    pub fn new_for_test() -> Arc<Self> {
        let config = ResolvedConfig::default();
        let provider: Arc<dyn LlmProvider> = create_provider(
            &config.active_provider,
            &config.active_provider_config(),
        )
        .into();
        let tools = Arc::new(ToolRegistry::new());
        let memory = Arc::new(MemoryCore::open_in_memory().expect("in-memory mem"));
        let skills = SkillManager::new();
        let tmp = std::env::temp_dir().join(format!(
            "qianxun_runtime_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = Arc::new(SessionStore::new(&tmp).expect("open store"));
        let shared = Arc::new(SharedState::new(
            config.clone(),
            provider.clone(),
            tools.clone(),
            memory.clone(),
            skills.clone(),
        ));
        let agent_host = Arc::new(AgentLoopHost::new(10, shared.clone(), store.clone()));
        let (shutdown_tx, _) = watch::channel(());
        Arc::new(Self {
            agent_host,
            config: Arc::new(config),
            provider,
            tools,
            memory,
            skills,
            shared,
            store,
            plans: Arc::new(Mutex::new(std::collections::HashMap::new())),
            shutdown_tx,
            restored: Arc::new(AtomicBool::new(false)),
        })
    }
}

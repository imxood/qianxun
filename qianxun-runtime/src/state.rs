// qianxun-runtime/src/state.rs
// RuntimeState — 抽自 qianxun/src/runtime/mod.rs (原 daemon 启动逻辑 1:1)
// 复用给 qianxun binary (daemon / cli / tui / server) 跟 qianxun-desktop (Tauri webview)
// 跟 ADR-0003 (合并 desktop + 2-mode 互斥) 一致
//
// 字段: 9 核心 (provider / config / tools / memory / skills / shared / agent_host / store / shutdown_tx)
// 6 daemon-specific 字段 (admin / llm_providers / active_conns / log_ring / started_at / processing_loop_enabled)
// 留在 qianxun binary 内的 AppState, 嵌入 `Arc<RuntimeState>`

use std::path::PathBuf;
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
    pub agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub memory: Arc<MemoryCore>,
    pub skills: SkillManager,
    pub shared: Arc<SharedState>,
    pub store: Arc<SessionStore>,
    pub plans: Arc<PlanStore>,
    pub shutdown_tx: watch::Sender<()>,
}

impl RuntimeState {
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

        // agent_host: 10 sessions 上限
        let agent_host = Arc::new(AgentLoopHost::new(10, shared.clone(), store.clone()));

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
        })
    }
}

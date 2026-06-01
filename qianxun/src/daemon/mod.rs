pub mod agent_host;
pub mod router;
pub mod session_runtime;

use std::sync::Arc;

use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::{create_provider, LlmProvider};
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;
use tokio::sync::watch;

use qianxun_memory::MemoryCore;

use crate::daemon::agent_host::{AgentLoopHost, SharedState};

/// Daemon 共享状态 (Stage 1 最小集).
///
/// 字段是 Stage 1 真正需要的全部共享依赖. 与设计文档 §3.2 描述的
/// 完整 AppState 相比, Stage 1 暂不引入 BudgetManager / LlmProviderPool /
/// SessionStore / VpsWsClient 等模块 (见 README 已知 TODO).
pub struct AppState {
    pub agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    /// 共享 LLM provider 句柄 (Stage 1 = active provider, 单实例).
    pub provider: Arc<dyn LlmProvider>,
    /// 共享 ToolRegistry (Stage 1 = 空 registry, builtin 注册留 Stage 2/3).
    pub tools: Arc<ToolRegistry>,
    /// 共享 MemoryCore (Stage 1 = in_memory SQLite, 真实 db 留 Stage 3).
    pub memory: Arc<MemoryCore>,
    /// 共享 SkillManager (Stage 1 = 空 manager, 真实加载留 Stage 2/3).
    pub skills: SkillManager,
    /// 共享子系统集合 (被 AgentLoopHost 引用以构造 SessionRuntime).
    pub shared: Arc<SharedState>,
    /// 关闭信号.
    pub shutdown_tx: watch::Sender<()>,
}

/// 启动 Daemon HTTP 服务.
///
/// Stage 1 接受外部传入的 `ResolvedConfig` (在 main.rs 中已解析好, 见
/// `Config::from_file` → `resolve` 链路), 构造共享子系统并组装 AppState.
pub async fn run(port: u16, resolved: ResolvedConfig) -> anyhow::Result<()> {
    tracing::info!("Daemon starting on 127.0.0.1:{port}");

    let (shutdown_tx, mut shutdown_rx) = watch::channel(());

    // ── 构造共享子系统 (Stage 1 最小集) ──
    // provider: 来自 resolved.active_provider_config
    let provider: Arc<dyn LlmProvider> = create_provider(
        &resolved.active_provider,
        &resolved.active_provider_config(),
    )
    .into();
    // tools: 空 registry (builtin register_all 留 Stage 2/3)
    let tools = Arc::new(ToolRegistry::new());
    // memory: in_memory SQLite 占位 (真实 ~/.qianxun/mem.db 留 Stage 3)
    let memory = Arc::new(MemoryCore::open_in_memory()?);
    // skills: 空 manager (load_all 留 Stage 2/3)
    let skills = SkillManager::new();

    // 包成 Arc<SharedState>, 让 AgentLoopHost 也能引用同一份
    let shared = Arc::new(SharedState::new(
        resolved.clone(),
        provider.clone(),
        tools.clone(),
        memory.clone(),
        skills.clone(),
    ));
    let agent_host = Arc::new(AgentLoopHost::new(10, shared.clone()));

    let config = Arc::new(resolved);
    let state = Arc::new(AppState {
        agent_host,
        config,
        provider,
        tools,
        memory,
        skills,
        shared,
        shutdown_tx,
    });

    // 启动 reap_stale 后台任务 (Stage 1 暂不 await, 实际不退出)
    let reap_host = state.agent_host.clone();
    tokio::spawn(async move {
        reap_host.reap_stale().await;
    });

    let app = router::build_router(state);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_rx.changed().await.ok();
        })
        .await?;

    tracing::info!("Daemon stopped");
    Ok(())
}

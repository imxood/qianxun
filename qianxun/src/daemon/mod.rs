pub mod agent_host;
pub mod llm_providers;
pub mod persistence;
pub mod router;
pub mod service;
pub mod session_runtime;
pub mod sse;

use std::path::PathBuf;
use std::sync::Arc;

use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::{create_provider, LlmProvider};
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;
use tokio::sync::watch;

use qianxun_memory::MemoryCore;

use crate::daemon::agent_host::{AgentLoopHost, SharedState};
use crate::daemon::llm_providers::LlmProviderManager;
use crate::daemon::persistence::SessionStore;

/// Daemon 共享状态 (Stage 1 最小集).
///
/// 字段是 Stage 1 真正需要的全部共享依赖. 与设计文档 §3.2 描述的
/// 完整 AppState 相比, Stage 1 暂不引入 BudgetManager / LlmProviderPool /
/// SessionStore / VpsWsClient 等模块 (见 README 已知 TODO).
///
/// Stage 2 新增 `processing_loop_enabled` 标记 — Stage 2 暂不接
/// `processing_loop::handle_user_message` 全套, 直接调
/// `provider.stream_completion` 实现 12 个 SSE 事件. Stage 3 接入
/// 完整 processing_loop 后, 此 flag 切到 true 并将 prompt_handler
/// 改为通过 `OutputSink` 桥接.
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
    /// Session 持久化 (Stage 3 新增, 3 张 daemon_ 表).
    pub store: Arc<SessionStore>,
    /// Stage 7a: LLM provider 管理器 (CRUD + test, 走 in-memory cache).
    pub llm_providers: Arc<LlmProviderManager>,
    /// 关闭信号.
    pub shutdown_tx: watch::Sender<()>,
    /// Stage 2 留 false: 直接调 `provider.stream_completion` 走 SSE 流;
    /// Stage 3 切 true: 接入 `processing_loop::handle_user_message` + 工具执行.
    pub processing_loop_enabled: bool,
}

/// 启动 Daemon HTTP 服务.
///
/// Stage 1 接受外部传入的 `ResolvedConfig` (在 main.rs 中已解析好, 见
/// `Config::from_file` → `resolve` 链路), 构造共享子系统并组装 AppState.
///
/// Stage 7a 新增 `ui_dist: Option<PathBuf>` 参数, 控制是否 serve SvelteKit
/// 静态 dist (路径不存在时不 panic, 启动时 warn 即可).
pub async fn run(
    port: u16,
    resolved: ResolvedConfig,
    ui_dist: Option<PathBuf>,
) -> anyhow::Result<()> {
    tracing::info!("Daemon starting on 127.0.0.1:{port}");

    // Stage 7a: Web UI dist 路径决策 + 启动时日志.
    match &ui_dist {
        Some(p) if p.is_dir() => {
            tracing::info!("[daemon] Web UI serving at /_ui/* from {}", p.display());
        }
        Some(p) => {
            tracing::warn!(
                "[daemon] Web UI dist path does not exist: {} (/_ui/* will return 503). \
                 Build with: pnpm --dir qianxun/src/daemon/ui build",
                p.display()
            );
        }
        None => {
            tracing::info!("[daemon] Web UI disabled (no --ui-dist / QIANXUN_UI_DIST)");
        }
    }

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

    // Stage 3: SessionStore 必须在 AgentLoopHost 之前创建, 这样 host
    // 启动时可以调 `restore_from_disk()` 加载上次未完成的 session.
    // 默认路径: ~/.qianxun/daemon.db (创建目录若不存在).
    let store_path = qianxun_core::workspace::qianxun_dir()
        .map(|d| d.join("daemon.db"))
        .ok_or_else(|| anyhow::anyhow!("cannot determine ~/.qianxun home dir"))?;
    let store = Arc::new(SessionStore::new(&store_path)?);
    tracing::info!(
        "[daemon] session store initialized at {}",
        store_path.display()
    );

    // 包成 Arc<SharedState>, 让 AgentLoopHost 也能引用同一份
    let shared = Arc::new(SharedState::new(
        resolved.clone(),
        provider.clone(),
        tools.clone(),
        memory.clone(),
        skills.clone(),
    ));
    let agent_host = Arc::new(AgentLoopHost::new(10, shared.clone(), store.clone()));

    // Stage 3: 启动恢复 — 加载上次未关闭的 session (Stage 2 留空)
    match agent_host.restore_from_disk().await {
        Ok(n) if n > 0 => {
            tracing::info!("[daemon] restored {n} session(s) from disk");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("[daemon] restore_from_disk failed: {e} (continuing with empty state)");
        }
    }

    let config = Arc::new(resolved);
    // Stage 7a: 从 config 初始化 LLM provider 管理器.
    let llm_providers = Arc::new(LlmProviderManager::from_config(&config));
    tracing::info!(
        "[daemon] LLM provider manager initialized: {} providers, active={}",
        llm_providers.list().len(),
        llm_providers.active_id()
    );

    let state = Arc::new(AppState {
        agent_host,
        config,
        provider,
        tools,
        memory,
        skills,
        shared,
        store,
        llm_providers,
        shutdown_tx,
        processing_loop_enabled: false,
    });

    // 启动 reap_stale 后台任务 (Stage 1 暂不 await, 实际不退出)
    let reap_host = state.agent_host.clone();
    tokio::spawn(async move {
        reap_host.reap_stale().await;
    });

    let app = router::build_router(state, ui_dist);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_rx.changed().await.ok();
        })
        .await?;

    tracing::info!("Daemon stopped");
    Ok(())
}

pub mod agent_host;
pub mod auth;
pub mod kanban_host;
pub mod llm_providers;
pub mod output_sink;
pub mod persistence;
pub mod router;
pub mod service;
pub mod session_runtime;
pub mod sse;
pub mod team_registry;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::{create_provider, LlmProvider};
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;
use tokio::sync::watch;

use qianxun_memory::MemoryCore;

use crate::buf_writer::LogRing;
use crate::daemon::agent_host::{AgentLoopHost, SharedState};
use crate::daemon::auth::AdminCredential;
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
#[allow(dead_code)] // 部分字段 (provider/shared/processing_loop_enabled/active_conns) 在测试中未读, 留 Phase 4 接入
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
    /// Stage 7b: daemon 启动时间戳. `/v1/system/metrics` 计算 uptime.
    /// MVP-3 plan 1: Kanban 子系统集成 (kanban_db + dispatcher + team_registry).
    /// 3 个字段都用 `Option` 让测试场景 (无 Kanban) 不需要构造完整 host.
    pub kanban_db: Option<qianxun_core::kanban::db::KanbanDb>,
    pub kanban_team_registry: Option<qianxun_core::kanban::team::TeamRegistry>,
    pub kanban_host: Option<Arc<kanban_host::KanbanHost>>,
    pub started_at: Instant,
    /// Stage 7b: 当前活跃 HTTP 请求数. auth_middleware 进时 +1, 出时 -1
    /// (drop guard 实现). `/v1/system/metrics` 报告.
    pub active_conns: Arc<AtomicUsize>,
    /// Stage 7b: 内存环形日志缓冲. `/v1/system/logs` endpoint 的数据源.
    /// 暂未接 tracing-subscriber make_writer, 留给 Stage 7c 集成; 当前
    /// 主要是给 endpoint 一个可测试的 ring buffer 抽象.
    pub log_ring: Arc<LogRing>,
    /// Stage 10a: Admin credential (password_hash + token_secret).
    /// auth_middleware 用 `admin.token_secret` 验签 (替代 Stage 6a 的
    /// `QIANXUN_JWT_SECRET` env var).
    pub admin: Arc<AdminCredential>,
}

/// 启动 Daemon HTTP 服务.
///
/// Stage 1 接受外部传入的 `ResolvedConfig` (在 main.rs 中已解析好, 见
/// `Config::from_file` → `resolve` 链路), 构造共享子系统并组装 AppState.
///
/// Stage 7a 新增 `ui_dist: Option<PathBuf>` 参数, 控制是否 serve SvelteKit
/// 静态 dist (路径不存在时不 panic, 启动时 warn 即可).
///
/// Stage 10a: `admin: Arc<AdminCredential>` (由 main.rs 加载 — 失败时
/// 进程已 fail-fast). 这里直接放进 AppState.
///
/// Stage 12 新增 `ui_dev: Option<String>`: dev 模式 — 把 `/ui/*` 反代到 vite
/// dev server, 跟 `--ui-dist` 互斥. 设了 `QIANXUN_SKIP_UI_BUILD=1` 时 build.rs
/// 不跑 pnpm, 配合 scripts/dev.py 启动从 60s → 5s.
pub async fn run(
    port: u16,
    resolved: ResolvedConfig,
    ui_dist: Option<PathBuf>,
    ui_dev: Option<String>,
    admin: Arc<AdminCredential>,
) -> anyhow::Result<()> {
    tracing::info!("Daemon starting on 127.0.0.1:{port}");

    // Stage 7a: Web UI dist 路径决策 + 启动时日志.
    // Stage 12: dev 模式 (`--ui-dev`) 优先 — 跳过兜底 pnpm, 不需要 dist 目录.
    if let Some(dev_url) = ui_dev.as_deref() {
        tracing::info!("[daemon] Web UI dev mode: /ui/* → {} (proxy)", dev_url);
    } else {
        match &ui_dist {
            Some(p) if p.is_dir() => {
                tracing::info!("[daemon] Web UI serving at /ui/* from {}", p.display());
            }
            Some(p) => {
                // 没 build 静态 — 返 503 提示用户跑 pnpm build 或用 scripts/dev.py.
                // 之前会兜底跑 pnpm build, 但 dev 阶段用户用 --ui-dev 走反代
                // 就够了, release 用户用 scripts/release.py 提前 build. 去掉
                // 兜底省 30s 启动.
                tracing::warn!(
                    "[daemon] Web UI dist path does not exist: {} (/ui/* will return 503). \
                     Run `scripts/dev.py` for dev, or `scripts/release.py` for prod build",
                    p.display()
                );
            }
            None => {
                tracing::info!("[daemon] Web UI disabled (no --ui-dist / QIANXUN_UI_DIST)");
            }
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
    // tools: 空 registry + register_all_builtin (Day 1 真初始化, 失败 fallback)
    let mut tools = ToolRegistry::new();
    match tools.register_all_builtin() {
        Ok(n) => tracing::info!(registered = n, "[daemon] builtin tools registered"),
        Err(e) => {
            tracing::warn!(
                error = ?e,
                "[daemon] register_all_builtin failed, fallback to empty"
            );
            // tools 保留为空, 继续
        }
    }
    let tools = Arc::new(tools);

    // memory: 改 in_memory → open("~/.qianxun/mem.db") (Day 3 真初始化, 失败 fallback)
    let mem_path = qianxun_core::workspace::qianxun_dir()
        .map(|d| d.join("mem.db"))
        .unwrap_or_else(|| PathBuf::from("./mem.db"));
    let memory = match MemoryCore::open(&mem_path) {
        Ok(core) => {
            tracing::info!(path = ?mem_path, "[daemon] memory opened");
            Arc::new(core)
        }
        Err(e) => {
            tracing::warn!(
                error = ?e,
                path = ?mem_path,
                "[daemon] memory open failed, fallback to in_memory"
            );
            Arc::new(MemoryCore::open_in_memory().expect("in_memory fallback"))
        }
    };

    // skills: 空 manager + load_all (Day 2 真初始化, 当前 API 无 fail, 空目录静默 OK)
    let skills = SkillManager::load_all(None);
    let skill_count = skills.skill_count();
    if skill_count > 0 {
        tracing::info!(count = skill_count, "[daemon] skills loaded");
    } else {
        tracing::info!("[daemon] no skills loaded (empty or all failed)");
    }

    // Stage 3: SessionStore 必须在 AgentLoopHost 之前创建, 这样 host
    // 启动时可以调 `restore_from_disk()` 加载上次未完成的 session.
    // 默认路径: ~/.qianxun/daemon.db (创建目录若不存在).
    let store_path = qianxun_core::workspace::qianxun_dir()
        .map(|d| d.join("daemon.db"))
        .ok_or_else(|| anyhow::anyhow!("cannot determine ~/.qianxun home dir"))?;
    let store = Arc::new(SessionStore::new(&store_path)?);
    // MVP-3 plan 1: 提前 clone store 共享 connection, 给 KanbanDb 复用 daemon.db
    let store_db = store.db_arc();
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
        agent_host: agent_host.clone(),
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
        // sysinfo 评估: 传递依赖过大 (~80+ 包含 windows-sys, objc2-*, ntapi),
        // 超出 CLAUDE.md "< 30" 约束. 改用 stdlib + /proc/self/status (Linux) +
        // tasklist (Windows) 手读. 这里不用 sysinfo.
        started_at: Instant::now(),
        active_conns: Arc::new(AtomicUsize::new(0)),
        log_ring: Arc::new(LogRing::new()),
        admin,
        // MVP-3 plan 1: 跟 store 共享 connection, 复用 daemon.db
        kanban_db: Some(qianxun_core::kanban::db::KanbanDb::from_connection(store_db.clone())),
        kanban_team_registry: Some(qianxun_core::kanban::team::TeamRegistry::load_default()),
        kanban_host: {
            let team_reg = qianxun_core::kanban::team::TeamRegistry::load_default();
            let host = Arc::new(kanban_host::KanbanHost::new(
                qianxun_core::kanban::db::KanbanDb::from_connection(store_db.clone()),
                team_reg,
            ));
            host.clone().start(agent_host.clone());
            Some(host)
        },
    });

    // 启动 reap_stale 后台任务 (Stage 1 暂不 await, 实际不退出)
    let reap_host = state.agent_host.clone();
    tokio::spawn(async move {
        reap_host.reap_stale().await;
    });

    let app = router::build_router(state.clone(), ui_dist, ui_dev);

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;

    // Stage 10b: 装 SIGINT + SIGTERM 处理器. 收到信号时 broadcast
    // shutdown_tx (让 with_graceful_shutdown 退出 + 触发后续 6 步
    // graceful_shutdown_orchestrator). 跟 cli.rs 现有的 ctrl_c 模式一致.
    let signal_tx = state.shutdown_tx.clone();
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();

        #[cfg(unix)]
        let term = async {
            match tokio::signal::unix::signal(
                tokio::signal::unix::SignalKind::terminate(),
            ) {
                Ok(mut sig) => {
                    sig.recv().await;
                }
                Err(e) => {
                    tracing::error!("[daemon] failed to install SIGTERM handler: {e}");
                    std::future::pending::<()>().await;
                }
            }
        };
        #[cfg(not(unix))]
        let term = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => tracing::info!("[daemon] received SIGINT (Ctrl-C)"),
            _ = term => tracing::info!("[daemon] received SIGTERM"),
        }
        let _ = signal_tx.send(());
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
        })
        .await?;

    // Stage 10b: with_graceful_shutdown 已退出 (停止 accept 新连接).
    // 跑剩余的 6 步 graceful shutdown:
    //   step 3: 等活跃 HTTP/SSE 连接 drain
    //   step 4: cancel 所有活跃 session
    //   step 5: flush store (WAL checkpoint)
    //   step 6: log + return
    // step 1 (broadcast) 和 step 2 (axum 内部停 accept) 已经在 signal handler
    // + with_graceful_shutdown 完成.
    graceful_shutdown_orchestrator(state).await;

    tracing::info!("Daemon stopped");
    Ok(())
}

/// Stage 10b: 6 步 graceful shutdown 编排函数 (从 axum serve 返回后调).
///
/// 拆出独立函数以便单测覆盖 (signal handler 难测, 但 orchestrator 行为
/// 可以 mock AppState 跑). 6 步:
///   1. broadcast shutdown signal  ←─ signal handler 已在 mod.rs::run 完成
///   2. axum 停止 accept 新连接   ←─ with_graceful_shutdown 已完成
///   3. 等活跃 conns drain (max 30s, 超时 warn 继续)
///   4. cancel 所有活跃 session
///   5. flush store (WAL checkpoint)
///   6. log + return
///
/// 返回: () 总是成功 (内部错误只 warn, 不 panic / 不中断).
pub async fn graceful_shutdown_orchestrator(state: Arc<AppState>) {
    use std::time::{Duration, Instant};

    // ── step 3: wait active conns to drain ────────────────────
    tracing::info!("[daemon] step 3: waiting for active connections to drain (max 30s)");
    let drain_start = Instant::now();
    let drain_deadline = drain_start + Duration::from_secs(30);
    while router::active_conns_count() > 0 && Instant::now() < drain_deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let remaining = router::active_conns_count();
    if remaining > 0 {
        tracing::warn!(
            "[daemon] step 3: timed out, {remaining} active conn(s) remain after 30s"
        );
    } else {
        tracing::info!(
            "[daemon] step 3: all connections drained in {:?}",
            drain_start.elapsed()
        );
    }

    // ── step 4: cancel active sessions ────────────────────────
    tracing::info!("[daemon] step 4: cancelling active sessions");
    let total = state.agent_host.session_count();
    let cancelled = state.agent_host.shutdown_all();
    tracing::info!("[daemon] step 4: cancelled {cancelled}/{total} session(s)");

    // ── step 5: flush store ───────────────────────────────────
    tracing::info!("[daemon] step 5: flushing session store (WAL checkpoint)");
    match state.store.flush() {
        Ok(()) => tracing::info!("[daemon] step 5: store flushed successfully"),
        Err(e) => tracing::warn!("[daemon] step 5: flush failed: {e} (continuing)"),
    }

    // ── step 6: done ──────────────────────────────────────────
    tracing::info!("[daemon] step 6: graceful shutdown complete");
}

// ─── Tests (Stage 10b) ──────────────────────────────────────────

/// MVP-3 plan 1 test helper: 构造最小 Kanban 字段 (3 个) 复用 store.db_arc().
/// 4 处测试都调这个, 避免重复 8 行构造.
pub fn make_minimal_kanban_for_test(
    store: &Arc<SessionStore>,
) -> (
    qianxun_core::kanban::db::KanbanDb,
    qianxun_core::kanban::team::TeamRegistry,
    Arc<crate::daemon::kanban_host::KanbanHost>,
) {
    let team_reg = qianxun_core::kanban::team::TeamRegistry::load_default();
    let host = Arc::new(crate::daemon::kanban_host::KanbanHost::new(
        qianxun_core::kanban::db::KanbanDb::from_connection(store.db_arc()),
        team_reg.clone(),
    ));
    (qianxun_core::kanban::db::KanbanDb::from_connection(store.db_arc()), team_reg, host)
}

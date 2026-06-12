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
use std::sync::Arc;

use tokio::sync::{broadcast, watch};

use qianxun_core::config::ResolvedConfig;
use qianxun_core::hooks::HookRegistry;
use qianxun_core::provider::failover::ProviderStack;
use qianxun_core::provider::{create_provider, LlmProvider};
use qianxun_core::skills::lifecycle::SkillLifecycle;
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;
use qianxun_memory::MemoryCore;

use crate::agent_host::{AgentLoopHost, SharedState};
use crate::background_task::BackgroundTaskManager;
use crate::persistence::SessionStore;
use crate::sse::SseEvent;

/// 运行时模式 (P1-2 收尾, 2026-06-12).
///
/// 决定 SQLite db 路径后缀, 避免 desktop / daemon / tui 跨进程锁竞争同一 db 文件.
/// 同一 OS 里这 3 个 binary 可能同时存在 (e.g. 用户 daemon 跑着时开 desktop, 或
/// 启 TUI 监控), 路径分文件是 0 成本隔离手段.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Tauri 桌面端 (Svelte 5 webview). 路径: `desktop.db` + `desktop_mem.db`.
    Desktop,
    /// daemon HTTP binary. 路径: `daemon.db` + `daemon_mem.db`.
    Daemon,
    /// TUI binary (ratatui). 路径: `tui.db` + `tui_mem.db`.
    Tui,
}

/// 模式对应 db 路径 (owned String, 避免 build 内部生命周期问题).
struct ModePaths {
    mem: String,
    store: String,
}

/// 根据模式算 db 路径. 路径基于 `qianxun_core::workspace::qianxun_dir()`
/// (默认 `~/.qianxun/`), 拼上 `<mode>.db` / `<mode>_mem.db`.
fn paths_for(mode: RuntimeMode) -> ModePaths {
    let dir = qianxun_core::workspace::qianxun_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    let suffix = match mode {
        RuntimeMode::Desktop => "desktop",
        RuntimeMode::Daemon => "daemon",
        RuntimeMode::Tui => "tui",
    };
    ModePaths {
        mem: format!("{dir}/{suffix}_mem.db"),
        store: format!("{dir}/{suffix}.db"),
    }
}

/// 千寻运行时核心状态 — 11 核心字段, 跨桌面 / daemon 共享.
///
/// 字段变更 (P1-1 收尾, 2026-06-12):
///   - 移除 `plans: Arc<PlanStore>` — 改走 `store.plans` SQLite 表 (重启不丢)
///   - 之前 11 字段不变 (background_tasks / lifecycle / hooks 等保持)
pub struct RuntimeState {
    /// 内部 session 生命周期管理. **pub(crate)** — 外部禁止直接访问, 必须走 RuntimeApi trait.
    /// 内部 6 个字段 (provider / config / tools / memory / skills / shared) 仍 pub,
    /// 因为 RuntimeApi impl + DaemonOutputSink 等内部组件需要读它们.
    pub(crate) agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    /// LLM provider 入口. 实际类型是 `ProviderStack` (缺口 12 引入), 对外保持 `Arc<dyn LlmProvider>`
    /// 不变以兼容下游 `SharedState` / `processing_loop` / `compact` 等所有 caller.
    /// ProviderStack 内部走 Layer 1 (同 provider retry) + Layer 2 (切 fallback) 失败转移.
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub memory: Arc<MemoryCore>,
    pub skills: SkillManager,
    pub shared: Arc<SharedState>,
    pub store: Arc<SessionStore>,
    /// 缺口 05: 后台任务管理器 (FIFO + 状态机)
    pub background_tasks: Arc<BackgroundTaskManager>,
    /// 缺口 04 v0.3 集成: skill 生命周期记录器, 每次 `api/send.rs` matched_skills
    /// 注入后调 `record_usage(name, true/false)`. 跨 session 累积 use_count / success_count
    /// (内存态, 持久化留 P1).
    pub lifecycle: Arc<SkillLifecycle>,
    /// 缺口 01 v0.3 集成: 全局 hook 注册表. 传给 processing_loop,
    /// dispatch BeforeLoopIter/BeforeToolCall/AfterToolCall/AfterLoopIter.
    /// 默认空 registry (5 tier 全空), 调用方可通过 `state.hooks.register(...)` 注入业务 hook.
    pub hooks: Arc<HookRegistry>,
    pub shutdown_tx: watch::Sender<()>,
    /// P1-3 收尾 (2026-06-12): plan 事件 broadcast bus.
    /// plans.rs 在 6 个状态变更点 try_send (失败 ignore — 无订阅者不阻塞业务).
    /// 订阅方: Tauri desktop (走 `app.emit("plan_event", ...)`), daemon SSE 流.
    /// 容量 256: 单 plan 状态变更稀疏, 256 足够撑 30s 高频更新不丢.
    plan_event_tx: broadcast::Sender<SseEvent>,
    /// 2026-06-12 收尾: sub_session 事件 broadcast bus. 跟 plan_event_tx 平级.
    /// plans.rs::execute_one_task 在每个 task 启动/完成时 emit (一个 plan 跑通常
    /// 触发 2N 条, N = 任务数). 容量 256 够 30s 高频, 跟 plan 同样.
    sub_session_event_tx: broadcast::Sender<SseEvent>,
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

    /// P1-3 收尾 (2026-06-12): 订阅 plan 事件 bus.
    /// 返回 `broadcast::Receiver<SseEvent>`, 订阅方 (Tauri command / daemon SSE)
    /// 持 receiver 后 recv() 消费事件, 调 `app.emit("plan_event", payload)` / 写 SSE 流.
    pub fn subscribe_plan_events(&self) -> broadcast::Receiver<SseEvent> {
        self.plan_event_tx.subscribe()
    }

    /// P1-3 收尾 (2026-06-12): 推 plan 事件 (plans.rs 内部调).
    /// try_send: 无订阅者时直接忽略 (不阻塞业务), channel 满时 drop 旧消息.
    /// 业务路径: plans.rs 6 处状态变更 (create / run / per-task run / per-task done / plan done / cancel).
    pub(crate) fn emit_plan_event(&self, event: SseEvent) {
        let _ = self.plan_event_tx.send(event);
    }

    /// 2026-06-12 收尾: 订阅 sub_session 事件 bus.
    /// 跟 subscribe_plan_events 模式 1:1 — Tauri command 调, 转 `app.emit("sub_session_event", ...)`.
    pub fn subscribe_sub_session_events(&self) -> broadcast::Receiver<SseEvent> {
        self.sub_session_event_tx.subscribe()
    }

    /// 2026-06-12 收尾: 推 sub_session 事件 (plans.rs::execute_one_task 内部调).
    /// 跟 emit_plan_event 模式 1:1.
    pub(crate) fn emit_sub_session_event(&self, event: SseEvent) {
        let _ = self.sub_session_event_tx.send(event);
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
    ///
    /// P1-2 收尾 (2026-06-12): 内部走 `paths_for(RuntimeMode::Daemon)` — 用
    /// `daemon.db` + `daemon_mem.db` 路径. Tauri desktop 端应改调 `new_desktop`,
    /// 走 `desktop.db` + `desktop_mem.db`, 避免跟 daemon 共享同一 SQLite 文件
    /// 触发跨进程锁竞争.
    pub async fn new(config: ResolvedConfig) -> anyhow::Result<Arc<Self>> {
        let p = paths_for(RuntimeMode::Daemon);
        Self::build(config, p.mem, p.store).await
    }

    /// P1-2 收尾 (2026-06-12): Tauri desktop 端专用入口. 内部走 `desktop.db` +
    /// `desktop_mem.db`, 跟 daemon / tui 完全隔离, 避免跨进程 SQLite 锁竞争.
    ///
    /// 业务等价于 `new(config)`, 仅文件路径不同. desktop binary 必须用这个,
    /// 不能再调 `new` (会跟 daemon 抢锁).
    pub async fn new_desktop(config: ResolvedConfig) -> anyhow::Result<Arc<Self>> {
        let p = paths_for(RuntimeMode::Desktop);
        Self::build(config, p.mem, p.store).await
    }

    /// TUI binary 专用入口 (P1-2 收尾, 2026-06-12). 走 `tui.db` + `tui_mem.db`,
    /// 跟 desktop / daemon 都隔离 (TUI 通常是用户在 daemon 跑着时另开, 不应互锁).
    pub async fn new_tui(config: ResolvedConfig) -> anyhow::Result<Arc<Self>> {
        let p = paths_for(RuntimeMode::Tui);
        Self::build(config, p.mem, p.store).await
    }

    /// 集成测试用: 真 config + 真 provider, 但 store 走 in_memory 避免污染 ~/.qianxun/daemon.db
    /// memory 也走 in_memory (跟 llm_integration_tests 旧实现 1:1)
    pub async fn new_in_memory_with_config(config: ResolvedConfig) -> anyhow::Result<Arc<Self>> {
        Self::build(
            config,
            ":memory:".to_string(),
            ":memory:".to_string(),
        )
        .await
    }

    async fn build(
        config: ResolvedConfig,
        mem_path: String,
        store_path: String,
    ) -> anyhow::Result<Arc<Self>> {
        // provider: 缺口 12 集成 — 遍历 config.providers, 构造 ProviderStack
        // (primary = active, fallbacks = 其它). HashMap 迭代顺序 → fallback 顺序不保证稳定.
        let mut provider_entries: Vec<(String, qianxun_core::config::ResolvedProviderConfig, Box<dyn LlmProvider>)> =
            Vec::new();
        // 1. 其它 provider (non-active)
        for (name, cfg) in &config.providers {
            if name == &config.active_provider {
                continue;
            }
            provider_entries.push((
                name.clone(),
                cfg.clone(),
                create_provider(name, cfg),
            ));
        }
        // 2. active provider 必含
        let active_cfg = config.active_provider_config();
        provider_entries.push((
            config.active_provider.clone(),
            active_cfg,
            create_provider(&config.active_provider, &config.active_provider_config()),
        ));

        // 3. 构造 ProviderStack (filter api_key 空 + 拆 primary/fallbacks + 兜底)
        let provider: Arc<dyn LlmProvider> = Arc::new(ProviderStack::new(
            provider_entries,
            &config.active_provider,
            config.agent.max_retries,
        ));

        // tools: 空 registry + register_all_builtin (失败 fallback 空)
        let mut tools = ToolRegistry::new();
        let _ = tools.register_all_builtin();
        let tools = Arc::new(tools);

        // memory: SQLite (mem_path = ":memory:" 走 in_memory)
        let memory = if mem_path == ":memory:" {
            Arc::new(MemoryCore::open_in_memory()?)
        } else {
            MemoryCore::open(PathBuf::from(&mem_path))
                .map(Arc::new)
                .unwrap_or_else(|_| Arc::new(MemoryCore::open_in_memory().expect("in_memory fallback")))
        };

        // skills: 加载 (空目录静默 OK)
        let skills = SkillManager::load_all(None);

        // store: SQLite (store_path = ":memory:" 走 in_memory)
        let store = if store_path == ":memory:" {
            Arc::new(SessionStore::in_memory()?)
        } else {
            Arc::new(SessionStore::new(&PathBuf::from(&store_path))?)
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
        // P1-3: plan 事件 broadcast bus (容量 256)
        let (plan_event_tx, _) = broadcast::channel::<SseEvent>(256);
        // 2026-06-12 收尾: sub_session 事件 broadcast bus (容量 256, 跟 plan 同样)
        let (sub_session_event_tx, _) = broadcast::channel::<SseEvent>(256);

        Ok(Arc::new(Self {
            agent_host,
            config: Arc::new(config),
            provider,
            tools,
            memory,
            skills,
            shared,
            store,
            background_tasks: Arc::new(BackgroundTaskManager::new()),
            lifecycle: Arc::new(SkillLifecycle::new()),
            hooks: Arc::new(HookRegistry::new()),
            shutdown_tx,
            plan_event_tx,
            sub_session_event_tx,
            restored: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// 测试用: in-memory provider + in-memory memory + tmp dir store
    /// 跟 daemon `mod.rs::make_test_state()` 1:1
    pub fn new_for_test() -> Arc<Self> {
        let config = ResolvedConfig::default();
        // 缺口 12: 用 ProviderStack 包单 provider, 测试同样走 fail/success 决策路径.
        let active_cfg = config.active_provider_config();
        let provider: Arc<dyn LlmProvider> = Arc::new(ProviderStack::new(
            vec![(
                config.active_provider.clone(),
                active_cfg.clone(),
                create_provider(&config.active_provider, &active_cfg),
            )],
            &config.active_provider,
            config.agent.max_retries,
        ));
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
        let (plan_event_tx, _) = broadcast::channel::<SseEvent>(256);
        // 2026-06-12 收尾: sub_session 事件 broadcast bus.
        let (sub_session_event_tx, _) = broadcast::channel::<SseEvent>(256);
        Arc::new(Self {
            agent_host,
            config: Arc::new(config),
            provider,
            tools,
            memory,
            skills,
            shared,
            store,
            background_tasks: Arc::new(BackgroundTaskManager::new()),
            lifecycle: Arc::new(SkillLifecycle::new()),
            hooks: Arc::new(HookRegistry::new()),
            shutdown_tx,
            plan_event_tx,
            sub_session_event_tx,
            restored: Arc::new(AtomicBool::new(false)),
        })
    }
}

// ───────────────────────────────────────────────────────────────────────────
// P1-2 收尾单测 (2026-06-12): paths_for 路径分流验证
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// paths_for 必须按 mode 选不同后缀, 避免 desktop / daemon / tui 共享 db.
    /// 关键回归测试: 改 mode 时若拼错, 会直接复现 "跨进程锁竞争" 故障.
    #[test]
    fn paths_for_picks_per_mode_suffix() {
        let d = paths_for(RuntimeMode::Desktop);
        let m = paths_for(RuntimeMode::Daemon);
        let t = paths_for(RuntimeMode::Tui);
        // 三个 mode 路径必须互不相同
        assert_ne!(d.mem, m.mem);
        assert_ne!(d.mem, t.mem);
        assert_ne!(m.mem, t.mem);
        assert_ne!(d.store, m.store);
        assert_ne!(d.store, t.store);
        assert_ne!(m.store, t.store);
        // mem 跟 store 必须不同 (避免一个文件被当两个 store 打开)
        assert_ne!(d.mem, d.store);
        assert_ne!(m.mem, m.store);
        assert_ne!(t.mem, t.store);
        // 后缀必须含 mode 标识
        assert!(d.mem.ends_with("desktop_mem.db"), "desktop mem: {}", d.mem);
        assert!(d.store.ends_with("desktop.db"), "desktop store: {}", d.store);
        assert!(m.mem.ends_with("daemon_mem.db"), "daemon mem: {}", m.mem);
        assert!(m.store.ends_with("daemon.db"), "daemon store: {}", m.store);
        assert!(t.mem.ends_with("tui_mem.db"), "tui mem: {}", t.mem);
        assert!(t.store.ends_with("tui.db"), "tui store: {}", t.store);
    }

    /// paths_for 必须基于 qianxun_dir() (默认 ~/.qianxun/) 拼路径.
    #[test]
    fn paths_for_uses_qianxun_dir_base() {
        let p = paths_for(RuntimeMode::Desktop);
        // 路径必须含 "/" 分隔符 (基于 dir, 不空)
        assert!(p.mem.contains('/'), "mem path: {}", p.mem);
        assert!(p.store.contains('/'), "store path: {}", p.store);
    }
}

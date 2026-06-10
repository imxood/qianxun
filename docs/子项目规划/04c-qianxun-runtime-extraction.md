# A1 详细设计: 抽 `qianxun-runtime` 新 crate

- **Status**: Proposed
- **Date**: 2026-06-08
- **Authors**: Mavis
- **关联**: `04b-tauri-runtime-integration.md` (sub-task #1)
- **前置**: maxu 已 mv `qianxun/src/daemon/` → `qianxun/src/runtime/`, `cargo check --bin qx` 通过

---

## 0. TL;DR (60 秒看完)

**目标**: 把 `qianxun/src/runtime/` 内的 6 个核心 .rs 抽到新 crate `qianxun-runtime`, 跟 `qianxun-core` / `qianxun-memory` 平级. desktop 跟 qianxun binary 都 dep 它, **业务 0 重复**.

**规模**: 9 步, 7-9 小时, 约 1-1.5 天

**风险**: 中 (跨 crate 重构, 涉及 AppState 14 字段拆分 + router 50 处 find/replace). 验证点齐全, 1 步 1 步推进不会失控.

---

## 1. 迁移后代码架构 (迁移前后对比)

### 1.1 workspace 拓扑

**迁移前** (现状):

```
E:\git\maxu\qianxun\
├── qianxun-core/                # workspace lib (agent/provider/tools/skills/mcp)
├── qianxun-memory/              # workspace lib (SQLite + FTS5 + Vector)
└── qianxun/                     # binary (cli/tui/acp/daemon/server/client)
    └── src/
        └── runtime/             # 9 .rs (5 核心 + 4 daemon-specific)
            # ↑ 私有 module, desktop 拿不到
```

**迁移后**:

```
E:\git\maxu\qianxun\
├── qianxun-core/                # lib (不动)
├── qianxun-memory/              # lib (不动)
├── qianxun-runtime/             # ← 新 workspace lib (6 核心)
│   └── src/
│       ├── lib.rs               # pub mod + 顶层重导出
│       ├── state.rs             # RuntimeState
│       ├── agent_host.rs        # AgentLoopHost, SharedState
│       ├── service.rs           # systemd/Windows service 模板
│       ├── persistence.rs       # SessionStore (SQLite)
│       ├── session_runtime.rs   # SessionRuntime, SessionId
│       ├── output_sink.rs       # OutputSink trait + impl
│       └── sse.rs               # SseEvent (12 events) + Builder
└── qianxun/                     # binary (cli/tui/acp/daemon/server/client)
    └── src/
        └── runtime/             # ← 只剩 daemon-specific (HTTP 包装)
            ├── mod.rs           # AppState (内嵌 RuntimeState) + run() 启 axum
            ├── router.rs       # axum router (含 RuntimeState 调用)
            ├── sse_axum.rs     # event_from_sse / event_to_sse (axum 包装)
            ├── auth.rs         # AdminCredential
            ├── llm_providers.rs # LlmProviderManager (CRUD)
            ├── 2 个 integration tests
            ├── ui/              # 旧 SvelteKit (后续退役)
            └── deliverable-8a-daemon.md  # 历史文档
```

`qianxun-desktop/src-tauri` 仍显式 `[workspace]` 隔离, 不受 workspace 加成员影响. 后续 sub-task #2 加 `qianxun-runtime` dep.

### 1.2 crate 依赖图 (迁移后)

```
                      ┌─────────────────┐
                      │ qianxun-desktop │ (Tauri 2.x)
                      │  src-tauri/     │
                      └────┬────────────┘
                           │ path (待加, sub-task #2)
                           ↓
┌─────────────────────┐  ┌──────────────────────┐  ┌──────────────────┐
│ qianxun             │  │ qianxun-runtime      │  │ qianxun-desktop  │
│ (binary: cli/tui/  │  │ (新 lib, 6 核心)    │  │ 自身 (Svelte)    │
│  acp/daemon/...)   │  │                      │  │                  │
│                    │  │  ┌─ state.rs         │  │  ┌─ Svelte UI   │
│ ┌─ runtime/        │  │  ├─ agent_host.rs    │  │  ├─ stores      │
│ │  ├─ mod.rs       │  │  ├─ service.rs       │  │  └─ Tauri cmd  │
│ │  ├─ router.rs    │──┤  ├─ persistence.rs   │──┤                  │
│ │  ├─ sse_axum.rs  │  │  ├─ session_runtime  │  └──────────────────┘
│ │  ├─ auth.rs      │  │  ├─ output_sink.rs   │
│ │  ├─ llm_providers │  │  └─ sse.rs           │
│ │  ├─ ui/ (待退役) │  └──────────┬───────────┘
│ │  └─ 2 tests      │             │ path
│ └─ main.rs         │             ↓
│                    │  ┌──────────────────────┐
│ cli/, tui/, acp/,  │  │ qianxun-core         │
│ server/, client/   │  │ (lib: agent,         │
│ buf_writer.rs      │  │  provider, tools,    │
│                    │  │  skills, mcp)        │
│                    │  └──────────────────────┘
│                    │             ↑ path
│                    │  ┌──────────────────────┐
│                    │──│ qianxun-memory       │
│                    │  │ (lib: SQLite FTS5)   │
└─────────────────────┘  └──────────────────────┘
```

**依赖方向** (单向, 无循环):
- `qianxun-runtime` → `qianxun-core` + `qianxun-memory`
- `qianxun` (binary) → `qianxun-runtime` + `qianxun-core` + `qianxun-memory`
- `qianxun-desktop` (Tauri) → `qianxun-runtime` + `qianxun-core` (sub-task #2 加)

### 1.3 6 核心模块依赖 (qianxun-runtime 内, 无循环)

```
sse.rs           (独立)        ← 业务 SSE 协议层, 12 events
service.rs       (独立)        ← systemd/Windows service 模板
persistence.rs   (独立)        ← SessionStore (SQLite)
session_runtime.rs (独立)      ← SessionRuntime + SessionId
output_sink.rs  → persistence + sse
agent_host.rs   → persistence + session_runtime + memory
```

### 1.4 `RuntimeState` vs `AppState` (关键拆分)

**迁移前**: `qianxun/src/runtime/mod.rs` 有 14 字段 `AppState`, 业务跟 HTTP 特定混在一起.

**迁移后**:

```rust
// qianxun-runtime/src/state.rs (新, 9 核心字段)
pub struct RuntimeState {
    pub agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub memory: Arc<MemoryCore>,
    pub skills: SkillManager,
    pub shared: Arc<SharedState>,
    pub store: Arc<SessionStore>,
    pub shutdown_tx: watch::Sender<()>,
}

// qianxun/src/runtime/mod.rs (改, 6 daemon-specific 字段 + 嵌入 RuntimeState)
pub struct AppState {
    pub runtime: Arc<RuntimeState>,         // ← 嵌入
    pub llm_providers: Arc<LlmProviderManager>,  // 🅱️ HTTP CRUD
    pub started_at: Instant,                    // 🅱️ uptime metric
    pub active_conns: Arc<AtomicUsize>,         // 🅱️ HTTP metric
    pub log_ring: Arc<LogRing>,                  // 🅱️ logs endpoint
    pub admin: Arc<AdminCredential>,             // 🅱️ auth
}
```

**字段访问链** (迁移后):
- 业务访问: `state.runtime.agent_host` / `state.runtime.store` / ...
- daemon-specific: `state.llm_providers` / `state.admin` / ...
- 现有 `state.agent_host` → `state.runtime.agent_host` (router 全面 find/replace, ~50 处)

### 1.5 数据流 (desktop 跟 daemon 共享 RuntimeState)

```
┌──────────────────────────────────────────────────────────────┐
│ qianxun-desktop binary (Tauri 2.x)                          │
│                                                              │
│  Tauri webview (Svelte UI)                                   │
│       ↕ Tauri invoke (in-process, 类型安全)                  │
│  Tauri commands (src-tauri/src/commands/*.rs)               │
│       ↕ 调 RuntimeState 方法                                  │
│  RuntimeState (path dep qianxun-runtime)                    │
│       ↕ 调 AgentLoopHost / SessionStore / ...                 │
│  qianxun-core engine (path dep)                              │
│                                                              │
│  SQLite: ~/.qianxun/desktop.db (新, 不跟 daemon.db 共享)       │
└──────────────────────────────────────────────────────────────┘

vs

┌──────────────────────────────────────────────────────────────┐
│ qianxun binary (CLI)                                         │
│                                                              │
│  qianxun daemon / server / tui / acp / client               │
│       ↕                                                     │
│  AppState (含 runtime: Arc<RuntimeState>)                    │
│       ↕ 调 RuntimeState 方法 + daemon-specific 字段          │
│  RuntimeState (path dep 同一个 crate)                        │
│       ↕                                                      │
│  qianxun-core + qianxun-memory                               │
│                                                              │
│  SQLite: ~/.qianxun/daemon.db (现有)                          │
└──────────────────────────────────────────────────────────────┘
```

**关键决策**:
- 桌面端跟 daemon **不共享 SQLite db**, 各自 `~/.qianxun/desktop.db` / `daemon.db`. 避免跨进程文件锁冲突.
- 同一 `qianxun-runtime` crate 代码, 同一份 `RuntimeState` 逻辑, 不同 db path.

---

## 2. 迁移计划 (9 步, 每步有前/后/验证)

**总时间**: 7-9h. **回滚策略**: 每步独立, 不通过就 git checkout 回到上一步 (maxu 手动 commit).

### Step 1: 建空 crate 跑通编译 (1-2h)

**前**:
- 无 `qianxun-runtime/` 目录
- workspace `Cargo.toml` members: `[qianxun-core, qianxun-memory, qianxun]`

**后**:
- 新 `qianxun-runtime/Cargo.toml` (空 deps: qianxun-core, qianxun-memory, tokio 等基础)
- 新 `qianxun-runtime/src/lib.rs` (空, 只 `//! Crate doc`)
- workspace `Cargo.toml` 加 `"qianxun-runtime"`
- `qianxun/Cargo.toml` 加 `qianxun-runtime = { path = "../qianxun-runtime" }`

**动作**:
1. Write `qianxun-runtime/Cargo.toml`
2. Write `qianxun-runtime/src/lib.rs`
3. Edit `Cargo.toml` (workspace) 加成员
4. Edit `qianxun/Cargo.toml` 加 dep
5. 跑 `cargo check -p qianxun-runtime` → 应该过 (lib 空)
6. 跑 `cargo check --bin qx` → 应该过 (引用新 crate 但没用)

**验证**:
- [ ] `cargo check -p qianxun-runtime` 0 error
- [ ] `cargo check --bin qx` 0 error (跟 step 0 等价)
- [ ] workspace `cargo metadata` 列出 `qianxun-runtime`

**回滚**: `rm -rf qianxun-runtime/` + 改 2 个 `Cargo.toml`.

### Step 2: 挪 `service.rs` (1h, 独立)

**前**:
- `qianxun/src/runtime/service.rs` (7KB, systemd/Windows 模板)
- `qianxun/src/runtime/mod.rs` 有 `pub mod service;`

**后**:
- 新 `qianxun-runtime/src/service.rs` (内容 1:1 搬, use 路径不动因为没用 `crate::xxx`)
- `qianxun-runtime/src/lib.rs` 加 `pub mod service; pub use service::*;`
- `qianxun/src/runtime/mod.rs` 删 `pub mod service;` + `pub use service::*;` 改成 `use qianxun_runtime::service::*;`
- 现有 daemon 内 `crate::runtime::service::*` 引用 (cli 子命令 / 测试) 改 `qianxun_runtime::service::*`

**动作**:
1. `git mv qianxun/src/runtime/service.rs qianxun-runtime/src/service.rs`
2. Edit `qianxun-runtime/src/lib.rs` 加 mod
3. Edit `qianxun/src/runtime/mod.rs` 删 mod + 改 use
4. `grep -rn "crate::runtime::service" qianxun/src/` 改引用

**验证**:
- [ ] `cargo check --bin qx` 0 error
- [ ] `cargo test --bin qx runtime::service::tests` 全 pass (3 个 systemd/windows 测试)
- [ ] `cargo test -p qianxun-runtime service::tests` 全 pass

**回滚**: `git mv` 回来, 反向 use 改回去.

### Step 3: 挪 `persistence.rs` (1h, 独立)

**前**:
- `qianxun/src/runtime/persistence.rs` (29KB, `SessionStore` SQLite)
- 现有依赖: `qianxun_core::agent::conversation::Conversation`, `rusqlite`, `chrono`
- 现有 use 内部依赖: 无 (独立)

**后**:
- 新 `qianxun-runtime/src/persistence.rs` (内容搬, use 路径不动)
- `qianxun-runtime/src/lib.rs` 加 `pub mod persistence; pub use persistence::SessionStore;`
- daemon 引用 `crate::runtime::persistence::SessionStore` 改 `qianxun_runtime::SessionStore`
- router.rs 引用 find/replace

**动作**:
1. `git mv qianxun/src/runtime/persistence.rs qianxun-runtime/src/persistence.rs`
2. Edit `qianxun-runtime/src/lib.rs`
3. Edit `qianxun/src/runtime/mod.rs` 删 mod
4. `grep -rn "crate::runtime::persistence\|use persistence::" qianxun/src/` 改 `qianxun_runtime::SessionStore`

**验证**:
- [ ] `cargo check --bin qx` 0 error
- [ ] `cargo test -p qianxun-runtime persistence` 全 pass (内嵌测试)
- [ ] `cargo test --bin qx mvp1_integration_tests` 不回归

**回滚**: 同 step 2.

### Step 4: 挪 `sse.rs` (1h, 独立, 25KB)

**前**:
- `qianxun/src/runtime/sse.rs` (25KB, SseEvent 12 events + SseEventBuilder 状态机)
- 现有依赖: `qianxun_core::provider::types::LlmStreamEvent`, `qianxun_core::types::StopReason`, `serde`
- 现有内部 use 依赖: 无
- **重要**: axum 包装 `event_from_sse` / `event_to_sse` 在 `router.rs:1520-1531`, 不在 sse.rs 内

**后**:
- 新 `qianxun-runtime/src/sse.rs` (内容搬, 无 use 路径改)
- `qianxun-runtime/src/lib.rs` 加 `pub mod sse; pub use sse::{SseEvent, SseEventBuilder};`
- 抽 `event_from_sse` / `event_to_sse` 到 `qianxun/src/runtime/sse_axum.rs` (新文件, axum 特定, 留 qianxun binary)
- daemon 引用 `crate::runtime::sse::*` 改 `qianxun_runtime::{SseEvent, SseEventBuilder}`
- router.rs `use crate::runtime::sse::{SseEvent, SseEventBuilder}` → `use qianxun_runtime::{SseEvent, SseEventBuilder}; use crate::runtime::sse_axum::{event_from_sse, event_to_sse};`

**动作**:
1. `git mv qianxun/src/runtime/sse.rs qianxun-runtime/src/sse.rs`
2. Edit `qianxun-runtime/src/lib.rs`
3. Write `qianxun/src/runtime/sse_axum.rs` (从 router.rs 抽出 2 个函数)
4. Edit `router.rs` 删 2 个 fn, 改 use
5. Edit `qianxun/src/runtime/mod.rs` 删 `pub mod sse;` 加 `pub mod sse_axum;`

**验证**:
- [ ] `cargo test -p qianxun-runtime sse` 全 pass (12 事件 + SseEventBuilder 状态机测试)
- [ ] `cargo test -p qianxun-runtime sse::tests` 12 事件序列化测试
- [ ] `cargo check --bin qx` 0 error
- [ ] `cargo test --bin qx llm_integration_tests` 不回归 (用 SseEventBuilder)

**回滚**: 同 step 2, sse_axum.rs 删, router.rs 还原 2 个 fn.

### Step 5: 挪 `session_runtime.rs` (0.5h, 独立, 6KB)

**前**:
- `qianxun/src/runtime/session_runtime.rs` (6KB, SessionRuntime + SessionId)
- 现有依赖: `qianxun_core::agent::engine::AgentLoop` 等
- 现有内部 use: 无

**后**:
- 新 `qianxun-runtime/src/session_runtime.rs`
- `qianxun-runtime/src/lib.rs` 加 `pub mod session_runtime; pub use session_runtime::{SessionId, SessionRuntime};`
- daemon 改 use

**动作**: 跟 step 2 模板.

**验证**:
- [ ] `cargo test -p qianxun-runtime session_runtime` 通过

### Step 6: 挪 `output_sink.rs` (1h, 依赖 persistence + sse)

**前**:
- `qianxun/src/runtime/output_sink.rs` (41KB, OutputSink trait + impl)
- 现有 use 内部依赖: `use crate::runtime::persistence::SessionStore;` + `use crate::runtime::sse::{SseEvent, SseEventBuilder};`

**后**:
- 新 `qianxun-runtime/src/output_sink.rs`
- 改 use: `use crate::persistence::SessionStore;` + `use crate::sse::{SseEvent, SseEventBuilder};` (现在是 sibling module)
- `qianxun-runtime/src/lib.rs` 加 `pub mod output_sink; pub use output_sink::OutputSink;`
- daemon 改 use

**动作**:
1. `git mv qianxun/src/runtime/output_sink.rs qianxun-runtime/src/output_sink.rs`
2. Edit `qianxun-runtime/src/output_sink.rs` 改 2 个 use
3. Edit `qianxun-runtime/src/lib.rs`
4. Edit `qianxun/src/runtime/mod.rs` 删 mod

**验证**:
- [ ] `cargo test -p qianxun-runtime output_sink` 通过 (内嵌测试 + 集成测试)
- [ ] `cargo check --bin qx` 0 error

### Step 7: 挪 `agent_host.rs` (1h, 依赖 persistence + session_runtime + memory)

**前**:
- `qianxun/src/runtime/agent_host.rs` (22KB, AgentLoopHost + SharedState)
- 现有 use 内部依赖: `use crate::runtime::persistence::SessionStore;` + `use crate::runtime::session_runtime::{SessionId, SessionRuntime};`

**后**:
- 新 `qianxun-runtime/src/agent_host.rs`
- 改 use: `use crate::persistence::SessionStore;` + `use crate::session_runtime::{SessionId, SessionRuntime};`
- `qianxun-runtime/src/lib.rs` 加 `pub mod agent_host; pub use agent_host::{AgentLoopHost, SharedState};`
- daemon 改 use

**动作**:
1. `git mv qianxun/src/runtime/agent_host.rs qianxun-runtime/src/agent_host.rs`
2. Edit `qianxun-runtime/src/agent_host.rs` 改 2 个 use
3. Edit `qianxun-runtime/src/lib.rs`
4. Edit `qianxun/src/runtime/mod.rs` 删 mod

**验证**:
- [ ] `cargo test -p qianxun-runtime agent_host` 通过
- [ ] `cargo check --bin qx` 0 error

### Step 8: 抽 `RuntimeState` + 改 `AppState` (1.5h, 关键步骤)

**前**:
- `qianxun/src/runtime/mod.rs` 有 14 字段 `AppState`
- `mod.rs::run()` 大段初始化 (create_provider / ToolRegistry / MemoryCore.open / SessionStore.new / AgentLoopHost.new / restore_from_disk)
- `mod.rs` 内 `make_test_state()` 给测试用
- `router.rs` 大量 `state.xxx` 引用 (~50 处)

**后**:

#### 8a. 写 `qianxun-runtime/src/state.rs`

```rust
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;

use qianxun_core::config::ResolvedConfig;
use qianxun_core::provider::{create_provider, LlmProvider};
use qianxun_core::skills::SkillManager;
use qianxun_core::tools::ToolRegistry;
use qianxun_memory::MemoryCore;

use crate::agent_host::{AgentLoopHost, SharedState};
use crate::persistence::SessionStore;

pub struct RuntimeState {
    pub agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub memory: Arc<MemoryCore>,
    pub skills: SkillManager,
    pub shared: Arc<SharedState>,
    pub store: Arc<SessionStore>,
    pub shutdown_tx: watch::Sender<()>,
}

impl RuntimeState {
    /// 完整初始化: provider / tools / memory / skills / SessionStore / AgentLoopHost
    /// 跟 daemon 启动逻辑 1:1 (从 `qianxun/src/runtime/mod.rs::run` 抽出来)
    pub async fn new(config: ResolvedConfig) -> anyhow::Result<Arc<Self>> {
        let provider: Arc<dyn LlmProvider> = create_provider(
            &config.active_provider,
            &config.active_provider_config(),
        ).into();
        let mut tools = ToolRegistry::new();
        let _ = tools.register_all_builtin();
        let tools = Arc::new(tools);
        let mem_path = qianxun_core::workspace::qianxun_dir()
            .map(|d| d.join("mem.db"))
            .unwrap_or_else(|| PathBuf::from("./mem.db"));
        let memory = MemoryCore::open(&mem_path)
            .map(Arc::new)
            .unwrap_or_else(|_| Arc::new(MemoryCore::open_in_memory().expect("in_memory fallback")));
        let skills = SkillManager::load_all(None);
        let store_path = qianxun_core::workspace::qianxun_dir()
            .map(|d| d.join("daemon.db"))
            .ok_or_else(|| anyhow::anyhow!("cannot determine ~/.qianxun home dir"))?;
        let store = Arc::new(SessionStore::new(&store_path)?);
        let shared = Arc::new(SharedState::new(
            config.clone(), provider.clone(), tools.clone(),
            memory.clone(), skills.clone(),
        ));
        let agent_host = Arc::new(AgentLoopHost::new(10, shared.clone(), store.clone()));
        agent_host.restore_from_disk().await.ok();
        let (shutdown_tx, _) = watch::channel(());
        Ok(Arc::new(Self {
            agent_host, config: Arc::new(config), provider, tools,
            memory, skills, shared, store, shutdown_tx,
        }))
    }

    /// 测试用: in-memory provider + memory + tmp dir store
    pub fn new_for_test() -> Arc<Self> {
        let config = ResolvedConfig::default();
        let provider: Arc<dyn LlmProvider> = create_provider(
            &config.active_provider, &config.active_provider_config()
        ).into();
        let tools = Arc::new(ToolRegistry::new());
        let memory = Arc::new(MemoryCore::open_in_memory().expect("in-memory mem"));
        let skills = SkillManager::new();
        let tmp = std::env::temp_dir().join(format!(
            "qianxun_runtime_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let store = Arc::new(SessionStore::new(&tmp).expect("open store"));
        let shared = Arc::new(SharedState::new(
            config.clone(), provider.clone(), tools.clone(),
            memory.clone(), skills.clone(),
        ));
        let agent_host = Arc::new(AgentLoopHost::for_test(10, config.clone()));
        let (shutdown_tx, _) = watch::channel(());
        Arc::new(Self {
            agent_host, config: Arc::new(config), provider, tools,
            memory, skills, shared, store, shutdown_tx,
        })
    }
}
```

#### 8b. 改 `qianxun/src/runtime/mod.rs::AppState`

```rust
// 改前 (14 字段):
pub struct AppState {
    pub agent_host: Arc<AgentLoopHost>,
    pub config: Arc<ResolvedConfig>,
    // ... 14 字段
}

// 改后 (6 字段 + 嵌入 RuntimeState):
use qianxun_runtime::RuntimeState;

pub struct AppState {
    pub runtime: Arc<RuntimeState>,
    pub llm_providers: Arc<LlmProviderManager>,
    pub started_at: Instant,
    pub active_conns: Arc<AtomicUsize>,
    pub log_ring: Arc<LogRing>,
    pub admin: Arc<AdminCredential>,
}
```

#### 8c. 简化 `mod.rs::run()`

```rust
// 改前: 大段初始化 (line 95-232)
// 改后:
pub async fn run(
    port: u16,
    config: ResolvedConfig,
    ui_dist: Option<PathBuf>,
    admin: Arc<AdminCredential>,
) -> anyhow::Result<()> {
    tracing::info!("Daemon starting on 127.0.0.1:{port}");
    // ... ui_dist warn log ...

    let runtime = RuntimeState::new(config).await?;  // ← 1 行代替 ~140 行
    let config = runtime.config.clone();

    let llm_providers = Arc::new(LlmProviderManager::from_config(&config));
    // ... (其它 daemon-specific 初始化)

    let state = Arc::new(AppState {
        runtime,  // ← 嵌入
        llm_providers,
        started_at: Instant::now(),
        active_conns: Arc::new(AtomicUsize::new(0)),
        log_ring: Arc::new(LogRing::new()),
        admin,
    });
    // ... (HTTP 启动逻辑不变)
}
```

#### 8d. 改 `qianxun/src/runtime/mod.rs::make_test_state()` (line 362-414)

```rust
// 改前: make_test_state() 内手动构 AppState 所有字段
// 改后:
fn make_test_state() -> Arc<AppState> {
    let runtime = RuntimeState::new_for_test();
    Arc::new(AppState {
        runtime,
        llm_providers: Arc::new(LlmProviderManager::from_config(&ResolvedConfig::default())),
        started_at: Instant::now(),
        active_conns: Arc::new(AtomicUsize::new(0)),
        log_ring: Arc::new(LogRing::new()),
        admin: Arc::new(AdminCredential::for_test("test_secret_xx", "placeholder_hash")),
    })
}
```

#### 8e. router.rs 全面 find/replace (~50 处)

| 改前 | 改后 |
|---|---|
| `state.agent_host` | `state.runtime.agent_host` |
| `state.config` | `state.runtime.config` |
| `state.provider` | `state.runtime.provider` |
| `state.tools` | `state.runtime.tools` |
| `state.memory` | `state.runtime.memory` |
| `state.skills` | `state.runtime.skills` |
| `state.shared` | `state.runtime.shared` |
| `state.store` | `state.runtime.store` |
| `state.shutdown_tx` | `state.runtime.shutdown_tx` |

(daemon-specific 字段保留: `state.llm_providers`, `state.started_at`, `state.active_conns`, `state.log_ring`, `state.admin`)

**动作**:
1. Write `qianxun-runtime/src/state.rs` (~110 行)
2. Edit `qianxun-runtime/src/lib.rs` 加 `pub mod state; pub use state::RuntimeState;`
3. Edit `qianxun/src/runtime/mod.rs`:
   - 改 `AppState` 字段 (14 → 6 + 1 嵌入)
   - 改 `use` 加 `qianxun_runtime::RuntimeState`
   - 改 `run()` 调 `RuntimeState::new()` (~140 行 → 1 行)
   - 改 `make_test_state()` 调 `RuntimeState::new_for_test()`
4. Edit `qianxun/src/runtime/router.rs` find/replace 9 个字段, ~50 处
5. Edit 其他引用 (cli/, tui/, acp/, server/, client/) 任何 `state.xxx` 引用

**验证**:
- [ ] `cargo check --bin qx` 0 error
- [ ] `cargo test --bin qx` 全 pass (含 graceful_shutdown / mvp1 / llm_integration)
- [ ] `cargo test -p qianxun-runtime state` 通过 (新加的 RuntimeState 测试)

**回滚**: git checkout 整个 step 8 的修改.

### Step 9: 收尾 (1h)

**动作**:
1. `cargo test --workspace` 全 pass
2. `cargo clippy --workspace` 0 warning (修 lint)
3. 删 6 旧文件 (Step 2-7 已经 mv, 实际不用再删, 确认路径):
   - `qianxun/src/runtime/{agent_host,service,persistence,session_runtime,output_sink,sse}.rs` 应该已经不存在
4. 写经验文档 `docs/经验/2026-06-08_qianxun_runtime_extraction.md`
5. (可选) `git status` 看一下变更规模

**验证清单**:
- [ ] `cargo test --workspace` 全 pass
- [ ] `cargo clippy --workspace` 0 warning
- [ ] 6 旧 .rs 不存在
- [ ] `qianxun/src/runtime/` 只剩 5 daemon-specific + 2 tests + ui/ + 历史
- [ ] 经验文档写完

---

## 3. 验证清单 (sub-task #1 跑通验收)

- [ ] workspace `cargo check --workspace` 0 error
- [ ] `cargo test -p qianxun-runtime` 5 核心 + state 测试全 pass
- [ ] `cargo test --bin qx` daemon 现有测试 (graceful_shutdown / mvp1_integration / llm_integration) 全 pass
- [ ] `cargo clippy --workspace` 0 warning
- [ ] 6 旧 .rs 已删, `qianxun/src/runtime/` 目录干净
- [ ] desktop `Cargo.toml` **不**改 (sub-task #2 才加 dep)
- [ ] 经验沉淀到 `docs/经验/`

---

## 4. 风险 + 缓解

| 风险 | 缓解 |
|---|---|
| `state.xxx` → `state.runtime.xxx` 漏改 | 编译器 unknown field 报错, 逐个修. 9 字段 find/replace |
| `AppState::make_test_state` 跟 RuntimeState 循环引用 | RuntimeState 独立, 测试用 `new_for_test()` |
| 5 核心 use 路径改漏 | 编译器 unresolved import 报错 |
| 跨 crate 编译时间 +1 | 接受 (qianxun-runtime 比 qianxun-core 大, 但共享 cargo build cache) |
| qianxun-runtime dep qianxun-memory, qianxun-memory dep qianxun-core (单向链) | 验证后无循环 |
| Tauri desktop 集成时, qianxun-runtime 编译 Tauri 进程可能慢 | Tauri 2.x build cache 抵消, 接受 |
| sse_axum.rs 从 router.rs 抽出漏 import | step 4 验证里 cargo test llm_integration 必过 |

---

## 5. 不在本设计内 (后续 sub-task)

- desktop `Cargo.toml` 加 `qianxun-runtime` dep → **sub-task #2**
- Tauri commands 注册 RuntimeState 方法 → **sub-task #3**
- 抽 `RuntimeApi` trait (daemon router + Tauri command 共用) → **sub-task #3** 内
- 退役 `qianxun/src/runtime/ui/` (旧 SvelteKit) → 后续清理

---

## 6. References

- `04b-tauri-runtime-integration.md` (本规划上位, sub-task #1-7 排序)
- ADR-0003 (合并 desktop + 2-mode 互斥)
- `qianxun/src/runtime/` (5 核心 + 6 daemon-specific 当前)
- `qianxun-core` / `qianxun-memory` (workspace 现有 lib crate)
- `qianxun-desktop/src-tauri/Cargo.toml` (待 sub-task #2 改)

---

## 7. 实施检查表 (执行时打印)

执行时按这个表走, 每步打勾:

- [ ] Step 1: 空 crate 编译通过
- [ ] Step 2: service.rs 挪完
- [ ] Step 3: persistence.rs 挪完
- [ ] Step 4: sse.rs 挪完 + sse_axum.rs 抽出
- [ ] Step 5: session_runtime.rs 挪完
- [ ] Step 6: output_sink.rs 挪完
- [ ] Step 7: agent_host.rs 挪完
- [ ] Step 8: RuntimeState + AppState 拆完 (关键步骤)
- [ ] Step 9: workspace 全测 + clippy + 经验文档

# Tauri + Runtime 集成规划

- **Status**: Proposed
- **Date**: 2026-06-08
- **Authors**: Mavis (按 maxu 决策写)
- **关联**: ADR-0003 (合并 desktop + 2-mode 互斥)

---

## Context

千寻 desktop 当前是 mock 阶段 (51 文件, 跑 setTimeout 模拟流式). 4a-1 写完 IPC client + mock server + 11 个单元测试.

`qianxun` binary 之前叫 `daemon` 的代码已被 maxu 重命名为 `runtime` (mv `qianxun/src/daemon/` → `qianxun/src/runtime/`, `cargo check --bin qx` 通过). 这是为 desktop 集成做的第一步.

**目标**: desktop (Tauri 2.x webview) 能调真 runtime (engine / session / plan / 持久化), 跟用户"tauri 桌面端能正常工作, 这是首先要实现的"对齐.

## 当前状态

```
qianxun/src/
├── runtime/                      # ← 重命名自 daemon, 9 module
│   ├── agent_host.rs (22KB)      # ⭐ 核心: AgentLoopHost
│   ├── service.rs (7KB)          # ⭐ 核心: daemon service 抽象
│   ├── persistence.rs (29KB)     # ⭐ 核心: SessionStore SQLite
│   ├── session_runtime.rs (6KB)  # ⭐ 核心: session 跑时
│   ├── output_sink.rs (41KB)     # ⭐ 核心: OutputSink
│   ├── auth.rs (19KB)            # 🅱️ daemon-specific
│   ├── llm_providers.rs (23KB)   # 🅱️ daemon-specific
│   ├── mod.rs (20KB)             # 🅱️ 启 axum + AppState (HTTP 特定)
│   ├── router.rs (153KB)         # 🅱️ axum router + middleware
│   ├── sse.rs (25KB)             # 🅱️ axum SSE builder
│   ├── llm_integration_tests.rs  # 🅰️ 测试 (留)
│   ├── mvp1_integration_tests.rs # 🅰️ 测试 (留)
│   ├── deliverable-8a-daemon.md  # 🅰️ 历史文档 (留)
│   └── ui/                       # 🅱️ 旧 SvelteKit UI (跟 desktop 重叠, 退役)
├── main.rs (mod runtime;)
├── tui/, cli/, acp/, server/, client/  # 其他多模式入口
```

**5 核心** 是 desktop 复用目标. **6 daemon-specific + 2 测试 + 1 历史文档** 留 qianxun binary 内的 `runtime` (HTTP 特定 / 业务测试 / 历史).

## 复用方案对比

要让 desktop 调 runtime 5 核心, 必须把它们从 `qianxun` binary 私有 module 抽出来, 因为 desktop 当前的 `Cargo.toml` 只 dep `qianxun-core`, 拿不到 `qianxun::runtime::*` (那是 binary 私有的).

| 方案 | 描述 | 改动 | 工作量 | 风险 |
|---|---|---|---|---|
| **A1. 抽 `qianxun-runtime` 新 crate** ⭐ | runtime 5 核心 + 必要依赖 → 新 crate `qianxun-runtime`, 跟 `qianxun-core` / `qianxun-memory` 平级, desktop + qianxun 都 dep 它 | 新 crate +1, runtime 5 .rs 移过去, qianxun-memory dep 移过去 | 0.5-1 天 | **低**: 业务隔离清晰, 编译影响小 |
| A2. 全挪到 `qianxun-core` | runtime 5 核心 + MemoryCore → `qianxun-core` (跟 `qianxun-memory` 平级), qianxun-memory 解散或 thin wrapper | qianxun-core 变成 monolith, 跨 crate 循环依赖要破 | 1-1.5 天 | 中: 工作量大, 改动面广 |
| A3. desktop dep `qianxun` binary | desktop 加 `qianxun = { path = "../qianxun" }` | bin crate dep 容易死锁/编译慢 | 0.5 天 | **高**: 不推荐 |

**推荐 A1** (跟用户"最简化代码" + "避免冗余" 偏好一致):
- 1 个新 crate 但 业务隔离清晰 (5 核心跟 daemon-specific 分开)
- 编译影响小 (qianxun binary 走 `qianxun_runtime::*` 代替内部 module)
- desktop 复用路径清晰 (path dep `qianxun-runtime`)
- 不动现有 crate 结构 (qianxun-core / qianxun-memory 不变)

### A1 详细 plan

**新 crate** `qianxun-runtime/`:
- `Cargo.toml` (workspace 成员, dep `qianxun-core` + `qianxun-memory` + axum + tokio + ...)
- `src/lib.rs` (pub mod)
- 5 .rs 挪过来 (agent_host / service / persistence / session_runtime / output_sink)
- use 路径改 (`use crate::daemon::xxx` → `use crate::xxx`)
- AppState 从 `runtime/mod.rs` 抽到 `qianxun_runtime::RuntimeState` (共享给 desktop + qianxun binary)
- 测试保留

**qianxun binary 改动**:
- `Cargo.toml` 加 `qianxun-runtime = { path = "../qianxun-runtime" }`
- `src/main.rs` 改 `mod runtime;` → `mod runtime;` (只留 router / sse / auth / llm_providers / tests), 加 `use qianxun_runtime::*;`
- 6 个 daemon-specific 留 `qianxun/src/runtime/` 内, 调 `qianxun_runtime::RuntimeState` (HTTP 包装)

**desktop 改动** (后续, 4a-2):
- `qianxun-desktop/src-tauri/Cargo.toml` 加 `qianxun-runtime = { path = "../../qianxun-runtime" }`
- `src-tauri/src/runtime_setup.rs` (新): 跟 daemon 一样初始化 runtime (provider, tools, memory, skills, SessionStore, AgentLoopHost)
- Tauri commands 注册: 把 `RuntimeState` 方法注册成 Tauri commands, Svelte webview 走 invoke
- Svelte stores 改 Tauri invoke, 删 mock 阶段 setTimeout

### Cargo workspace 调整

```toml
# E:\git\maxu\qianxun\Cargo.toml
[workspace]
members = [
    "qianxun-core",
    "qianxun-memory",
    "qianxun",        # binary
    "qianxun-runtime", # ← 新加
    # 注意: qianxun-desktop/src-tauri 显式 [workspace] 隔离, 不加这里
]
```

## Sub-task 排序 (跟 ADR-0003 9 项对齐 + 用户的"先做 tauri + core" 优先级)

| # | Sub-task | 时间 | 阻塞 | 复用 |
|---|---|---|---|---|
| **1** ⭐ | 抽 `qianxun-runtime` 新 crate (5 核心 .rs + 必要 deps) | 0.5-1 天 | — | ✅ 全复用, 改 use 路径 |
| 2 | desktop `src-tauri/Cargo.toml` 加 `qianxun-runtime` dep, 加 `qianxun-memory` dep | 0.5 天 | 1 | ✅ |
| 3 | Tauri commands 注册: `health_check` 调真 runtime, 新增 `list_sessions` / `send_message` / `create_plan` / `cancel_session` / `load_session` | 1 天 | 2 | ✅ 复用 daemon router handler 逻辑 (router 留在 qianxun/src/runtime/) |
| 4 | Svelte `chatStore` 改 Tauri invoke: 删 `streamMock` (mock 阶段 helper), `send` 调 `invoke('send_message')` + Tauri event listen 流式事件 | 1 天 | 3 | 改 transport, 业务不变 |
| 5 | Svelte `planStore` 改 Tauri invoke: 删 `scheduleAutoComplete` (setTimeout), 改调 `invoke('create_plan')` + event listen plan_update | 0.5 天 | 3 | 改 transport |
| 6 | Svelte 其他 stores (session / project / subSession) 改 Tauri invoke, 持久化数据全切 | 1 天 | 3 | 改 transport |
| 7 | desktop 端到端跑通: 1) 启 desktop 2) 发消息 3) 看真 LLM 流式响应 4) plan 创建 + 调度 5) session 持久化 6) 重启状态恢复 | 1 天 | 4-6 | 端到端验证 |

**总 5-6 天**.

## 数据流 (集成后)

```
┌─────────────────────────────────────────────────────┐
│ qianxun-desktop binary (Tauri 2.x)                 │
│                                                     │
│  Tauri webview (Svelte UI)                          │
│       ↕ Tauri invoke (in-process, 类型安全)          │
│  Tauri commands                                     │
│  (注册在 src-tauri/src/commands/*.rs)              │
│       ↕                                             │
│  qianxun_runtime::RuntimeState (in-process)         │
│  (path dep 复用 daemon 5 核心)                      │
│       ↕                                             │
│  qianxun-core (engine, plan, tools, provider)       │
│                                                     │
│  SQLite: ~/.qianxun/desktop.db (新文件, 不跟 daemon.db 共享)│
└─────────────────────────────────────────────────────┘

vs

┌─────────────────────────────────────────────────────┐
│ qianxun binary (CLI)                                │
│                                                     │
│  qianxun daemon / server / tui / acp / client       │
│       ↕                                             │
│  qianxun::runtime (mod.rs / router / sse / auth)    │
│  (HTTP 特定, 留 qianxun binary)                      │
│       ↕                                             │
│  qianxun_runtime::RuntimeState (同一个 crate)        │
│       ↕                                             │
│  qianxun-core + qianxun-memory                       │
│                                                     │
│  SQLite: ~/.qianxun/daemon.db (现有)                │
└─────────────────────────────────────────────────────┘
```

**关键**: 桌面端用 `~/.qianxun/desktop.db`, daemon 用 `~/.qianxun/daemon.db`. 两个独立 db, 避免 SQLite 跨进程文件锁冲突. (同一进程没冲突, 但**未来 desktop 跟 daemon 同时跑**是用户场景之一, 不能共享 db).

## 验证清单 (跟 FEATURE-CHECKLIST 60 项对得上)

**Sub-task #1 (抽 qianxun-runtime) 验证**:
- [ ] `cargo check -p qianxun-runtime` 通过
- [ ] `cargo test -p qianxun-runtime` 现有测试全 pass
- [ ] `cargo check --bin qx` 仍通过 (qianxun binary 改完)
- [ ] `cargo test -p qianxun --bin qx` daemon 现有测试 (mvp1_integration_tests 等) 仍 pass

**Sub-task #2-3 (desktop 集成) 验证**:
- [ ] `cargo check` (Tauri) 通过
- [ ] Tauri 启, webview 显示 Svelte UI
- [ ] 发消息 → 真 LLM 流式响应 (Tauri event)
- [ ] 切 session → Tauri invoke 调真 RuntimeState
- [ ] 启 / 关 / 重启 desktop, 状态正确持久化

**Sub-task #7 (端到端) 验证**: 跑通 FEATURE-CHECKLIST 60 项

## 风险 + 缓解

| 风险 | 缓解 |
|---|---|
| runtime 5 核心跨 crate 抽, 内部 use 路径大量改 | 一次性 find/replace `crate::daemon::` → `crate::`, 加 Cargo.toml 自动 lint |
| qianxun-memory 跟 qianxun-runtime 双向依赖风险 | 现状单向: qianxun-memory → qianxun-core. qianxun-runtime 跟 qianxun-memory 是 sibling, qianxun-runtime dep qianxun-memory 即可 |
| desktop db 跟 daemon db 独立, 数据同步 | 接受 — 桌面跟 daemon 是两套环境. 未来需要同步时, 加 import/export 命令 |
| Cargo workspace 调整, qianxun-desktop/src-tauri 显式隔离 | qianxun-desktop/src-tauri 已经有 `[workspace]` 隔离, 新 crate 不影响它 |
| Tauri command 跟 daemon router handler 业务重复 | 抽 trait `RuntimeApi`, daemon router 跟 Tauri command 都实现这 trait, 业务 0 重复 (后续 sub-task) |

## 不在本次范围 (跟 ADR-0003 留规划一致)

- ~~ACP stdio 模式集成~~ — 留规划, 未来某天干
- ~~WebSocket server 集成~~ — ADR-0003 提到, 留未来 (Zed 当前 WS ACP 不成熟)
- ~~退役 `qianxun/src/runtime/ui/` (旧 SvelteKit UI)~~ — 跟 desktop 重叠, 4a 后续清理
- ~~更新 `_shared-contract.md` 跟真实现一致~~ — 4a 后续

## References

- ADR-0003: `docs/30_决策/ADR-0003_desktop_2mode.md` (2-mode 互斥架构)
- `docs/40_经验/2026-06-08_desktop_mock_phase.md` (mock 阶段经验)
- `docs/40_经验/2026-06-08_phase_4a-1_runbook.md` (4a-1 跑通指南)
- qianxun/src/runtime/ (5 核心 + 6 daemon-specific, 详细见上文)
- qianxun-core + qianxun-memory (workspace 现有 lib crate)
- qianxun-desktop/src-tauri/Cargo.toml (只 dep qianxun-core, 待加 qianxun-runtime)

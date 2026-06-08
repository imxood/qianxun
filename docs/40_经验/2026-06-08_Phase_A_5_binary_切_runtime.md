# Phase A 收尾经验: 5 Binary 入口切 qianxun-runtime

> 日期: 2026-06-08
> 范围: TUI / ACP / CLI / client / server 5 个 binary 入口切 `qianxun-runtime::RuntimeState`
> 状态: ✅ TUI/ACP 切完, CLI/client/server 不需要改 (CLI 走 TUI, client 是 thin client, server 是 VPS Hub 不跑 AgentLoop)

## TL;DR

| 入口 | 改前 | 改后 | 备注 |
|---|---|---|---|
| **TUI** | `App::new(config)` 自己 `create_provider` + `register_builtin` + `build_memory` + `load_all skills` | `App::with_runtime(state)` 拿 `state.provider` / `(*state.tools)` / `(*state.memory)` / `state.skills` | 5 binary 共享同一份 RuntimeState 初始化逻辑 (单点维护) |
| **ACP** | `AcpRequestHandler` 持有 `provider` + `agent_config` + `compact_config` + `budget_*` + `tools` 6 字段 | `AcpRequestHandler` 持有 `Arc<RuntimeState>` + `forwarding_tools` 2 字段 | forwarding tools 留 (依赖 transport RPC), provider/config/memory 全从 state 拿 |
| **CLI** | `cli::run::run_repl(config)` → `tui::run(config)` | 不变 (间接走 TUI) | TUI 已切, CLI 自动受益 |
| **client** | thin HTTP client, 不持有 runtime | 不变 | 本来就不需要 provider/tools |
| **server** | VPS WebSocket Hub, 不跑 AgentLoop | 不变 | N/A — Phase B 范围 |

## 关键决策

### 1. TUI App::new 改写成 App::new → RuntimeState::new → App::with_runtime 链

```rust
// 旧: 独立初始化, 跟 desktop/daemon 重复实现
async fn new(config: ResolvedConfig, ...) -> ... {
    let mut tools = ToolRegistry::new();
    builtin::register_all(&mut tools);
    // ... 5+ 步骤
}

// 新: 委托 RuntimeState, 跟 desktop/daemon 单点维护
async fn new(config: ResolvedConfig, ...) -> ... {
    let state = qianxun_runtime::RuntimeState::new(config).await?;
    Self::with_runtime(state, project_root, global_instructions).await
}

async fn with_runtime(state: Arc<RuntimeState>, ...) -> ... {
    let provider = state.provider.clone();      // Arc<dyn LlmProvider>
    let mut tools = (*state.tools).clone();     // ToolRegistry: Clone
    let memory = Some(Box::new((*state.memory).clone()));  // MemoryCore: 加 #[derive(Clone)]
    let skills = state.skills.clone();          // SkillManager: Clone
    // 局部补: SkillReadTool + workspace MCP (RuntimeState::build 不连 workspace MCP)
    tools.register_builtin(Arc::new(builtin::SkillReadTool { manager: Arc::new(skills.clone()) }));
    connect_workspace_mcp(ws_root.as_deref(), &mut tools).await;
    // ...
}
```

**为什么这么分两步**:
- `App::new(config)` 仍然存在, 给 `test_app()` 测试 helper 用 (单线程无 LLM, 不需要真 RuntimeState)
- `App::with_runtime(state)` 是新生产路径, 接受外部传入的 RuntimeState
- 5 binary 入口 (TUI/ACP/CLI/desktop) 共享 `RuntimeState::new(config)` 初始化, **不重复**实现

### 2. ACP forwarding_tools 跟 state.tools 是两套

```rust
pub struct AcpRequestHandler {
    pub transport: Arc<AcpTransport>,
    pub state: Arc<RuntimeState>,                          // ✅ provider/memory/config 统一来源
    pub forwarding_tools: Arc<ToolRegistry>,              // ACP 特有: 文件读/写转 RPC
    pub sessions: Arc<Mutex<SessionManager>>,
    pub output_tx: mpsc::UnboundedSender<AcpOutputEvent>,
}
```

**为什么不合并 forwarding_tools 进 state.tools**:
- forwarding tools 依赖 `transport` (文件读/写走 JSON-RPC 发到 Zed)
- state.tools 走 `register_all_builtin` + workspace MCP (本地执行)
- 两者语义不同: forwarding 是 RPC 代理, state.tools 是本地工具
- Phase A 保留两者并存, 后续 Phase 4b/4c 评估合并

### 3. AcpSession 删 `memory` 字段

```rust
// 旧: per-session 持有 memory
pub struct AcpSession {
    pub memory: Option<Box<dyn MemoryObserver + Send>>,
    // ...
}

// 新: 走 state.memory (Arc<MemoryCore>, 全局共享)
pub struct AcpSession {
    // memory 字段删除
    // ...
}
```

**为什么**:
- MemoryCore 全局唯一 (一个 daemon 进程一个 mem.db)
- 之前 per-session 持有只是把 Arc clone 一份, 浪费内存且语义错位
- 跟 RuntimeState 的 `state.memory` 单一来源对齐

### 4. MemoryCore 加 `#[derive(Clone)]`

```rust
// qianxun-memory/src/lib.rs
- pub struct MemoryCore { ... }
+ #[derive(Clone)]
+ pub struct MemoryCore { ... }
```

**为什么不自己实现**:
- 内部只有 `Arc<Mutex<Connection>>` + `Arc<Mutex<Option<CurrentSession>>>`, 全是 Arc
- `#[derive(Clone)]` 自动生成 `Arc::clone` 序列, 跟手写一致
- Clone 不复制 SQLite 连接, 多个 `MemoryCore` 共享同一连接和 current_session

### 5. main.rs ACP 路径简化

```rust
// 旧: 6 个参数传递
if cli.acp_mode {
    let provider = create_provider(&resolved.active_provider, &resolved.active_provider_config());
    crate::acp::run_acp_server(provider, resolved.agent.clone(),
        Some(resolved.compaction.clone()),
        resolved.budget.max_input_tokens, resolved.budget.max_output_tokens).await?;
}

// 新: 1 个 state
if cli.acp_mode {
    let state = qianxun_runtime::RuntimeState::new(resolved).await?;
    crate::acp::run_acp_server(state).await?;
}
```

**业务收益**:
- main.rs ACP 路径跟 daemon 路径统一: 都是 `RuntimeState::new(resolved).await?` + 分发
- ACP 跟 TUI/CLI/desktop 共享 init 逻辑, bug 修复一处生效

## 踩过的坑

### 1. `App::with_runtime` 函数名引用了但没实现 — 上轮 Phase 4a 写漏

**症状**:
```rust
pub async fn run_with_runtime(
    state: Arc<qianxun_runtime::RuntimeState>,
    ...
) -> anyhow::Result<()> {
    let mut app = App::with_runtime(state, project_root, global_instructions).await?;
    // ...
}
```
`App::with_runtime` 调用了, 但函数没实现. 整个 `cargo check --bin qx` 直接 fail.

**根因**:
- 上一轮 (commit 7d9a99a 之前) 试图把 TUI 切 RuntimeState, 写了入口 `run_with_runtime` 和 `tui::run` 改调 `RuntimeState::new(config).await?`, 但漏了 `App::with_runtime` 的实现
- 头文件引用 → 编译错误 → 整套 binary 都跑不了 (daemon 命令也走 qx 入口)

**修法**:
- 加 `App::with_runtime(state, project_root, global_instructions) -> ...` 函数, 内部从 state 拿 provider/tools/memory/skills
- `App::new(config)` 改写成 `RuntimeState::new(config).await?` + `App::with_runtime(state, ...)` 委托

**教训**:
- 任何抽 refactor, 写完入口 + 调用点后, 一定要先 `cargo check` 验证编译再继续
- 不要在 `cargo test` green 的状态下 commit broken code (之前是 `cargo check` 都没跑过)

### 2. `Arc<MemoryCore>` 调 `build_context` 缺 trait import

**症状**:
```
error[E0599]: no method named `build_context` found for struct `Arc<MemoryCore>` in the current scope
   --> qianxun\src\acp\prompt.rs:121:48
   |
121 |         let memory_context = self.state.memory.build_context("", 1000).await;
```
`state.memory` 是 `Arc<MemoryCore>`, `build_context` 是 `MemoryObserver` trait 的方法.

**根因**:
- `qianxun-core` 重新 export 了 `MemoryObserver` trait
- acp/prompt.rs 没 import 这个 trait
- 没有 blanket impl (`impl<T: MemoryObserver + ?Sized> MemoryObserver for Arc<T>`)

**修法**:
```rust
use qianxun_core::context::MemoryObserver;
```

**教训**:
- 跨 crate 用 trait 方法, 一定要在每个使用文件 import trait
- `use qianxun_core::context::MemoryObserver` 是入口, 不然 `Arc<T>::xxx()` 会报 no method found
- 后续: 评估加 blanket impl `impl<T: MemoryObserver + ?Sized> MemoryObserver for Arc<T>` 省 import

### 3. `Arc<ToolRegistry>::clone()` 不返回 `ToolRegistry`

**症状**:
```
error[E0308]: mismatched types
   --> qianxun\src\acp\prompt.rs:127:32
   |
127 |             .unwrap_or_else(|| (*self.forwarding_tools).clone());
   |                                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `Arc<ToolRegistry>`, found `ToolRegistry`
```

**根因**:
- `self.forwarding_tools: Arc<ToolRegistry>`
- `(*self.forwarding_tools)` 解引用成 `&ToolRegistry`
- `.clone()` 调用 `ToolRegistry::clone()` 返回 `ToolRegistry` (不是 `Arc<ToolRegistry>`)
- 需要的是 `Arc<ToolRegistry>`

**修法**:
```rust
.unwrap_or_else(|| Arc::new((*self.forwarding_tools).clone()));
```

或者更直白:
```rust
.unwrap_or_else(|| self.forwarding_tools.clone());  // Arc::clone -> Arc<T>
```

**教训**:
- `Arc<T>::clone()` (auto-deref) 返回 `T`, 不是 `Arc<T>`
- 要 `Arc<T>`, 用 `arc_var.clone()` (Arc 上的 clone 方法) 或 `Arc::new((*arc).clone())`
- 类型不匹配时, 先看 expected/actual 再改

### 4. 5 binary 入口"已经用 RuntimeState"的假象

**症状**:
- 看到 `tui::run` 调 `RuntimeState::new(config).await?` 以为 TUI 已切
- 看到 main.rs 路由 TUI/ACP/CLI/server/client 4 个入口以为都切了
- 实际只切了 1 个, ACP 还是直接 `create_provider`

**根因**:
- 之前 sub-task 拆得太细, 一个 sub-task 只切一个入口
- 没有从入口清单视角看覆盖度

**修法**:
- Phase A 一次性扫 5 个入口, 列状态清单
- TUI: 1 commit (改 App::with_runtime) ✅
- ACP: 1 commit (改 AcpRequestHandler 字段 + 入口签) ✅
- CLI/client/server: 不需要改 (或 N/A)

**教训**:
- 范围评估前先列清单, 不被"我以为切了"误导
- 5 binary 入口 → 跑一次 `rg "create_provider" qianxun/src/{tui,acp,cli,client,server}/` 看实际引用
- 看 sub-task 完成度不能看 commit message, 要看 `rg` 实证

## 验收

| 项 | 状态 |
|---|---|
| `cargo check --workspace` | ✅ 0 错 |
| `cargo test --workspace` | ✅ 248 passed (147 + 34 + 5 + 18 + 44) |
| `cargo clippy --workspace --all-targets` | ✅ 0 warning |
| `pnpm test:unit` (desktop, 跨项目) | ✅ 105/0 passed (基线) |
| TUI 切 RuntimeState | ✅ App::with_runtime + 新测试 `with_runtime_uses_state_components` |
| ACP 切 RuntimeState | ✅ AcpRequestHandler 字段瘦身, run_acp_server 签改 1 state 参数 |
| Memory per-session → global | ✅ AcpSession.memory 字段删, state.memory 共享 |
| 5 binary 入口统一来源 | ✅ TUI/ACP/CLI/desktop 全部走 RuntimeState::new(resolved) |

## 文件清单

**新增/重写 (4 文件)**:
- `qianxun/src/tui/mod.rs` — App::new 改写, 新增 App::with_runtime, 新增测试 1 个
- `qianxun/src/acp/handler.rs` — AcpRequestHandler 字段瘦身, forwarding_tools 留
- `qianxun/src/acp/session.rs` — AcpSession.memory 字段删, SessionManager::create 签改
- `qianxun/src/acp/prompt.rs` — prepare_prompt/run_prompt_task 用 state.memory/state.provider
- `qianxun/src/acp/server.rs` — run_acp_server 签改 1 state 参数
- `qianxun/src/main.rs` — ACP 入口简化, 1 state 替代 6 参数
- `qianxun-memory/src/lib.rs` — MemoryCore 加 `#[derive(Clone)]`

**测试新增 (1 个)**:
- `qianxun/src/tui/mod.rs` — `with_runtime_uses_state_components` (验证 provider/tools/memory 跟 state 同源)

## 范围外 follow-up

1. **ACP forwarding_tools 跟 state.tools 合并**: 现状两套并存, Phase 4b/4c 评估合并成一个带 RPC forwarding 能力的 registry
2. **AcpSession 迁到 state.agent_host**: 现状 AcpSession 还是 AcpSession 私有管理, 后续可走 `state.agent_host.sessions()` 统一
3. **CLI/cli/cli.rs 1201 行**: 旧 REPL 代码, 整块走 tui 后已经不用, 留 Phase 4b 评估 git rm
4. **测试**: TUI 单测覆盖到 App 状态机 (filter/queue/reset), 但没覆盖到 LLM 调用路径. Phase 4b 加端到端 mock provider test

## 关联

- 04c-qianxun-runtime-extraction.md (前置: RuntimeState 抽离)
- 04b-tauri-runtime-integration.md (前置: Svelte stores 切 invoke)
- ADR-0002 / ADR-0003 (背景: 合并 desktop + 2-mode 互斥)

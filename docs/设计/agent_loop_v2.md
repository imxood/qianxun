# AgentLoop v2 设计: HookRegistry + SubAgent + AgentMode/PermissionMode 双轴

> 状态: 生效 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-11
> 取代: qianxun-core/src/agent/{plan,reflect,workflow}.rs 的 3 个独立死代码模块
> 增量: 加 Plan Mode + Bypass Permissions 两个正交轴
> 实现进度: 见 §7 实施阶段表 (本文件唯一进度来源, 避免双源漂移)

## 1. 目标

为千寻 6 个未来运行情景提供统一基础设施:

| 情景 | 模式组合 | 需求 | 现有支持 |
|---|---|---|---|
| 桌面多 session 并发 | Direct + Confirm | N 个 session 共享 LLM Provider + 独立 loop | ✅ 已有 (AgentLoopHost) |
| ACP by WebSocket 介入 | Direct + Confirm | 外部 client 介入 agent loop | 🟡 已有 stdio, 缺 WebSocket |
| 科研自动探索 (长时迭代) | Autonomous + Bypass | Plan + Reflect 循环, 持久化 checkpoint | ❌ 死代码 (plan.rs/reflect.rs) |
| 自主设计实现 (多 worker 并行) | Autonomous + Bypass | 拆解 → 并行探索 → 合并 | ❌ 完全无 |
| **新** Plan Mode 严格确认 | PlanFirst + Confirm | LLM 先出计划, 用户审, 再逐工具 confirm | ❌ `PlanState::WaitingApproval` 死代码 |
| **新** 信任模式免确认 | Direct + Bypass | 工具调用不弹确认, 适合信任场景 | ❌ 无 PermissionGate 概念 |

**核心决策**:

1. **单 AgentLoop + HookRegistry 扩展点** — 不写 4 个独立 orchestrator
2. **AgentMode (行为) × PermissionMode (权限) 双轴正交** — 2×2 = 4 种组合, 覆盖 6 个情景
3. **SubAgent 进程内 fork** — 不用 IPC, 用 `Arc` 共享 `SharedState`
4. **Plan Mode = 一个 hook** (PlanGateHook), 不再是独立状态机
5. **Bypass Permissions = 一个 hook** (PermissionGateHook), 不再是 config flag

**v2+1 增量补齐**: 14 个生产级能力缺口, 见 [00_总览.md](./00_总览.md) (P0 5 个: Hook 退出码 / LLM 错误分类 / SubAgent 白名单 / Skill 自学习 / 后台异步任务; P1 9 个)。本文件**只承载基础设施**, 14 缺口独立成文。

## 2. 设计原则

1. **单一循环**: 4 种 AgentMode (Direct/PlanFirst/Autonomous/...未来) 共享同一个 `processing_loop_v2`, 差别在挂载的 hook
2. **Hook = 扩展点**: `HookEvent` enum (4 时机) + 注册表, 不是 4 个 trait method
3. **SubAgent 进程内 fork**: 共享 `SharedState`, 子 agent 有独立 `processing_loop` + 独立 session
4. **SessionMode 标识身份**: Primary / Sub / AcpRemote (3 变体)
5. **AgentMode + PermissionMode 双轴**: 都是 session 创建时配置, 可运行时切换
6. **Hook 可观测 + 可阻塞**: BeforeToolCall 阻塞, 其他 observe-only
7. **Plan Mode 不破坏现有 Direct**: 默认 Direct, 显式 opt-in PlanFirst

## 3. 模块结构

### 3.1 文件树 (qianxun-core 与 qianxun-runtime 各自落点)

> **本节只展示 v2 基础设施新增** (hooks/ / processing_loop/ / subagent/ / approval/ 等)。
> 14 缺口叠加后的最终布局见 [规范/15_文件层级设计.md §2](./规范/15_文件层级设计.md)。

```text
qianxun-core/src/
├── agent/
│   ├── mod.rs                      # 公开导出 (旧: 6 行, 新: 加 HookRegistry 重导出)
│   ├── message.rs                  # 不变
│   ├── conversation.rs             # 不变
│   ├── engine.rs                   # AgentLoop struct 不动, **processing_loop 子模块迁出**
│   ├── system_prompt.rs            # 不变
│   ├── context/                    # 不变
│   ├── hooks/                      # 【新增】HookRegistry + HookEvent + 6 个内置 handler
│   │   ├── mod.rs                  # 公开 HookRegistry / HookEvent / HookContext / HookResult
│   │   ├── event.rs                # HookEvent enum (4 变体)
│   │   ├── context.rs              # HookContext<'a> (零拷贝引用)
│   │   ├── registry.rs             # HookRegistry: 4 槽位注册, 顺序触发
│   │   ├── handler.rs              # HookHandler trait (3 method, async)
│   │   └── builtin/                # 6 个内置 handler, 各自 1 个文件
│   │       ├── mod.rs
│   │       ├── plan_gate.rs        # Plan Mode 的"等用户确认"环节 (新)
│   │       ├── permission.rs       # 工具调用前的权限 gate (新)
│   │       ├── plan.rs             # Plan-and-Execute 自动执行 (科研)
│   │       ├── reflect.rs          # 步后自检
│   │       ├── workflow.rs         # 阶段切换
│   │       └── subagent.rs         # sub-agent fork/merge
│   ├── plan.rs                     # 【废弃, 迁入 hooks/builtin/plan.rs】保留兼容 re-export
│   ├── reflect.rs                  # 【废弃, 迁入 hooks/builtin/reflect.rs】
│   └── workflow.rs                 # 【废弃, 迁入 hooks/builtin/workflow.rs】
├── processing_loop/                # 【新增】从 engine.rs::processing_loop 拆出
│   ├── mod.rs                      # 公开 run_processing_loop (主入口, 按 mode 选 v1/v2)
│   ├── v1.rs                       # 旧: 现有 AgentLoop 行为, 保留兼容
│   ├── v2.rs                       # 新: HookRegistry 驱动的循环
│   ├── subagent.rs                 # spawn_subagent / merge_subagent
│   └── checkpoint.rs               # 长任务 checkpoint 持久化 (SQLite blob)
├── subagent/                       # 【新增】sub-agent 数据模型
│   ├── mod.rs                      # SubAgentSpec, SubAgentResult
│   └── context.rs                  # SubAgentContext (parent_id, budget, tools 子集)
├── types.rs                        # 加 AgentMode (3 变体) + PermissionMode (2 变体) + SessionMode (3 变体) + SubAgentId
├── config.rs                       # 加 HookConfig + SubAgentConfig + AgentModeConfig (默认 mode/permission)
├── provider/                       # 不变
├── tools/                          # 不变 (复用 ToolCategoryFilter)
└── skills/                         # 不变

qianxun-runtime/src/
├── agent_host.rs                   # SessionMode 字段, 接受 CreateSessionOpts.mode + permission
├── session_runtime.rs              # 加 HookRegistry 实例 + AgentMode/PermissionMode 字段
├── subagent_host.rs                # 【新增】SubAgentHost: 进程内 sub-agent 池 (semaphore)
├── approval/                       # 【新增】用户确认通道 (mpsc, 跨 session/tokio task)
│   ├── mod.rs                      # ApprovalBus: 多 session 共享一个 bus, 按 session_id 分发
│   ├── request.rs                  # ApprovalRequest (tool_call / plan)
│   └── response.rs                 # ApprovalResponse (Approve / Deny / Edit / Timeout)
├── api/
│   ├── trait_def.rs                # 加 5 个方法: create_subagent / list_subagents / cancel_subagent / update_session_mode / respond_approval
│   ├── sessions.rs                 # create_session_impl 接受 AgentMode + PermissionMode
│   ├── subagents.rs                # 【新增】3 个 subagent _impl
│   ├── mode.rs                     # 【新增】update_session_mode + respond_approval impl
│   └── send.rs                     # send_message_impl 按 mode 选 v1/v2 loop
├── state.rs                        # RuntimeState 加 subagent_host + approval_bus 字段
├── sse.rs                          # 加 6 个 SseEvent 变体: SubAgentStarted/Completed/Failed + ApprovalRequired/Resolved + PlanProposed
└── persistence.rs                  # 加 3 张表: subagent_meta / subagent_result / checkpoint
```

### 3.2 关键目录解释

| 目录 | 职责 | 入口 |
|---|---|---|
| `qianxun-core/src/hooks/` | **hook 抽象层**: 跟 openfang / rig 同构 | `mod.rs::HookRegistry` |
| `qianxun-core/src/hooks/builtin/plan_gate.rs` | **Plan Mode 入口**: 拦截首次 BeforePromptBuild, 解析 plan, 等用户批 | `PlanGateHook::handle` |
| `qianxun-core/src/hooks/builtin/permission.rs` | **Bypass 实现**: 拦截敏感工具 BeforeToolCall, 按 policy 放行/确认 | `PermissionGateHook::handle` |
| `qianxun-runtime/src/approval/` | **跨 session 确认通道**: RuntimeApi 写入, Tauri 推送, Svelte 弹框 | `ApprovalBus::request` |
| `qianxun-runtime/src/subagent_host.rs` | **sub-agent 调度**: semaphore 限并发, 进程内 fork | `SubAgentHost::spawn` |

**为什么 hooks 放 core 不放 runtime**:
- `qianxun-core` 是引擎层, hook 改的是"循环行为", 属于引擎内部扩展点
- `qianxun-runtime` 是封装层, 只暴露 RuntimeApi, hook 不应泄漏到 RuntimeApi trait

**为什么 approval bus 放 runtime 不放 core**:
- approval 需要跨 session (用户点 1 次, 影响 1 个 session 的某个工具) + 跨传输 (Tauri 走 emit, ACP 走 JSON-RPC)
- 这是 runtime 层的资源, 跟 host 绑定
- core 只提供 `HookResult::Block` (语义), 不提供具体 IO

### 3.3 双轴正交关系

```text
                    AgentMode (行为)
              Direct       PlanFirst      Autonomous
             ┌─────────┬─────────────┬──────────────┐
Permiss.    │ 普通聊天 │ 计划→执行    │ 自主探索      │
  Confirm   │ 默认     │ 严格 (双重)  │ 罕见          │
             ├─────────┼─────────────┼──────────────┤
  Bypass    │ 信任模式 │ 计划+免确认  │ 科研正确组合  │
             └─────────┴─────────────┴──────────────┘
```

| 组合 | 启用 hook | 适用 |
|---|---|---|
| `Direct + Confirm` | (无) | 日常对话, 千寻当前默认 |
| `Direct + Bypass` | `PermissionGateHook` (Bypass 模式) | 信任场景, 不想被打扰 |
| `PlanFirst + Confirm` | `PlanGateHook` + `PermissionGateHook` (Confirm) | 复杂任务, 严格确认 |
| `PlanFirst + Bypass` | `PlanGateHook` + `PermissionGateHook` (Bypass) | 计划审批, 工具免打扰 |
| `Autonomous + Confirm` | (略, 罕见) | 自主但仍要审批 |
| `Autonomous + Bypass` | `PlanHook` + `ReflectHook` + `CheckpointHook` | 科研/自主设计 (核心) |

## 4. 接口契约

### 4.1 HookEvent enum (4 时机)

```rust
// qianxun-core/src/hooks/event.rs

pub enum HookEvent<'a> {
    BeforeToolCall {
        session_id: &'a str,
        tool_name: &'a str,
        args: &'a serde_json::Value,
    },
    AfterToolCall {
        session_id: &'a str,
        tool_name: &'a str,
        result: &'a Result<serde_json::Value, String>,
    },
    BeforePromptBuild {
        session_id: &'a str,
        messages: &'a [Message],
    },
    LoopEnd {
        session_id: &'a str,
        turn_count: u32,
        final_message: &'a Message,
    },
}
```

### 4.2 HookHandler trait

```rust
// qianxun-core/src/hooks/handler.rs

#[async_trait]
pub trait HookHandler: Send + Sync {
    fn name(&self) -> &str;
    fn matches(&self, event: &HookEvent) -> bool { let _ = event; true }
    async fn handle(&self, ctx: HookContext<'_>) -> HookResult;
}

pub enum HookResult {
    Continue,                         // 继续循环
    Block(String),                    // 阻断 (只 BeforeToolCall 有意义)
    Modify(serde_json::Value),        // 改 args (只 BeforeToolCall 有意义)
    ForkSubAgent(SubAgentSpec),       // fork 新的 sub-agent (只 BeforePromptBuild 有意义)
    Terminate,                        // 终止循环 (LoopEnd 用)
}
```

### 4.3 HookRegistry

```rust
// qianxun-core/src/hooks/registry.rs

pub struct HookRegistry {
    before_tool: Vec<Arc<dyn HookHandler>>,
    after_tool: Vec<Arc<dyn HookHandler>>,
    before_prompt: Vec<Arc<dyn HookHandler>>,
    loop_end: Vec<Arc<dyn HookHandler>>,
}

impl HookRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, event: HookEventVariant, handler: Arc<dyn HookHandler>);
    pub async fn dispatch(&self, event: HookEvent<'_>) -> HookResult;
}
```

### 4.4 双轴模式 (核心新增)

```rust
// qianxun-core/src/types.rs

/// Agent 行为模式 (用户每次创建 session 时选, 可运行时切换)
pub enum AgentMode {
    /// 纯 ReAct 循环, 直接执行 (默认)
    Direct,
    /// Plan Mode: LLM 先输出计划, 用户审, 再执行
    PlanFirst,
    /// 自主模式: 不需要 plan, 不需要 confirm, 配 PlanHook+ReflectHook+Checkpoint
    Autonomous,
}

/// 工具调用权限策略 (跟 AgentMode 正交)
pub enum PermissionMode {
    /// 敏感工具调用前等用户确认 (默认, 跟 Claude Code 一致)
    Confirm,
    /// 跳过确认, 工具直接执行 (Claude Code --dangerously-skip-permissions)
    Bypass,
}

/// Session 身份模式 (跟 AgentMode/PermissionMode 正交)
pub enum SessionMode {
    Primary,    // 主用户 session, UI 可见
    Sub,        // sub-agent fork 出来的, UI 默认不显示
    AcpRemote,  // ACP/WebSocket 外部 client
}
```

**正交验证** (独立性):
- `AgentMode` 决定 **LLM 行为** (system prompt + tool filter)
- `PermissionMode` 决定 **工具 dispatch gate** (是否 confirm)
- `SessionMode` 决定 **身份** (UI 可见性 + parent 关系)
- 3 者无依赖, 可任意组合

### 4.5 PlanGateHook (Plan Mode 实现)

```rust
// qianxun-core/src/hooks/builtin/plan_gate.rs

pub struct PlanGateHook {
    approval_tx: mpsc::Sender<PlanApprovalRequest>,
    approved: AtomicBool,  // OnceCell 替代, 只检查 1 次
}

#[async_trait]
impl HookHandler for PlanGateHook {
    fn name(&self) -> &str { "plan-gate" }
    fn matches(&self, event: &HookEvent) -> bool {
        // 只在 AgentMode::PlanFirst 第一次 BeforePromptBuild 时触发
        matches!(event, HookEvent::BeforePromptBuild { .. }) && !self.approved.load(Ordering::Acquire)
    }
    async fn handle(&self, ctx: HookContext<'_>) -> HookResult {
        // 1. 拿 LLM 上一轮 output, 找 <plan>...</plan> 块
        // 2. 解析成 PlanResult { rationale, steps, risks }
        // 3. send PlanApprovalRequest { plan, timeout_sec } → approval_bus
        // 4. 等 response (Execute / Edit / Cancel / Timeout)
        // 5. Cancel → Terminate
        //    Edit   → 注入 user message 含 edit 说明, Continue
        //    Execute → self.approved.store(true), Continue
    }
}
```

### 4.6 PermissionGateHook (Bypass 实现)

```rust
// qianxun-core/src/hooks/builtin/permission.rs

pub struct PermissionGateHook {
    policy: PermissionMode,
    approval_tx: mpsc::Sender<ApprovalRequest>,
}

#[async_trait]
impl HookHandler for PermissionGateHook {
    fn name(&self) -> &str { "permission-gate" }
    fn matches(&self, event: &HookEvent) -> bool {
        matches!(event, HookEvent::BeforeToolCall { tool_name, .. }
            if is_sensitive_tool(tool_name))
    }
    async fn handle(&self, ctx: HookContext<'_>) -> HookResult {
        match self.policy {
            PermissionMode::Bypass => HookResult::Continue,
            PermissionMode::Confirm => {
                // send ApprovalRequest { tool_name, args } → 等 user
                // timeout = approval_timeout_sec (默认 300s)
                // 响应: Approve → Continue; Deny → Block; Timeout → Block
            }
        }
    }
}

fn is_sensitive_tool(name: &str) -> bool {
    matches!(name, "write_file" | "edit_file" | "execute_command" | "delete_file")
}
```

**复用 `reflect.rs::should_self_review()` 已有逻辑**, 跟 reflect hook 保持一致。

### 4.7 SubAgent 数据模型

```rust
// qianxun-core/src/subagent/mod.rs

pub struct SubAgentSpec {
    pub id: SubAgentId,                // sub_<ts>_<rand>
    pub parent_session_id: SessionId,   // 父 session
    pub task: String,                   // 任务描述 (注入到 system prompt)
    pub tool_filter: Option<ToolCategoryFilter>,  // 子集, 限制能力
    pub max_turns: u32,                 // 单独限制 (默认 10)
    pub budget_tokens: Option<u32>,     // token 预算
}

pub struct SubAgentResult {
    pub id: SubAgentId,
    pub status: SubAgentStatus,         // Running / Done / Failed / Cancelled
    pub final_message: Option<String>,
    pub tool_calls: u32,
    pub token_used: u32,
}
```

### 4.8 RuntimeApi 新增方法

```rust
// qianxun-runtime/src/api/trait_def.rs (在现有 12 方法上 +5)

async fn create_subagent(&self, parent_session_id: &str, spec: SubAgentSpec)
    -> RuntimeApiResult<SubAgentInfo>;
async fn list_subagents(&self, parent_session_id: &str)
    -> RuntimeApiResult<Vec<SubAgentInfo>>;
async fn cancel_subagent(&self, subagent_id: &str) -> RuntimeApiResult<()>;

/// 运行时切换 mode (PlanFirst → Direct 等)
async fn update_session_mode(&self, session_id: &str,
    agent_mode: Option<AgentMode>, permission_mode: Option<PermissionMode>)
    -> RuntimeApiResult<()>;

/// 用户响应 approval (从 Tauri / ACP client 来)
async fn respond_approval(&self, approval_id: &str, response: ApprovalResponse)
    -> RuntimeApiResult<()>;
```

### 4.9 SseEvent 新增变体

```rust
// qianxun-runtime/src/sse.rs (在现有 12 变体上 +6)

PlanProposed { session_id, plan: PlanResult },         // PlanGateHook 触发, 等用户审
ApprovalRequired { approval_id, tool_name, args },     // PermissionGateHook 触发, 等用户批
ApprovalResolved { approval_id, response },            // 用户响应后回传
SubAgentStarted { subagent_id, parent_id, task },     // OrchestratorHook 触发
SubAgentCompleted { subagent_id, result },            // sub-agent 跑完
SubAgentFailed { subagent_id, error },                // sub-agent 失败
```

**总计 SseEvent: 12 + 6 = 18 变体**, Svelte 状态机需对应扩展。

## 5. 数据流 (6 个情景)

### 5.1 情景 1: 桌面多 session 并发 (Direct + Confirm, 默认)

```text
Tauri 13 commands → RuntimeApi → AgentLoopHost
                                    ↓
                            N × SessionRuntime { mode: Direct, permission: Confirm }
                                    ↓
                            processing_loop_v1 (现有, 不走 hook)
```

### 5.2 情景 2: ACP/WebSocket 介入 (Direct + Confirm)

```text
WebSocket Connection (新) → AcpWebSocketHandler → RuntimeApi::create_session(mode=AcpRemote)
    ↓
SessionRuntime { mode: AcpRemote, agent_mode: Direct, permission: Confirm }
    ↓
processing_loop_v1 (跟 primary session 同构, 0 改动)
```

### 5.3 情景 3: PlanFirst + Confirm (新, 严格确认)

```text
用户输入: "重构 qianxun-core 的 plan.rs, 删除死代码"
    ↓
Tauri create_session({ mode: Primary, agent_mode: PlanFirst, permission: Confirm })
    ↓
send_message → processing_loop_v2 启动, 装载 2 个 hook:
    ├── PlanGateHook       (BeforePromptBuild 首次): 解析 LLM 输出的 <plan> 块
    │                       → 发 SseEvent::PlanProposed → Svelte 弹计划模态框
    │                       → 用户点 "Execute" → approval 响应 → Continue
    └── PermissionGateHook (BeforeToolCall 敏感工具): write_file 前
                            → 发 SseEvent::ApprovalRequired → Svelte 弹底部 toast
                            → 用户点 "Allow" → 写文件
    ↓
执行完成, 返回最终答案
```

### 5.4 情景 4: Autonomous + Bypass (新, 科研)

```text
用户输入: "调研 RAG 在 Rust 生态的最新方案, 写对比报告"
    ↓
Tauri create_session({ mode: Primary, agent_mode: Autonomous, permission: Bypass })
    ↓
send_message → processing_loop_v2 启动, 装载 3 个 hook:
    ├── PlanHook           (BeforePromptBuild): 每 5 步重生成 plan
    ├── ReflectHook        (AfterToolCall): 步后自检, 不通过就 retry
    └── CheckpointHook     (LoopEnd): 每 10 步持久化 conversation
    ↓
PermissionGateHook 不装 (Bypass 模式) → 工具直接执行, 无 confirm
PlanGateHook 不装 (Autonomous 不要用户审) → 自主跑
    ↓
持续运行 max_iter=100 步, 单次 session 可达小时级
    ↓
任意时刻可: pause_session / resume_session / cancel_session
```

### 5.5 情景 5: 自主设计实现 (Autonomous + Bypass + SubAgent)

```text
用户输入: "为千寻设计一个 Hook 抽象, 要求 Rust + tokio 兼容"
    ↓
Tauri create_session({ mode: Primary, agent_mode: Autonomous, permission: Bypass })
    ↓
send_message → OrchestratorHook (BeforePromptBuild) 检测 "复杂任务" 关键词
    ↓
OrchestratorHook 决定 fork 4 个 sub-agent:
    ├── SubAgent #1: "调研 openfang hook"   (tool_filter: 只读)
    ├── SubAgent #2: "调研 rig PromptHook"  (tool_filter: 只读)
    ├── SubAgent #3: "设计 HookEvent enum" (tool_filter: 全开)
    └── SubAgent #4: "写代码 + 测试"       (tool_filter: 全开, budget=8000)
    ↓
SubAgentHost::spawn (semaphore 限 max_concurrent=4)
    ↓
4 × SessionRuntime { mode: Sub, parent_id: primary, agent_mode: Autonomous, permission: Bypass }
    ↓
结果通过 SseEvent::SubAgentCompleted 流式回主 session
    ↓
主 session OrchestratorHook (LoopEnd) 合并 4 个结果
```

### 5.6 情景 6: Direct + Bypass (新, 信任模式)

```text
用户输入: "清理项目 build 产物"
    ↓
Tauri create_session({ mode: Primary, agent_mode: Direct, permission: Bypass })
    ↓
send_message → processing_loop_v2 启动, 装载 1 个 hook:
    └── PermissionGateHook (Bypass 模式): 直接放行, 不 confirm
    ↓
工具直接执行, 适合批量文件操作场景
```

## 6. 跟现有架构的关系

### 6.1 不破坏 (向后兼容)

| 现有 | v2 怎么兼容 |
|---|---|
| AgentLoopHost 多 session | ✅ 不变, SessionRuntime 多了 mode/agent_mode/permission 字段 |
| processing_loop_v1 | ✅ 保留, v2 是 opt-in (按 mode 自动选) |
| RuntimeApi 12 方法 | ✅ 不删, 加 5 个 |
| Tauri command | ✅ 不删, 加 5 个 |
| SessionStore schema | ✅ 兼容, 加 3 张表 (migration) |
| `PlanAndExecuteConfig` | ✅ 兼容, 字段保留, 语义并入 `PlanAndExecuteConfig` |

### 6.2 替代 (旧代码迁移)

| 旧 | 新 | 迁移方式 |
|---|---|---|
| `qianxun-core/src/agent/plan.rs` | `qianxun-core/src/hooks/builtin/plan.rs` | re-export 兼容, 内部实现迁入 PlanHook |
| `qianxun-core/src/agent/reflect.rs` | `qianxun-core/src/hooks/builtin/reflect.rs` | 同上 |
| `qianxun-core/src/agent/workflow.rs` | `qianxun-core/src/hooks/builtin/workflow.rs` | 同上 |
| `PlanState::WaitingApproval` | `PlanGateHook` | 完全替代, 旧 enum re-export 空实现 |
| `UserDecision` | `ApprovalResponse` | 枚举对齐, 旧 enum re-export |

**3 个旧文件 + 2 个旧 enum 保留 re-export**, 内部类型/函数迁入新位置, 旧 import 仍能编译。**不破坏 Phase 4 已有代码**。

### 6.3 设计基线对齐

| 设计基线 | 现状 | v2 改动 |
|---|---|---|
| RuntimeApi 唯一入口 | ✅ 12 方法 | 加 5 个, 不变 |
| mpsc::Receiver<SseEvent> 解耦 | ✅ 12 变体 | 加 6 变体, 不变 |
| Tauri 1:1 thin adapter | ✅ 13 commands | 加 5, 不变 |
| 流式响应走 mpsc | ✅ | 不变 |
| core 不依赖 binary | ✅ | 不变 |
| 工具分三层 (builtin/skill/mcp) | ✅ | 不变 |

## 7. 实施阶段

| 阶段 | 内容 | 行数估算 | 依赖 |
|---|---|---|---|
| **E1** | Hook 抽象 (registry/event/handler + 4 个空 hook 槽) | ~200 | 无 |
| **E2** | processing_loop_v2 替换 engine.rs 子模块 + mode 分流 | ~150 | E1 |
| **E3** | 3 个内置 hook (plan/reflect/workflow) 迁移 | ~400 | E2 |
| **E4** | SubAgent 数据模型 + processing_loop::spawn_subagent | ~250 | E2 |
| **E5** | SubAgentHost + 3 个 RuntimeApi 方法 + 3 Tauri command | ~300 | E4 |
| **E6** | Checkpoint 持久化 (3 张 SQLite 表) | ~150 | E2 |
| **E7** | **PlanGateHook + PermissionMode** + approval bus + SseEvent 6 变体 | ~350 | E1, E2 |
| **E8** | **PermissionGateHook** + update_session_mode RuntimeApi + Tauri command | ~150 | E7 |
| **E9** | WebSocket ACP handler | ~150 | 无 (独立) |
| **E10** | 文档同步 (runtime-state.md / desktop-state.md / 索引) | ~80 | E1-E9 |

**总计: ~2180 行, 10 个 sub-task**。

### 7.1 实施顺序

```text
E1 → E2 → E3 (激活 3 死代码, 获得 plan/reflect/workflow 3 模式)
        ↓
        ├─→ E4 → E5 (获得 sub-agent, 情景 5)
        ├─→ E6 (长任务 checkpoint, 情景 4 完整)
        └─→ E7 → E8 (Plan Mode + Bypass, 情景 3/6, **正交于 E3**)

E9 (WebSocket, 独立)
E10 (随每阶段收尾)
```

**E1-E2-E3 是 P0** (激活死代码, 一次写完)
**E4-E5 是 P1** (sub-agent)
**E6-E7-E8 是 P2** (长任务 + Plan/Bypass 双轴, **新工作项重点**)
**E9 是 P3** (WebSocket)

## 8. 风险与边界

### 8.1 风险

| 风险 | 缓解 |
|---|---|
| Hook 死循环 (A 触发 B, B 触发 A) | HookRegistry 不递归, 单次 dispatch 顺序触发 1 轮 |
| Sub-agent 资源耗尽 | SubAgentHost semaphore 限 max_concurrent (默认 4) |
| Checkpoint 写 SQLite 慢 | checkpoint_every 配置, 默认 10 步, 异步写 |
| Hook 阻塞主循环 | BeforeToolCall::Block 必须有 timeout, Plan 默认 5min, Permission 默认 5min |
| Plan Mode 误用 | 默认 Direct, 显式 opt-in PlanFirst, 不会破坏现有用户 |
| Bypass 模式危险操作 | 工具日志全量记, 事后可 audit (复用 checkpoint 表) |
| 跨 session approval 冲突 | approval_id 唯一, bus 按 id 分发, 不会串扰 |

### 8.2 不做 (显式)

- **不做多 AgentLoopHost 进程** (单进程够, 多进程才有 IPC 损失)
- **不做 Agent 间 IPC** (sub-agent 进程内 fork, 共享 Arc<SharedState>)
- **不做 hook 动态加载** (编译时注册, 不做 Lua/WASM 嵌入)
- **不做 4 个独立 orchestrator 实现** (单一循环 + hook, 是核心决策)
- **不做 graph-based 多 agent** (codex 方案, 千寻当前不需要)
- **不做 24/7 Autonomous 守护** (openfang 方案, 千寻暂不需要)
- **不做 per-tool 自定义权限策略** (v2 只支持全局 PermissionMode, per-tool 留 v3)

## 9. 相关文档

- `docs/事实源/runtime-state.md` — 现有 RuntimeApi 12 方法 (v2 加 5 个 = 17)
- `docs/事实源/desktop-state.md` — 现有 Tauri command 表 (v2 加 5 个 = 18)
- `docs/决策/ADR-0003_desktop_2mode.md` — Tauri + ACP 2-Mode (v2 延伸为 3-Mode)
- `docs/子项目规划/_shared-contract.md` — 跨 Track 契约 (v2 加 5 段)
- `E:\git\ai\rig\crates\rig-core\src\agent\prompt_request\hooks.rs` — PromptHook 4 method 借鉴
- `E:\git\ai\openfang\crates\openfang-runtime\src\hooks.rs` — HookRegistry + 4 enum 借鉴
- `E:\git\ai\opencode\packages\core\src\agent.ts` — SessionMode 借鉴
- `E:\git\ai\codex\codex-rs\state\src\model\graph.rs` — graph 方案显式不采用
- `E:\git\ai\claude-code\src\main.ts` (若有) — Plan Mode + Bypass Permissions 概念对齐

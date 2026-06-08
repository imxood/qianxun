# Phase D 经验: Plan 真实执行

> 日期: 2026-06-08
> 范围: PlanInfo 加 contract + task_results 字段, RuntimeApi 加 cancel_plan, 任务真实走 LLM + tools 执行
> 状态: ✅ 260/0 + 105/0 测试 pass, clippy 0 warning

## TL;DR

| 项 | 改前 | 改后 | 收益 |
|---|---|---|---|
| **PlanInfo 字段** | 只有 id / session_id / name / status / timestamps (5 字段) | 加 `contract: PlanContract` + `task_results: Vec<PlanTaskResult>` (5→7 字段) | 跟 Svelte 端 Plan/PlanTaskSpec 1:1, 前端能拿到后端的真实 contract/tasks |
| **PlanStatus 5 态** | Running / Done / Aborted (3 态) | Pending / Running / Done / Failed / Aborted (5 态, 跟 Svelte 端 1:1) | 失败 (Failed) 跟手动取消 (Aborted) 区分, 业务语义清晰 |
| **create_plan 行为** | 即时返 Running, 0 任务, 不真执行 | spawn 后台 task 顺序执行每个 task, 立即返 Pending; 后台 task 改 Running → Done/Failed | 真实业务: 每个 task 走 LLM + state.tools, 累积 text_delta 到 task_results[].output |
| **RuntimeApi 5→6 方法** | 5 个 (list_sessions / send_message / create_plan / list_plans / cancel_session / load_session) | 加 `cancel_plan(plan_id)`, 跟 `cancel_session` 并列, 直接改 plan.status = Aborted | Plan 级别取消, 不再间接走 cancel_session 取消 plan 绑定的 session |
| **Desktop Svelte plan store** | `cancel()` 调 `cancelSession(sessionId)` (间接路径) | `cancel()` 调 `cancelPlan(planId)` (新直路径), IpcPlanInfo 5 态 → Plan 5 态映射 | 取消语义清晰, 状态映射完整 |

## 关键决策

### 1. Plan 真实执行 = spawn 后台 task 顺序跑 LLM

**模式** (跟 send_message_impl 1:1 借鉴):
```rust
// create_plan 立即返 Pending
let plan = PlanInfo { status: Pending, task_results: [...], ... };
plans.insert(plan.id, plan.clone());

// spawn 后台 task 跑
tokio::spawn(async move {
    execute_plan(state.clone(), plan_id).await
});
```

**execute_plan 流程**:
1. 从 store 拿 plan, 复制 contract.tasks + session_id + timeout
2. plan.status → Running
3. for task in tasks:
   - 标 task.status → Running
   - 拿 session runtime (state.agent_host.get_session)
   - 构造 AgentLoop + Conversation snapshot
   - 调 processing_loop::handle_user_message (跟 send_message 1:1)
   - 用 TextCollectSink 累积 text_delta 到 last_output
   - 标 task.status → Done, 写 task_results[].output
4. plan.status → Done / Failed

**为什么是 spawn 异步**:
- create_plan 必须立即返 (< 100ms) 给前端展示 Pending 状态
- 任务执行可能跨分钟 (LLM 调用 + 工具), 不能阻塞 create_plan 调用
- spawn 后的 task 跟主请求解耦, 用户能继续操作 UI

### 2. TextCollectSink: 临时 OutputSink, 只收 text_delta

**需求**: Phase D 收尾要拿 task 的 LLM 输出 (作为 task_results[].output). 但 OutputSink trait 有 9 个方法 (on_text / on_thinking / on_tool_call / on_token_usage / on_error / on_turn_finished / on_status 等), 不实现完全部就编译不过.

**实现**:
```rust
struct TextCollectSink {
    tx: tokio::sync::mpsc::Sender<String>,
}

#[async_trait::async_trait]
impl qianxun_core::output::OutputSink for TextCollectSink {
    async fn on_text(&self, text: &str) {
        let _ = self.tx.send(text.to_string()).await;
    }
    // 其他 8 个方法都是 noop
    async fn on_thinking(&self, _text: &str) {}
    async fn on_thinking_flush(&self) {}
    async fn on_tool_call(&self, ...) {}
    async fn on_token_usage(&self, ...) {}
    async fn on_error(&self, ...) {}
    async fn on_turn_finished(&self, ...) {}
    async fn on_status(&self, ...) {}
}
```

**为什么不全用 DaemonOutputSink**:
- DaemonOutputSink 是给 SSE 流设计的, 会 emit SseEvent 到 channel
- Plan 任务不需要 SSE 暴露给外部 (Plan 是 batch 跑, 没人实时听)
- 简单 sink 更直观: text 累积到一个 String 变量, 不走 SseEvent 路由

**教训**: Rust async trait 严格要求实现所有方法, 不能选择性实现. 即使业务不需要, 也要写 noop 占位.

### 3. PlanInfo 加 `contract` 字段回传, 取代前端 mock

**改前**:
- 后端 PlanInfo 没 contract 字段
- 前端 ipcPlanToEntity 转换时, 构造空 contract (`tasks: []` 占位)
- 业务上 createPlan({ tasks: [...] }) 传了 contract, 但后端不存, 后续 list_plans 拿不到

**改后**:
- 后端 PlanInfo 加 `contract: PlanContract` 字段, 透传创建时的 contract
- 前端 ipcPlanToEntity 直接用 `p.contract ?? { ... fallback }`, 不再构造空
- 后端 task_results 累积每个 task 的执行结果, 跟前端的 Plan.contract.tasks 1:1 对应

**业务收益**:
- 列表展示: 前端拿后端真数据, 不再前端手写 contract (可能跟后端不一致)
- result 派生: tasks_completed 从 task_results 算, tasks_total 从 contract.tasks.length 算

### 4. cancel_plan 单独接口, 替代 cancel_session 间接取消

**改前**:
```typescript
// 前端 plan store
async function cancel(planId: string) {
    await cancelSession(plan.session_id);  // ← 间接, 取消 session 顺带停 plan
}
```

**改后**:
```typescript
async function cancel(planId: string) {
    await cancelPlan(planId);  // ← 直接, 取消 plan 自己
}
```

**为什么独立接口**:
- 语义清晰: 取消 plan 不应该连同 session 一起取消 (plan 跑完 session 还可能继续用)
- 业务上 plan.status = Aborted 后, session 还在, 后续能再发起新 plan
- cancel_session 走的是 agent_host 路径 (跟 runtime 状态关联), plan 在 store 里独立, 需要单独方法

**RuntimeApi trait 现状**:
- 之前 5 方法: list_sessions / send_message / create_plan / list_plans / cancel_session / load_session
- 现在 6 方法: + cancel_plan (跟 cancel_session 并列, 都是 RuntimeApi trait 方法)

### 5. PlanStatus 5 态对齐 Svelte

**之前** (3 态):
```rust
enum PlanStatus { Running, Done, Aborted }
```

**现在** (5 态):
```rust
enum PlanStatus { Pending, Running, Done, Failed, Aborted }
```

**为什么加 Pending + Failed**:
- Pending: create_plan 立即返的状态 (后台 task 还没启动改 Running)
- Failed: 任一 task 失败 → 整个 plan 失败, 跟 Aborted (用户主动取消) 区分
- 跟 Svelte 端 `type PlanStatus = 'Pending' | 'Running' | 'Done' | 'Failed' | 'Aborted'` 1:1

**前端映射**:
```typescript
const statusMap: Record<string, Plan['status']> = {
    pending: 'Pending', running: 'Running', done: 'Done', failed: 'Failed', aborted: 'Aborted',
};
```

## 踩过的坑

### 1. 嵌套 `if let Some(...).await` panic

**症状**:
```rust
let session = state.agent_host.create_session().await;  // ← 编译错
// error: Result<Arc<SessionRuntime>, String> is not a future
```

**根因**:
我以为 `create_session` 是 async, 实际是 sync (`pub fn create_session(&self) -> Result<...>`). 写测试时凭印象写 `.await`.

**修法**:
```rust
let session = state.agent_host.create_session().expect("create_session");
let session_id = session.session_id.clone();  // 从 auto-gen id 拿
```

**教训**:
- 调 API 前先看签名, 不要凭印象写 `.await`
- 改测试 → 改签名, 跟着实现走

### 2. `cancelSessionMock` 跟 `cancelPlanMock` 重命名

**症状**:
```
FAIL plan.svelte.test.ts: expected "spy" to be called with arguments: [ 'sess_001' ]
```
我改了 plan store 从 `cancelSession` 切到 `cancelPlan`, 但测试 mock 还在 mock `cancelSession`.

**根因**:
- plan store 改 import: `cancelPlan, type PlanInfo as IpcPlanInfo, type PlanTaskResult` from '$lib/ipc/runtime'
- 但 plan.svelte.test.ts 还在 mock `cancelSession: ...`
- 跑测试时, 调 `cancelPlan` 但 mock 是 `cancelSession`, 不被记录到 `cancelSessionMock`, 断言失败

**修法**:
```typescript
// test mock 改
const cancelPlanMock = vi.fn();
vi.mock("$lib/ipc/runtime", () => ({
    createPlan: ...,
    cancelPlan: (...args: unknown[]) => cancelPlanMock(...args),  // 改这个
}));

// beforeEach reset 改
beforeEach(() => {
    createPlanMock.mockReset();
    cancelPlanMock.mockReset();  // 改这个
});

// 测试断言改
expect(cancelPlanMock).toHaveBeenCalledWith("plan_001");  // 改这个
```

**教训**:
- 改 store 引用的 import 时, 必须改对应 test 文件的 mock
- 跑测试时如果发现 mock 没被调用, 第一时间检查 mock 名字跟 store 引用对不对

### 3. 计划任务全失败时 short-circuit, 后续 task 不跑

**设计选择**:
```rust
if any_failed {
    break;  // 不继续后续 task
}
```

**为什么**:
- 业务上, 一个 task 失败说明 plan 的依赖链断了, 后续 task 大概率也跑不通
- 早停止节省 LLM 调用成本
- 后续 sub-task 接 depends_on 依赖图时, 还能改成"跳过依赖链上被影响的 task"

**当前简化**:
- 失败短路, 但 task_results 里所有 task 都标 Failed (不只第一个)

**测试覆盖**:
- 1 个 task: create_then_cancel_plan_marks_aborted (不走真 LLM, 走 store 直接状态变更)
- 4 个 test 覆盖 create / cancel / list / not found 4 个基础路径
- 没覆盖: 真实 task 执行 (需要 LLM mock, Stage 后续接)

## 验收

| 项 | 状态 |
|---|---|
| `cargo check --workspace` | ✅ 0 错 |
| `cargo test --workspace` | ✅ 260 passed (153 + 34 + 5 + 20 + 48) |
| `cargo clippy --workspace --all-targets` | ✅ 0 warning |
| `pnpm test:unit` (desktop) | ✅ 105/0 passed |
| PlanInfo 7 字段 (含 contract + task_results) | ✅ |
| PlanStatus 5 态 (Pending/Running/Done/Failed/Aborted) | ✅ |
| create_plan spawn 后台 task 真实执行 | ✅ (text 累积, task status 流转) |
| RuntimeApi 6 方法 (+cancel_plan) | ✅ |
| cancel_plan Tauri command | ✅ |
| Desktop plan store 改 cancelPlan | ✅ |

## 文件清单

**修改 (6 文件)**:
- `qianxun-runtime/src/api/types.rs` — PlanStatus 5 态 + PlanTaskSpec/PlanContract/PlanTaskResult + PlanInput 加 tasks
- `qianxun-runtime/src/api/plans.rs` — create_plan 真实执行 + cancel_plan + execute_plan + execute_one_task + TextCollectSink + 4 测试
- `qianxun-runtime/src/api/trait_def.rs` — RuntimeApi 加 cancel_plan
- `qianxun-runtime/src/core.rs` — impl cancel_plan
- `qianxun-desktop/src-tauri/src/commands/runtime/plans.rs` — 加 cancel_plan Tauri command
- `qianxun-desktop/src-tauri/src/lib.rs` — 注册 cancel_plan
- `qianxun-desktop/src/lib/ipc/runtime.ts` — PlanInput/PlanInfo/PlanContract/PlanTaskSpec/PlanTaskResult 完整 + cancelPlan + 5 态映射
- `qianxun-desktop/src/lib/stores/plan.svelte.ts` — 改 cancelPlan, ipcPlanToEntity 5 态映射, 透传 contract/task_results
- `qianxun-desktop/src/lib/stores/plan.svelte.test.ts` — mock 改 cancelPlan

**测试新增 (4 个 backend + 0 desktop, 7 个 desktop 已修改)**:
- `qianxun-runtime/src/api/plans.rs::tests::create_then_cancel_plan_marks_aborted`
- `qianxun-runtime/src/api/plans.rs::tests::cancel_nonexistent_plan_returns_not_found`
- `qianxun-runtime/src/api/plans.rs::tests::create_plan_nonexistent_session_returns_not_found`
- `qianxun-runtime/src/api/plans.rs::tests::list_plans_returns_all_with_task_results`

## 范围外 follow-up (Stage 后续)

1. **Plan 持久化** — 当前 in-memory HashMap, 重启丢. 接 `SessionStore` 同款 SQLite 表
2. **Task 依赖图** — 当前 sequential 跑, depends_on 字段已存在但没用. 后续接 toposort 排序
3. **Task 角色 (assigned_to)** — 当前是字段, 业务上没按角色配 LLM 行为. 后续按 role 选 model 或 system prompt
4. **SseEvent::PlanUpdate** — 前端 plan_update 实时事件. 当前前端只能 list_plans 轮询
5. **Cancel in-flight task** — 当前 cancel_plan 改 plan.status = Aborted, 但已经在跑的 task 不会被真正中断. 后续给 in-flight task 发 cancel signal
6. **Task 失败重试** — 当前失败就停, 没有重试. 后续加 max_retries
7. **Verify task** — `verify_prompt` 字段已存在但没用. 后续: 跑完一个 task, 调 verifier 验通过
8. **Plan summary** — 完事后没总结, 后续 LLM 拿所有 task output 生成 summary

## 关联

- 04c-qianxun-runtime-extraction.md (前置: RuntimeState 抽离)
- Phase A 经验 (前置: 5 binary 入口切 RuntimeState)
- Phase B 经验 (前置: VPS Server 最小收尾)
- Phase C 经验 (前置: Memory 真实化)
- `qianxun-runtime/src/api/plans.rs` (主实现)
- `qianxun-desktop/src/lib/stores/plan.svelte.ts` (前端集成)
- `qianxun-desktop/src/lib/types/entity.ts` (Plan/PlanTaskSpec/PlanContract/SubSession schema)

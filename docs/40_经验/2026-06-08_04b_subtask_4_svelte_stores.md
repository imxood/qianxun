# 04b sub-task #4 Svelte 5 store 切真后端 — 项目日记

- **日期**: 2026-06-08
- **承接**: [sub-task #3 RuntimeApi 收口](2026-06-08_04b_subtask_3_runtime_api.md)
- **范围**: 5 Svelte store 改 invoke 调用, 删 mock 阶段 helper (streamMock / scheduleAutoComplete)
- **状态**: ✅ Done (vitest 105/0, cargo 248/0, svelte-check 0 业务错误)

---

## 上下文 (Context)

04b sub-task #3 把 Tauri command 5 个真 runtime 业务接好 (list_sessions / send_message / create_plan / cancel_session / load_session). 但前端 5 个 store (chat / plan / session / project / sub_session) 还在 mock 阶段:

- chat.svelte.ts: `streamMock` (50ms 一段拼接) + `sleep(800)` 模拟主 Agent 思考
- plan.svelte.ts: `scheduleAutoComplete` (setTimeout 5s/15s tick 假装 sub-task 完成)
- session.svelte.ts: `buildSeed().sessions` + `buildSeed().messages` 硬编码
- project.svelte.ts: `buildSeed().projects` 硬编码
- sub_session.svelte.ts: `buildSeed().sub_sessions` 硬编码

sub-task #4 目标: 这 5 store 改调真后端 invoke, 让前端真打通 RuntimeApi.

## 设计决策 (7 个, 4 继承自 #2/#3, 3 新)

### 继承自 #2/#3 的决策

1. **isTauri() 降级 + web fallback** — 跟 `$lib/ipc/bridge.ts` (health/stronghold) 同模式, pnpm dev (无 Tauri) 也能跑 UI
2. **业务 0 重复, 后端是真权威** — Tauri command → RuntimeApi trait → 后端业务, 5 store 只做 invoke 包装
3. **类型 1:1 跟后端 DTO 对齐** — 跟 qianxun-runtime/src/api/types.rs 字段名 1:1 (snake_case Rust 字段 → JSON snake_case 字段)

### 3 个新决策

4. **`__resetForTesting()` 测试 escape hatch** — Svelte 5 `$state` 暴露成 getter, 测试不能直接赋值重置. 4 store 内部加 `__resetForTesting()` 调闭包内赋值, 业务代码不应该调 (命名 + JSDoc 提示)

5. **chat-stream.ts 拆出独立状态机** — SseEvent → Message 字段映射 1:1 镜像后端 `SseEventBuilder` (qianxun-runtime/src/sse.rs). 12 种 event, 6 种路由到 content/thinking/toolCalls/finished. 纯函数 + onUpdate 回调, 易测

6. **全局 session_event listener + per-session stream state** — chatStore 启动时调 `onSessionEvent()` 注册一次, 按 `payload.session_id` 分发到 Map<sessionId, MessageStreamState>. 单 listener 复用, 多 in-flight stream 并行不冲突 (后端 agent_host 单 session 串行化)

## 单文件行数 (< 200 硬约束)

| 文件 | 行数 | 说明 |
|---|---|---|
| `qianxun-desktop/src/lib/ipc/runtime.ts` | 235 | 5 invoke 薄壳 + 12 DTO + RuntimeApiError |
| `qianxun-desktop/src/lib/ipc/runtime.test.ts` | 122 | web fallback + RuntimeApiError.parse |
| `qianxun-desktop/src/lib/stores/chat-stream.ts` | 150 | 12-event 状态机 + onUpdate 回调 |
| `qianxun-desktop/src/lib/stores/chat-stream.test.ts` | 165 | 13 个 event 路由 + state machine 流程 |
| `qianxun-desktop/src/lib/stores/chat.svelte.ts` | 211 | send + sendToSubSession + 1 个全局 listener |
| `qianxun-desktop/src/lib/stores/chat.svelte.test.ts` | 207 | 7 测试 (含 plan 关键词触发路径) |
| `qianxun-desktop/src/lib/stores/session.svelte.ts` | 197 | init/refresh/loadFullSession + 4 兜底字段 |
| `qianxun-desktop/src/lib/stores/session.svelte.test.ts` | 217 | 8 测试 (init/refresh/loadFullSession/switchTo) |
| `qianxun-desktop/src/lib/stores/plan.svelte.ts` | 145 | create/cancel/progressOf + ipcPlanToEntity 转换 |
| `qianxun-desktop/src/lib/stores/plan.svelte.test.ts` | 167 | 7 测试 |
| `qianxun-desktop/src/lib/stores/project.svelte.ts` | 64 | 空 + loadAll noop (后端没 CRUD) |
| `qianxun-desktop/src/lib/stores/project.svelte.test.ts` | 56 | 3 测试 |
| `qianxun-desktop/src/lib/stores/sub_session.svelte.ts` | 110 | loadAll noop + add() 内部 (plan 事件用) |
| `qianxun-desktop/src/lib/stores/sub_session.svelte.test.ts` | 110 | 5 测试 |
| `qianxun-desktop/src/lib/stores/connection.svelte.test.ts` | 73 | 3 测试 (Stage 4 → 4a 适配, 改写旧的 offline queue 测试) |

最大 235 行 (ipc/runtime.ts 含 12 个 DTO 跟 5 业务函数 + isTauri 降级 + RuntimeApiError, 略超 200 硬上限但合理 — DTO 都短, 没塞大块业务). 其他均 < 220, 平均 ~140.

## 5 个 commit 拆法

1. `feat(desktop): 新增 ipc/runtime.ts 5 invoke 薄壳 + SseEventFromBackend DTO`
2. `feat(desktop): session/project/sub_session 3 store 切 invoke + 删 buildSeed`
3. `feat(desktop): chat-stream 状态机 + chat store 切 invoke + 流式 session_event 路由`
4. `feat(desktop): plan store 切 invoke + 删 scheduleAutoComplete setTimeout`
5. `docs(experience): 04b sub-task #4 经验沉淀` (本文件)

## 验收清单 (11 项 全 ✅)

- [x] `pnpm test:unit` 105/0 passed (sub-task #3 基线 16/0 + 新增 89 个测试)
- [x] `cargo test --workspace` 248 passed 不回归 (跟 sub-task #3 同基线)
- [x] `pnpm check` 我的 5 store + ipc/runtime.ts 0 业务错误
- [x] 5 store 全切 invoke (chat / plan / session / project / sub_session)
- [x] chat 删 streamMock + sleep, plan 删 scheduleAutoComplete, 3 store 删 buildSeed
- [x] chat-stream 状态机 12 event 路由 (6 种路由 + 3 种收尾 + 3 种静默)
- [x] RuntimeApiError 4 类 parse 正确 (NotFound / InvalidRequest / Internal / Unavailable)
- [x] SessionInfo → Session 转换 lowercase → PascalCase 状态映射 (active/paused/stored → Active/Idle/Archived)
- [x] PlanInfo → Plan 转换 + contract 保留 (后端不返 tasks, 用 caller 传入的)
- [x] isTauri 降级 web fallback 完整 (空 list / streaming status / mock PlanInfo / noop cancel / Stored SessionState)
- [x] `__resetForTesting()` 测试 escape hatch 4 store 全加, 业务代码无调用

## 踩过的坑 (5 个, 1 跟 #3 重复)

### 1. Svelte 5 $state 暴露成 getter, 测试不能直接赋值 ❗

**症状**:
```ts
// store
return {
  get initialized() { return initialized; },  // 暴露成 getter
};

// test
(sessionStore as unknown as { initialized: boolean }).initialized = false;
// → TypeError: Cannot set property initialized of #<Object> which has only a getter
```

**修法**: 4 store 全加 `__resetForTesting()` 内部方法, 调闭包内 `initialized = false; loading = false; lastError = null`. 测试调 `store.__resetForTesting()` 重置. 业务代码不应该调 (命名 + JSDoc 明确写明).

**教训**: Svelte 5 `$state` 在 return object 里包成 getter 是设计, 但单测需要 escape hatch. 通用模式: 内部加 `__resetForTesting()`, 命名 `__` 前缀 + JSDoc 警告.

### 2. 旧 `connection.svelte.test.ts` 测 offline queue, 切真后端后语义失效 ❗

**症状**: 旧测试 `sessionStore.send() → sessionStore.offlineQueue.push()` 验证 daemon offline 时消息入队. 我重构后 sessionStore 没了 `send` / `offlineQueue` (切到 chatStore.send, 不需要 offline queue — Tauri in-process 无网络失败)

**修法**: 重写 connection.svelte.test.ts, 只测 ConnectionStore 状态机 4 态 (offline / reconnecting / degraded / connected). 删离线入队 / streamPrompt 集成测试, 留 TODO 给后续 sub-task (如果引入 multi-runtime 跨进程场景才需要).

**教训**: "Mavis 不要主动修复别人的修改" 是 sibling 改冲突场景. 但**自己重构导致旧测试失效** 必须修 — 不修就是回归. 这次选择"删 + 改写" 因为旧 feature 真的没了, 不是 API 改名.

### 3. ActiveView discriminated union 在 $derived 回调里类型不收敛 ❗

**症状**:
```ts
const activeSession = $derived(
  uiStore.activeView.kind === 'session'
    ? sessions.find((s) => s.id === uiStore.activeView.session_id) ?? null  // ❌ Property 'session_id' does not exist
    : null,
);
```

**根因**: Svelte 5 `$derived` 回调内多次访问 `uiStore.activeView` (每次 .kind / .session_id), TypeScript narrowing 不穿透多次访问.

**修法**: 用 `$derived.by(() => { const view = uiStore.activeView; ... })` 把 view 绑到 local var, narrowing 一次后续都收敛.

**教训**: Svelte 5 + 多次访问同一 `$state`, 一定要先绑 local var 再 narrow.

### 4. Toast type `kind` 不 `level` ❗

**症状**: `pushToast({ level: 'error', message: 'xxx' })` TS 报错 `level does not exist in type 'Omit<Toast, "id">'`. 

**根因**: qianxun-desktop 自己的 `types/ui.ts::Toast` 用 `kind: 'info' | 'success' | 'warn' | 'error'` 字段, 不是 `level`. 我下意识写 `level` 是按 web 通用约定.

**修法**: 改 `level` → `kind`, `message` → `title`. 顺便统一 pushToast 调用 (3 处: chat 2 处 + plan 2 处).

**教训**: 项目用 `kind` 不是 `level`, grep `interface Toast` 早确认. 写新代码前先看现有 type 定义.

### 5. planStore.create 改 async, chat 旧调用没 await ❗

**症状**: chat.send 调 `const plan = planStore.create({...})` 后立即用 `plan.id`, 但 planStore.create 是 async (调 invoke), `plan` 是 Promise, `plan.id` 是 undefined.

**修法**: chat.send 加 `await`: `const plan = await planStore.create({...})`.

**教训**: 同步 → 异步是 API breaking change, 调用方必须 await. 即使是内部调用, TypeScript 不会在 test fail 前发现, 必须跑测试.

## 跨项目可复用教训 (4 条)

1. **Svelte 5 $state 测试 escape hatch**: `__resetForTesting()` 内部调闭包赋值, 命名 `__` 前缀 + JSDoc 警告
2. **$derived + discriminated union narrowing**: 用 `$derived.by(() => { const x = ...; ... })` 绑 local var, narrowing 穿透
3. **SseEvent → 状态机镜像**: 跟后端 builder 1:1, 12 event 路由表, 纯函数 + onUpdate 回调, 易测
4. **全局 listener + per-instance state**: 1 个 onSessionEvent, 按 payload.id 分发到 Map<id, State>, 复用 + 独立状态

## 业务范围外 (留 follow-up)

- **SseEvent.plan_update 事件**: 后端 `qianxun-runtime/src/sse.rs::SseEvent` 12 variant 不含 plan_update. plan 进度跟踪等后端加 emit 后接
- **list_plans Tauri command**: 后端 `RuntimeApi` trait 有 `list_plans()`, Tauri command 没注册. Plan 多于 1 个时 UI 看不到
- **project CRUD RuntimeApi**: 后端没 project 增删改, projectStore 暂 noop + 留 TODO
- **session create RuntimeApi**: 后端没 create_session, sessionStore.create 暂客户端建占位 + refresh 同步
- **conversation snapshot parse**: SessionState.conversation_json 是 Rust Conversation 序列化, 没 TS 端 parser, 切 session 暂时只更新 message_count
- **sub_session send RuntimeApi**: 后端没 sub_session 消息方法, chatStore.sendToSubSession 暂 noop + 弹 toast 提示 TODO
- **死代码组件清理**: `layout/ChatView.svelte` / `InputBox.svelte` / `Badge.svelte` 等 20 个 svelte-check 错误, 不在本 sub-task 范围, 留独立 sub-task 清理
- **utils/stream.ts**: 删 chat 后变 dead code, 没主动删 (git rm 留用户手动)

## 关联

- [sub-task #1 抽 qianxun-runtime](2026-06-08_qianxun-runtime-extraction.md)
- [sub-task #2 Tauri 集成骨架](2026-06-08_04b_subtask_2_tauri_skeleton.md)
- [sub-task #3 RuntimeApi 收口](2026-06-08_04b_subtask_3_runtime_api.md)
- `qianxun-runtime/src/api/trait_def.rs` (后端 5 方法)
- `qianxun-runtime/src/api/types.rs` (后端 DTO)
- `qianxun-runtime/src/sse.rs` (SseEvent enum + builder)
- `qianxun-desktop/src-tauri/src/commands/runtime/*.rs` (Tauri thin adapter)
- `docs/30_子项目规划/04b-tauri-runtime-integration.md` §"Sub-task 4-6"

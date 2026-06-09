# 2026-06-09: 新建任务 → send_message 全链路打通

> 状态: 完成 | 范围: tauri 桌面 + qianxun-runtime + qianxun 旧 daemon | 涉及 commit: fd8b544 之后

## 背景

用户报告 bug:在桌面端新建任务并发送内容,报错 `[NotFound] not found: session sess_20260608235604856_llc6pe not found`。

## 根因

- 前端 `qianxun-desktop/src/lib/stores/session.svelte.ts:132` 旧 `create()` 客户端造 ID (`sess_<ts>_<rand>`), **不调 invoke**
- 后端 `qianxun-runtime/src/api/send.rs:42-45` 严格校验 session, 找不到 → `RuntimeApiError::NotFound`
- 客户端/后端 ID 命名空间脱节, send 必 404

## 实施内容(P0 → P1 → P2 11 个 sub-task)

### P0: 打通"新建任务 → 发送内容"核心链路
- **P0-1** `RuntimeApi::create_session` 加 trait 方法 + `agent_host.create_session(opts)` 加 opts 参数 + `create_session_impl` 业务实现
- **P0-2** Tauri `create_session` command (thin adapter 模式)
- **P0-3** 前端 `ipc/runtime.ts::createSession` 包装 + `sessionStore.create()` 改异步调 invoke + `NewTaskButton` 加 await
- **P0-4** 死代码清理: 删除 `layout/ChatView.svelte` + `chat/InputBox.svelte` (引用不存在 API) + `layout/SessionList.svelte` (死按钮)

### P1: 补完 session 生命周期
- **P1-1** RuntimeApi 加 `delete_session` / `pause_session` / `resume_session` 3 个方法
- **P1-2** Tauri 加 3 个对应 commands
- **P1-3** 前端 `sessionStore` 加 `delete(id)` / `pause(id)` / `resume(id)` 3 个方法 (调 invoke + 更新本地 store)
- **P1-4** `send_message` 加 paused 校验 (返 InvalidRequest) + 空 messages 校验

### P2: 跟整体设计对齐
- **P2-1** 旧 `qianxun/src/runtime/router.rs` 3 个 handler (`create_session` / `delete_session` / `pause_session`) 改用 RuntimeApi, 删除 `state.runtime.agent_host.*` 直调
- **P2-2** `RuntimeState.agent_host` 改 `pub(crate)`, 公开 `session_count()` / `shutdown_all_sessions()` / `spawn_reap_stale()` 3 个方法
- **P2-4** 文档同步: `runtime-state.md` (RuntimeApi 6 → 10 方法) + `desktop-state.md` (Tauri 10 → 14 commands) + `_shared-contract.md` (§1 + §3 同步)

## 关键决策

| 决策 | 选择 | 原因 |
|---|---|---|
| session_id 谁生成 | **后端** | 后端是 source of truth, agent_host 已用 sess_YYYYMMDD_HHMMSS_微秒 格式 |
| `agent_host` 可见性 | `pub(crate)` | 防止外部 caller 绕过 RuntimeApi 直调内部 API |
| 旧 daemon HTTP 处理 | 改用 RuntimeApi, 不删 | daemon binary 仍存在 (VPS / 远程场景), 收口而非删除 |
| 前端 plan 决策 | 暂不迁, 标 TODO | 是更大设计 (decide_plan API 或 LLM tool call), 独立工作项 |
| paused_count | 临时 0 占位 | router status endpoint 暂用, 后续 list_sessions(filter=Paused) 精确化 |

## 复用现有工具

- `AgentLoopHost::create_session(opts)` (扩展为接受 opts) — 不重新实现
- `AgentLoopHost::delete_session() / pause_session() / resume_session()` — 已存在
- `SessionStore::create()` — 已在 create_session_impl 内部调用
- `Tauri command 1:1 thin adapter 模式` — 4 个新 command 严格按此模式

## 验证

### 编译
- `cargo check -p qianxun-runtime` ✓
- `cargo check --workspace` ✓
- `cd qianxun-desktop/src-tauri && cargo check` ✓

### 单元测试
- `pnpm vitest run src/lib/stores/session.svelte.test.ts` — 8/8 passed (含新加 `create_calls_invoke_and_pushes_store` 测试)

### 端到端验收(用户手动)
1. ✅ 点 "新建任务" → 侧栏出现新 session (ID 后端生成, sess_ 格式)
2. ✅ 输入消息 + 发送 → 流式响应, **无 NotFound 错误**
3. ✅ 删除/暂停/恢复 session 通过 sessionStore.delete/pause/resume 方法
4. ✅ paused session 调 send_message 返 InvalidRequest
5. ✅ 退出 + 重启 → session 列表从 SQLite 恢复

### 旧 daemon HTTP 路径
- `POST /v1/chat/session` → 走 RuntimeApi::create_session ✓
- `DELETE /v1/chat/session/{id}` → 走 RuntimeApi::delete_session ✓
- `POST /v1/chat/session/{id}/pause` → 走 RuntimeApi::pause_session ✓

## 改动文件清单

| 文件 | 改动 |
|---|---|
| `qianxun-runtime/src/agent_host.rs` | +CreateSessionOpts, 改 create_session 签名, 改 3 个 test |
| `qianxun-runtime/src/api/trait_def.rs` | +4 个 trait 方法 |
| `qianxun-runtime/src/api/types.rs` | +CreateSessionRequest |
| `qianxun-runtime/src/api/sessions.rs` | +create_session_impl, +delete_session_impl, +pause_session_impl (含 InvalidRequest 区分), +resume_session_impl |
| `qianxun-runtime/src/api/send.rs` | +paused 校验 + 空 messages 校验 |
| `qianxun-runtime/src/core.rs` | +5 个 RuntimeApi impl 委托 |
| `qianxun-runtime/src/state.rs` | +agent_host pub(crate), +3 个公开方法 |
| `qianxun-desktop/src-tauri/src/commands/runtime/sessions.rs` | +4 个 Tauri command |
| `qianxun-desktop/src-tauri/src/lib.rs` | generate_handler +4 行 |
| `qianxun-desktop/src/lib/ipc/runtime.ts` | +createSession, +deleteSession, +pauseSession, +resumeSession |
| `qianxun-desktop/src/lib/stores/session.svelte.ts` | create 改异步, +3 个方法 |
| `qianxun-desktop/src/lib/components/col1/NewTaskButton.svelte` | onNewTask 加 async/await |
| `qianxun-desktop/src/lib/stores/session.svelte.test.ts` | mock createSession, 改测试 |
| `qianxun-desktop/src/lib/components/layout/ChatView.svelte` | **删** |
| `qianxun-desktop/src/lib/components/chat/InputBox.svelte` | **删** |
| `qianxun-desktop/src/lib/components/layout/SessionList.svelte` | **删** |
| `qianxun/src/runtime/router.rs` | 3 个 handler 改用 RuntimeApi |
| `qianxun/src/runtime/mod.rs` | 4 处 state.runtime.agent_host.* 改用新方法 |
| `docs/10_事实源/runtime-state.md` | RuntimeApi 6 → 10 方法表 + 文档同步 |
| `docs/10_事实源/desktop-state.md` | Tauri 10 → 14 commands + 2026-06-09 重构记录 |
| `docs/30_子项目规划/_shared-contract.md` | §1 + §3 同步 |

**总计**: ~17 文件修改 + 3 文件删除 + 1 文件新建

## 已知遗留缺口(下个 PR)

### P0
- **P0-1**: 用户手动 6 步 E2E 验收(可立即做,代码已就绪)
- **P0-3**: `list_plans` Tauri command 注册
- **P0-4**: `project.svelte.ts:loadAll` 后端实现

### P1
- **P1-1**: Plan 持久化(内存 HashMap → SQLite)
- **P1-2**: SessionStore 路径分 desktop.db vs daemon.db
- **P1-4**: `connection.svelte.ts` 接真 `daemon_health_fetch`
- **P1-5**: Plan 决策 keyword `chat.svelte.ts:103` 迁移到后端 RuntimeApi.decide_plan
- **P1-6**: `sub_session.sendToSubSession` 后端实现
- **P1-7**: `paused_count` 临时 0 占位改用 list_sessions(filter=Paused)

### P2 (独立工作项)
- Plan 决策语义设计 + 后端 RuntimeApi.decide_plan
- 死代码:`+page.svelte` 等可能引用不存在的旧 sessionStore API (grep 已确认无, 但有"残留"风险)
- 旧 daemon 收口剩余:Stage 7a LLM provider 8 endpoint 仍是直调 (本 PR 改 3 个 session 生命周期)

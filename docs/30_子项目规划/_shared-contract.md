# Tauri + Runtime 共享契约

> 状态: 生效 | 创建: 2026-06-09 | 适用范围: qianxun-desktop + qianxun-runtime + qianxun-core
>
> **设计基线**: [ADR-0003: 桌面 + ACP 同进程 2-Mode 互斥](../30_决策/ADR-0003_desktop_2mode.md)
>
> **事实源**: [runtime-state.md](../10_事实源/runtime-state.md) + [desktop-state.md](../10_事实源/desktop-state.md)

## 概述

本文件定义 `tauri → core` 链路的所有跨 crate 接口契约。三方必须遵守:

- **Tauri 端** (`qianxun-desktop`): 通过 IPC invoke 调 Tauri command
- **Runtime 端** (`qianxun-runtime`): 暴露 `RuntimeApi` trait
- **Core 端** (`qianxun-core`): 提供 `AgentLoop` + `LlmProvider` + `ToolRegistry` + `MemoryCore`

变更此文件需同步更新 3 个代码位置:`RuntimeApi` trait、`#[tauri::command]` 注册、`Svelte 5` store 调用。

## §1 RuntimeApi 契约 (10 个方法)

`qianxun-runtime/src/api/trait_def.rs:35-79` 定义。`qianxun-runtime/src/core.rs:41-76` blanket `impl RuntimeApi for Arc<RuntimeState>`。

| 方法 | 签名 | 流式 | 错误 | 备注 |
|---|---|---|---|---|
| `list_sessions` | `(filter: SessionFilter) -> ListSessionsResponse` | ❌ | `NotFound` / `Internal` | filter: `active` / `paused` / `stored` / `all` |
| `create_session` | `(req: CreateSessionRequest) -> SessionInfo` | ❌ | `Unavailable` / `Internal` | **后端生成 sess_ 格式 ID** (2026-06-09 加) |
| `send_message` | `(session_id, req) -> (SendResponse, mpsc::Receiver<SseEvent>)` | ✅ | `NotFound` / `InvalidRequest` / `LlmError` | **唯一流式**,64 容量 mpsc; paused session 返 InvalidRequest |
| `create_plan` | `(input: PlanInput) -> PlanInfo` | ❌ | `Internal` | 内存 HashMap,重启丢 |
| `list_plans` | `() -> Vec<PlanInfo>` | ❌ | `Internal` | **P0 漏接**:Tauri 无 command |
| `cancel_plan` | `(plan_id) -> ()` | ❌ | `NotFound` / `Internal` | 状态置 `Aborted` |
| `cancel_session` | `(session_id) -> ()` | ❌ | `NotFound` | 软取消,仅设 `paused` flag |
| `delete_session` | `(session_id) -> ()` | ❌ | `NotFound` | 删 in-memory + SQLite (2026-06-09 加) |
| `pause_session` | `(session_id) -> ()` | ❌ | `NotFound` / `InvalidRequest` | already paused 返 InvalidRequest (2026-06-09 加) |
| `resume_session` | `(session_id) -> ()` | ❌ | `NotFound` / `InvalidRequest` | not paused 返 InvalidRequest (2026-06-09 加) |
| `load_session` | `(session_id) -> SessionState` | ❌ | `NotFound` / `Internal` | 含 `conversation_json` |

**不变量**:
- 所有方法 `async fn` + `RuntimeApiResult<T>` (= `Result<T, RuntimeApiError>`)
- `send_message` 立即返回 `(SendResponse, Receiver)`,**不**等待 LLM 响应
- 失败 fallback: `RuntimeState::new_for_test()` (`state.rs:147-155`),业务永远能启动

## §2 SseEvent 契约 (12 个变体)

`qianxun-runtime/src/sse.rs:29-89` 定义。`#[serde(tag = "type")]` 内部 tag,JSON 形如 `{"type":"text_delta","index":0,"text":"..."}`。

| # | 变体 | 关键字段 | 触发点 | 前端处理 |
|---|---|---|---|---|
| 1 | `MessageStart` | session_id, model, max_tokens | `DaemonOutputSink::begin_message()` | 初始化流状态 |
| 2 | `ContentBlockStart` | index, block_type | block 类型切换 | 切 block |
| 3 | `TextDelta` | index, text | LLM 流 token | append 到 content |
| 4 | `ThinkingDelta` | index, text | thinking 流 | append 到 thinking |
| 5 | `ToolUseDelta` | index, id, name, arguments_json | 预留,当前 provider 走批式不产生 | noop |
| 6 | `ToolUseComplete` | index, id, name, arguments | 批式 tool 调 | 注册 toolCall |
| 7 | `ToolResult` | tool_use_id, content, is_error, elapsed_ms | 工具执行后 | append 工具结果 |
| 8 | `ContentBlockStop` | index | block 切换 / finalize | 关闭 block |
| 9 | `Usage` | input/output/cache tokens | LLM usage 累计 | 更新 token 计数 |
| 10 | `MessageDelta` | stop_reason | 收尾时 | 设置 stopReason |
| 11 | `MessageStop` | (无) | 最末条 | finished = true |
| 12 | `Error` | code, message | LlmError 6 变体 → 4 个 error code | 显示错误 |

**不变量**:
- `MessageStart` 一定是流的第一个事件(在所有 `*Delta` / `*BlockStart` 之前)
- `MessageStop` 一定是流的最后一个事件(成功或失败后)
- `Error` 之后可能没有 `MessageStop`(异常终止)
- 同一 `index` 的 `ContentBlockStart` ... `ContentBlockStop` 严格配对

**错误码**(`Error.code` 4 个值):
- `auth` — API key 缺失或鉴权失败
- `rate_limit` — 触发限流
- `api_error` — LLM provider 返回错误
- `internal` — 本地异常(序列化、IO 等)

## §3 Tauri command 注册契约

`qianxun-desktop/src-tauri/src/lib.rs:56-69` `generate_handler!` 注册 14 个 (2026-06-09 增 4 个 session 生命周期),前端 `ipc/runtime.ts` 1:1 包装。

| Tauri command | RuntimeApi 方法 | 前端 invoke | 文件:行 |
|---|---|---|---|
| `list_sessions` | `list_sessions` | `listSessions(filter)` | `commands/runtime/sessions.rs:18-34` |
| `create_session` | `create_session` | `createSession(request)` | `commands/runtime/sessions.rs:30-50` |
| `delete_session` | `delete_session` | `deleteSession(sessionId)` | `commands/runtime/sessions.rs:52-67` |
| `pause_session` | `pause_session` | `pauseSession(sessionId)` | `commands/runtime/sessions.rs:69-83` |
| `resume_session` | `resume_session` | `resumeSession(sessionId)` | `commands/runtime/sessions.rs:85-99` |
| `send_message` | `send_message` | `sendMessage(sessionId, req)` | `commands/runtime/send.rs:37-51` |
| `create_plan` | `create_plan` | `createPlan(input)` | `commands/runtime/plans.rs:18-24` |
| `cancel_plan` | `cancel_plan` | `cancelPlan(planId)` | `commands/runtime/plans.rs:30-36` |
| `cancel_session` | `cancel_session` | `cancelSession(sessionId)` | `commands/runtime/cancel.rs:14-23` |
| `load_session` | `load_session` | `loadSession(sessionId)` | `commands/runtime/load.rs:16-25` |
| **`list_plans`** | `list_plans` | **缺失** | **P0 漏接** |
| `health_check` | (无) | `healthCheck()` | mock |
| `daemon_health_fetch` | (无) | `fetchDaemonHealth(url)` | 真 reqwest |
| `set_secret` / `get_secret` | (无) | `setSecret` / `getSecret` | iota_stronghold |
| `delete_secret` | (无) | `deleteSecret` | **P0 漏接**:后端缺失 |

**不变量**:
- Tauri command 是 RuntimeApi 方法的 1:1 包装,**不**做额外业务逻辑
- 入参用 `serde::Deserialize`,返参用 `serde::Serialize`
- 错误统一 `Result<T, String>`,前端 `RuntimeApiError.parse(raw)` 解析回 `RuntimeApiError` 枚举

## §4 Tauri Event Bus 契约

| 事件名 | payload | 触发点 | 前端订阅 |
|---|---|---|---|
| `"session_event"` | `{ session_id, event: SseEvent }` | `commands/runtime/send.rs:54-67` 每条 SseEvent 一次 | `ipc/runtime.ts:312` `onSessionEvent` 全局 listener |
| `"daemon://state-changed"` | `"connected"` / 状态字符串 | `events/state_changed.rs:11` 0.5s 启动后 | `bridge.ts:71` `onDaemonStateChanged` |

**不变量**:
- `"session_event"` payload 必含 `session_id`(路由键),前端按 session_id 分发到对应 `MessageStreamState`
- `mpsc` 通道容量 64,满后 `sink.send().await` 返 Err 静默,不 panic

## §5 端到端数据流契约

```
1. ChatView button click                                  (+page.svelte:59)
2. chat.svelte.ts:send() 追加 user msg                   (chat.svelte.ts:81)
3. ipc/runtime.ts:sendMessage() invoke "send_message"    (runtime.ts:237)
4. Tauri commands/runtime/send.rs:send_message           (send.rs:37-51)
5. RuntimeApi::send_message → send_message_impl          (api/send.rs:36)
6. tokio::spawn processing_loop::handle_user_message     (api/send.rs:114)
7. mpsc::Receiver<SseEvent> 64 容量                       (api/send.rs:99)
8. spawn_event_emitter 消费 rx, app.emit("session_event")(send.rs:54-67)
9. Svelte onSessionEvent 全局 listener 路由               (ipc/runtime.ts:312)
10. chat-stream.ts 12-event 状态机更新 MessageStreamState(chat-stream.ts:79)
11. sessionStore 反应式更新, MessageBubble 重渲染
```

**每跳都序列化**:仅跳 3 (Tauri IPC) 和跳 8 (`SseEvent` → JSON payload) 有序列化,其它都是 in-process Rust 函数调用或 mpsc 通道。

## §6 配置契约

| 项 | 路径 | 决策方 |
|---|---|---|
| `~/.qianxun/config.json` | `qianxun-core/src/workspace.rs:173` | core |
| `~/.qianxun/mem.db` | `qianxun-runtime/src/state.rs:48-50` | runtime |
| `~/.qianxun/daemon.db` | `qianxun-runtime/src/state.rs:51-53` | runtime |
| `<app_local_data_dir>/stronghold-snapshot.bin` | `commands/stronghold/snapshot.rs:10-19` | Tauri |
| 前端 `daemonUrl` 默认 | `http://127.0.0.1:23900` | `connection.svelte.ts:21` |
| API key env | `DEEPSEEK_API_KEY` (大小写不敏感) | `qianxun-core/src/config.rs:398-421` |
| Session 上限 | 10 (硬编码) | `qianxun-runtime/src/state.rs:114` |
| Provider 协议 | Anthropic Messages API + SSE | `qianxun-core/src/provider/deepseek.rs` |

**变更规则**:
- 改 `~/.qianxun/*` 路径 → 同步 `qianxun-core/src/workspace.rs` + `qianxun-runtime/src/state.rs`
- 改 API key env 变量名 → 同步 `qianxun-core/src/config.rs:401`
- 改 Session 上限 → 同步 `qianxun-runtime/src/state.rs:114, 163`

## §7 错误契约

**Tauri command 层** (`commands/runtime/*.rs`):
```rust
#[tauri::command]
async fn send_message(...) -> Result<SendResponse, String>
```
错误统一 `String`,前端 `RuntimeApiError.parse(raw)` 解析。

**RuntimeApi 层** (`api/trait_def.rs`):
```rust
pub type RuntimeApiResult<T> = Result<T, RuntimeApiError>;

pub enum RuntimeApiError {
    NotFound(String),         // session_id / plan_id 不存在
    Internal(String),         // 内部错误
    LlmError(LlmError),       // LLM 调用错误
    Cancelled,                // 用户取消
}
```

**LLM 层** (`qianxun-core/src/types.rs`):
```rust
pub enum LlmError {
    NoApiKey,
    HttpError(...),
    StreamError(...),
    SerializationError(...),
    RateLimited,
    AuthError,
    ...
}
```

**SseEvent Error** (`qianxun-runtime/src/sse.rs:205-256`): LlmError 6 变体 → 4 个 error code (`auth` / `rate_limit` / `api_error` / `internal`)。

## §8 跨 Track 协调规则

| 改这条契约 | 同步修改 |
|---|---|
| RuntimeApi 新增方法 | `trait_def.rs` + 全部 `commands/runtime/*.rs` + 全部 `lib/ipc/*.ts` + 全部 `lib/stores/*.svelte.ts` |
| RuntimeApi 改签名 | 同上 + `chat-stream.ts` 状态机 |
| SseEvent 加 variant | `sse.rs` + `chat-stream.ts:79` 12-case switch + `desktop-state.md` "SseEvent 12 变体" 段 |
| SseEvent 改字段 | 同上 + 各状态路由逻辑 |
| Tauri command 加 | `lib.rs:56-69` `generate_handler!` + `ipc/runtime.ts` 包装 + `desktop-state.md` "Tauri command 全表" 段 |
| 改 `~/.qianxun/*` 路径 | `workspace.rs` + `state.rs` + `desktop-state.md` 配置表 |

## 不在本契约范围

- VPS Server 复用 RuntimeApi 的情况(未实现,不在 "tauri → core" 主线)
- 旧 CLI 客户端(被 ADR-0003 取代)
- ACP 协议(Mode B 独立分支,见 `qianxun/src/acp/`)
- 旧 `daemon-design.md` 5 实体(Project/Session/Plan/SubSession/Experience) — 已被 4a-1 简化,RuntimeApi 当前不强制要求

## 相关文件

- 设计基线: `docs/30_决策/ADR-0003_desktop_2mode.md`
- 事实源: `docs/10_事实源/runtime-state.md` + `desktop-state.md`
- 集成规划: `docs/30_子项目规划/04b-tauri-runtime-integration.md`
- 抽取设计: `docs/30_子项目规划/04c-qianxun-runtime-extraction.md`
- 实施经验: `docs/40_经验/2026-06-08_04b_subtask_{2,3,4}_*.md`

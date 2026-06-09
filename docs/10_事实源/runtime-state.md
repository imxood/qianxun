# qianxun-runtime 子系统状态

> 状态: 生效 | 适用范围: qianxun-runtime crate | 最后更新: 2026-06-09

## 概述

`qianxun-runtime` 是 Agent 运行时封装层,提供 `RuntimeApi` trait 供 Tauri desktop 和 daemon binary 共享同一套 Agent 循环。它是 "tauri → core" 链路的中间层,负责:

- 持有共享状态 (`RuntimeState`)
- 暴露 6 个异步方法 (含一个流式)
- 通过 `mpsc::Receiver<SseEvent>` 解耦业务与传输
- 通过 `SessionStore` 持久化会话

## 模块结构

```
qianxun-runtime/src/
├── lib.rs                # 公开导出
├── core.rs               # RuntimeApi blanket impl (Arc<RuntimeState>)
├── state.rs              # RuntimeState 字段清单 + 构造
├── agent_host.rs         # AgentLoopHost (10 sessions 上限)
├── api/
│   ├── trait_def.rs      # RuntimeApi trait 定义
│   ├── send.rs           # send_message_impl (核心, ~140 行)
│   ├── plans.rs          # create/list/cancel plan (内存 HashMap)
│   ├── sessions.rs       # list_sessions
│   ├── load.rs           # load_session
│   └── cancel.rs         # cancel_session (软取消)
├── output_sink.rs        # DaemonOutputSink → 推 SseEvent 到 mpsc
├── sse.rs                # SseEvent 12 变体 + SseEventBuilder
├── persistence.rs        # SessionStore (SQLite, daemon.db)
└── session_runtime.rs    # 单 session 状态
```

## 公开 API (RuntimeApi trait)

`qianxun-runtime/src/api/trait_def.rs:35-79` 定义 10 个方法,`core.rs:41-76` blanket `impl RuntimeApi for Arc<RuntimeState>`。

| 方法 | 签名 | 备注 |
|---|---|---|
| `list_sessions` | `(filter: SessionFilter) -> ListSessionsResponse` | filter: `active` / `paused` / `stored` / `all` |
| `create_session` | `(req: CreateSessionRequest) -> SessionInfo` | **后端生成 sess_ 格式 ID, 持久化** (2026-06-09 加) |
| `send_message` | `(session_id, req) -> (SendResponse, Receiver<SseEvent>)` | **唯一流式**,64 容量 mpsc |
| `create_plan` | `(input: PlanInput) -> PlanInfo` | 内存 HashMap,**重启丢** |
| `list_plans` | `() -> Vec<PlanInfo>` | **P0 漏接**:Tauri 无 command 包装 |
| `cancel_plan` | `(plan_id) -> ()` | 状态置 `Aborted` |
| `cancel_session` | `(session_id) -> ()` | 软取消,仅设 `paused` flag |
| `delete_session` | `(session_id) -> ()` | 删除 in-memory + SQLite (2026-06-09 加) |
| `pause_session` | `(session_id) -> ()` | 设 paused, send_message 返 InvalidRequest (2026-06-09 加) |
| `resume_session` | `(session_id) -> ()` | 清 paused (2026-06-09 加) |
| `update_active_provider` | `(req: UpdateProviderRequest) -> ()` | 写 `~/.qianxun/config.json` (原子), **不热替换**, 需重启 desktop (2026-06-09 加) |
| `load_session` | `(session_id) -> SessionState` | 含 `conversation_json` |

**RuntimeState 内部字段**:
- `agent_host: pub(crate)` — 外部禁止直接访问, 必须走 RuntimeApi trait
- 其他 9 字段 (provider / config / tools / memory / skills / shared / store / plans / shutdown_tx) 仍 pub, 给 RuntimeApi impl + DaemonOutputSink 等内部组件用

**RuntimeState 公开方法** (供 graceful shutdown / metrics 用, 非 RuntimeApi):
- `session_count() -> usize` — in-memory session 数
- `shutdown_all_sessions() -> usize` — 标记所有 session 为 paused
- `spawn_reap_stale()` — 启动 1h 清理周期后台 task (RuntimeState::new 自动调)

**Session 数量限制 — 2026-06-09 修正**:
- **取消** `max_sessions` 硬编码 10 上限
- `agent_host.create_session()` 不再检查上限, 接受无限 session
- 理由: session 是**持久化状态** (SQLite 一行), 不是"运行中 LLM 调用"。5000 个 session 是合理的 (用户偏好)
- `max_sessions` 字段保留 (类型仍 `usize`), 默认 `usize::MAX`, 后续可改成从 `~/.qianxun/config.json` 读
- **运行中并发控制** 移到 processing_loop 内部 (Semaphore, 后续 PR), 跟 session 数量解耦

## SseEvent 12 变体

`qianxun-runtime/src/sse.rs:29-89` 定义。前端 `chat-stream.ts:79` 12-case switch 镜像处理。

| # | 变体 | 关键字段 | 触发点 |
|---|---|---|---|
| 1 | `MessageStart` | session_id, model, max_tokens | `DaemonOutputSink::begin_message()` |
| 2 | `ContentBlockStart` | index, block_type | block 类型切换时 |
| 3 | `TextDelta` | index, text | LLM 流 token |
| 4 | `ThinkingDelta` | index, text | thinking 流 |
| 5 | `ToolUseDelta` | index, id, name, arguments_json | 预留,当前 provider 走批式不产生 |
| 6 | `ToolUseComplete` | index, id, name, arguments | 批式 tool 调 |
| 7 | `ToolResult` | tool_use_id, content, is_error, elapsed_ms | 工具执行后 |
| 8 | `ContentBlockStop` | index | block 切换 / finalize |
| 9 | `Usage` | input/output/cache tokens | LLM usage 累计 |
| 10 | `MessageDelta` | stop_reason | 收尾时 |
| 11 | `MessageStop` | (无) | 最末条 |
| 12 | `Error` | code, message | `LlmError` 6 变体 → 4 个 error code |

JSON 序列化用 `#[serde(tag = "type")]`,形如 `{"type":"text_delta","index":0,"text":"..."}`。

## 端到端: send_message 内部流

```
send_message_impl(state, session_id, req)
  ├─ agent_host.get_session(session_id)         // 404 if missing
  ├─ if runtime.is_paused() → InvalidRequest (409)   // 2026-06-09 加
  ├─ if req.messages.is_empty() → InvalidRequest (400) // 2026-06-09 加
  ├─ conv.push_user_message(ContentBlock)
  ├─ state.memory.build_context(&msg, 2000)     // FTS5 检索
  ├─ skills.build_catalog                       // 加载 skill 提示
  ├─ AgentLoop::new(runtime.agent.clone())      // qianxun-core 引擎
  ├─ mpsc::channel::<SseEvent>(64)
  ├─ DaemonOutputSink::new(tx, ...)             // 输出到 mpsc
  ├─ tokio::spawn(processing_loop::handle_user_message)
  └─ return (SendResponse { status: "streaming" }, rx)
```

调用方 (Tauri `commands/runtime/send.rs:54-67`) 拿到 `rx` 后,`spawn_event_emitter` 消费 `rx.recv()` 并 `app.emit("session_event", payload)`。

## 配置

| 项 | 值 | 路径 |
|---|---|---|
| Memory 数据库 | `<qianxun_dir>/mem.db` | `state.rs:48-50` |
| Session 数据库 | `<qianxun_dir>/daemon.db` | `state.rs:51-53` |
| Session 上限 | **10 (硬编码)** | `state.rs:114` |
| 失败 fallback | `new_for_test()` → temp_dir | `state.rs:147-155` |
| API key 读取 | env `DEEPSEEK_API_KEY` → config.json → 空 | `qianxun-core/src/config.rs:398-421` |

## 已知缺口

### P0 (端到端跑通核心)

- **P0-1**: 用户手动 E2E 验收。6 步清单见 `40_经验/2026-06-08_Phase_ABCD_收尾总览.md:155-171`
- **P0-3**: `list_plans` 在 trait 里有,但 Tauri 无 command 包装,前端无 invoke
- **P0-4**: `project.svelte.ts` 后端缺 `list_projects` / `create_project` RuntimeApi

### P1 (影响体验和稳定性)

- **P1-1**: Plan 持久化(当前 in-memory HashMap,重启丢)
- **P1-2**: SessionStore 路径分 desktop.db vs daemon.db(避免跨进程锁)
- **P1-3**: `SseEvent::PlanUpdate` 实时事件(plan 进度只能轮询)
- **P1-4**: `cancel_session` 软取消(未接 `tokio::CancellationToken`,非真中断 LLM HTTP)
- **P1-5**: paused_count 走 list_sessions(目前 router.rs status endpoint 用 0 占位, 应改用 list_sessions(filter=Paused).await)

## 2026-06-09 日志增强

P0 解决了 5 个最 silent 的 silent failure 点 (用户"发消息没响应" + "前端看到 error toast 但后端无记录"的悬案):

- **L1 桌面端 tracing subscriber 初始化**: `qianxun-desktop/src-tauri/src/lib.rs` 调 `tracing_subscriber::fmt().with_env_filter("info,qianxun_runtime=debug").init()`, 默认 level=info, `RUST_LOG` env 可控. 之前业务 `tracing::info!/warn!` 全部被吞.
- **L2 Tauri command `send_message` tracing**: `commands/runtime/send.rs:38-67` 加 entry log (session_id, msg_count, user_chars), reject warn, spawn_event_emitter 抽样 log (每 50 事件 info 一次), stream closed final info (event_count 统计).
- **L3 `SseEventBuilder::error_from_llm` warn**: `qianxun-runtime/src/sse.rs:255` LlmError → SseEvent::Error 转换时加 `tracing::warn!(code, message)`. 之前 0 行, stderr 完全无审计.
- **L4 `output_sink.on_error` warn**: `qianxun-runtime/src/output_sink.rs:325` 之前 trait 实现直接 `self.error(e).await`, 加 `tracing::warn!(session, error)`.
- **L5 Anthropic HTTP 错误分级 log**: `qianxun-core/src/provider/anthropic_compat.rs:433` 4xx warn (user 问题: 401/403/404), 5xx error (服务端问题). 之前 0 行, 401/429/5xx 完全 silent.
- **L6 `send_message_impl` entry/spawn log**: `qianxun-runtime/src/api/send.rs:36,154` 0 行 tracing 变 entry info + spawn info + exit info. session 找不到时加 warn (之前 0 行 NotFound log).

排查时设 `RUST_LOG=info,anthropic=debug` 即可看到完整链路. Tauri 桌面端 stderr 也走同一 subscriber.

## 启动序列

`qianxun-runtime/src/state.rs:47-138` `RuntimeState::new(config)`:

1. 计算 `mem.db` / `daemon.db` 路径
2. `create_provider(&config.active_provider, ...)` → 缺 key 走 `new_for_test()` fallback
3. `tools.register_all_builtin()` (8 个 builtin 工具,失败 fallback 空 + warn)
4. `MemoryCore::open(&mem_path)` (失败 fallback in-memory + warn)
5. `SkillManager::load_all(None)` (空目录静默 OK)
6. `SessionStore::new(&store_path)`
7. `SharedState::new(...)` 构造 AgentLoop 共享状态
8. `AgentLoopHost::new(10, ...)` 启 host
9. `agent_host.restore_from_disk()` 从 SQLite 恢复 session

## 设计要点

1. **RuntimeApi 既是 trait 又是 factory**: blanket `impl RuntimeApi for Arc<RuntimeState>` 让 `state: State<'_, Arc<RuntimeState>>` (Tauri) 和 `state: Arc<RuntimeState>` (daemon) 共用同一套实现
2. **流式走 mpsc 不用 callback**: 业务 (send_message_impl) 跟传输 (Tauri emit / HTTP SSE) 完全解耦,同一份代码两种部署形态
3. **SseEvent 是 in-process enum,非 SSE 协议**: 序列化只在 emit 到 Tauri 那一刻发生
4. **SessionStore 独立于 MemoryCore**: 两者都是 SQLite 但是不同文件,关注点分离

## 不在本文件范围

- Tauri 侧 command 注册和前端 store → `desktop-state.md`
- AgentLoop 内部状态机 → `qianxun-core/src/agent/engine.rs` (不需单独事实源,代码即真相)
- VPS Server 复用 RuntimeApi 的情况 → 不在 "tauri → core" 主线

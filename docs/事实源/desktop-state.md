# qianxun-desktop 子系统状态

> 状态: 生效 | 适用范围: qianxun-desktop (Tauri + Svelte 5) | 最后更新: 2026-06-09 (启动 splash + 启动优化)

## 概述

Tauri 2.x 桌面端。Svelte 5 webview 通过 Tauri IPC `invoke` 调 `qianxun-runtime` API,**in-process,零网络**。这是 "tauri → core" 链路的前端入口。

`qianxun-desktop` **不在** 主 Cargo workspace,独立维护。

## 模块结构

```
qianxun-desktop/
├── src-tauri/                          # Rust 后端
│   ├── Cargo.toml                       # tauri 2.x + qianxun-runtime path dep
│   ├── tauri.conf.json                  # 1280x800, 7 平台 targets
│   ├── build.rs
│   └── src/
│       ├── main.rs                      # run() 入口 (delegate to lib::run)
│       ├── lib.rs                       # generate_handler! (10 commands)
│       ├── state/
│       │   └── runtime.rs               # 加载 config, 构造 RuntimeState
│       ├── commands/
│       │   ├── health/
│       │   │   ├── check.rs             # health_check (mock)
│       │   │   └── fetch.rs             # daemon_health_fetch (真 reqwest)
│       │   ├── runtime/
│       │   │   ├── send.rs              # send_message + spawn_event_emitter
│       │   │   ├── plans.rs             # create_plan / cancel_plan
│       │   │   ├── sessions.rs          # list_sessions
│       │   │   ├── load.rs              # load_session
│       │   │   └── cancel.rs            # cancel_session
│       │   └── stronghold/
│       │       └── key.rs               # set/get_secret (iota_stronghold)
│       └── events/
│           └── state_changed.rs         # emit "daemon://state-changed"
├── src/                                 # Svelte 5 前端
│   ├── lib/
│   │   ├── ipc/
│   │   │   ├── bridge.ts                # health + stronghold 包装
│   │   │   └── runtime.ts               # 6 个 runtime command invoke 包装
│   │   ├── stores/                      # 11 个 svelte store
│   │   │   ├── connection.svelte.ts     # 4 态机
│   │   │   ├── session.svelte.ts        # session 列表
│   │   │   ├── chat.svelte.ts           # 消息流 + onSessionEvent
│   │   │   ├── plan.svelte.ts           # plan CRUD
│   │   │   ├── project.svelte.ts        # [占位]
│   │   │   ├── sub_session.svelte.ts    # [占位]
│   │   │   ├── team.svelte.ts           # VPS 外部 fetch
│   │   │   ├── vps.svelte.ts            # VPS WebSocket
│   │   │   ├── settings.svelte.ts       # localStorage
│   │   │   ├── ui.svelte.ts             # 纯 UI
│   │   │   └── persist.svelte.ts        # 工具函数
│   │   │   └── chat-stream.ts           # 12-event 状态机 (非 svelte)
│   │   └── components/                  # 29 个 Svelte 组件
│   └── routes/
│       └── +page.svelte                 # 渲染 ChatView
├── package.json
├── pnpm-workspace.yaml
└── vite.config.ts
```

## Tauri command 全表

`qianxun-desktop/src-tauri/src/lib.rs:56-69` `generate_handler!` 注册 14 个 (2026-06-09 增 4 个)。

| Command | 文件:行 | 状态 | 说明 |
|---|---|---|---|
| `health_check` | `health/check.rs:5-15` | **mock** | 永远返 `Connected`,不调 runtime |
| `daemon_health_fetch` | `health/fetch.rs:9-82` | **真** | reqwest 3s 超时 GET `{url}/v1/system/health` |
| `set_secret` | `stronghold/key.rs:10-21` | **真** | iota_stronghold Argon2 + ChaCha20 |
| `get_secret` | `stronghold/key.rs:24-32` | **真** | 密码错或 key 不存在返 `Ok(None)` |
| **`delete_secret`** | (缺失) | **P0 漏接** | TS `bridge.ts:113` invoke 但后端无 command,Tauri 模式会失败 |
| `list_sessions` | `runtime/sessions.rs:18-34` | **真** | 委托 `RuntimeApi::list_sessions` |
| `create_session` | `runtime/sessions.rs:30-50` | **真** | **后端生成 sess_ 格式 ID** (2026-06-09 加) |
| `delete_session` | `runtime/sessions.rs:52-67` | **真** | 委托 `RuntimeApi::delete_session` (2026-06-09 加) |
| `pause_session` | `runtime/sessions.rs:69-83` | **真** | 委托 `RuntimeApi::pause_session` (2026-06-09 加) |
| `resume_session` | `runtime/sessions.rs:85-99` | **真** | 委托 `RuntimeApi::resume_session` (2026-06-09 加) |
| `send_message` | `runtime/send.rs:37-51` | **真** | 调 RuntimeApi + spawn emit task |
| `create_plan` | `runtime/plans.rs:18-24` | **真** | 委托 `RuntimeApi::create_plan` |
| `cancel_plan` | `runtime/plans.rs:30-36` | **真** | 委托,状态置 `Aborted` |
| `cancel_session` | `runtime/cancel.rs:14-23` | **真** | 软取消 |
| `load_session` | `runtime/load.rs:16-25` | **真** | 委托 `RuntimeApi::load_session` |
| `update_active_provider` | `runtime/sessions.rs` 尾部 | **真** | 委托 RuntimeApi (2026-06-09 加) |
| **`list_plans`** | (缺失) | **P0 漏接** | `RuntimeApi` trait 有,Tauri 无 command,前端无 invoke |

## 端到端链路 (Svelte 5 → LLM 流式响应)

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

每条 `SseEvent` 一次 `serde_json::to_value` 序列化 + Tauri 内部 emit,sub-ms 延迟。

## 前端 store 状态

| Store | 是否真调 invoke | 职责 |
|---|---|---|
| `connection.svelte.ts` | **真** (但接 mock) | 4 态机 (offline/reconnecting/degraded/connected),10s 周期 health check,调 `bridge.ts::fetchDaemonHealth` |
| `session.svelte.ts` | **真** | `init()` 调 `listSessions`,`switchTo()` 调 `loadSession` |
| `chat.svelte.ts` | **真** | `send()` 调 `sendMessage` invoke,`init()` 一次性 `onSessionEvent` 监听 |
| `plan.svelte.ts` | **真** | `create()` 调 `createPlan`,`cancel()` 调 `cancelPlan` |
| `project.svelte.ts` | **占位** | `loadAll()` noop,后端缺 `list_projects` |
| `sub_session.svelte.ts` | **占位** | `sendToSubSession` noop + TODO toast |
| `team.svelte.ts` | **真**(VPS 外部) | `refresh()` 走 VPS REST,不走 Tauri command |
| `vps.svelte.ts` | **真**(VPS 外部) | WebSocket + 3 个写方法 |
| `settings.svelte.ts` | 无 | 纯 localStorage 同步 |
| `ui.svelte.ts` | 无 | 纯 UI state |
| `persist.svelte.ts` | 无 | 工具函数 |

## 配置

| 项 | 值 | 路径 |
|---|---|---|
| 前端默认 daemonUrl | `http://127.0.0.1:23900` | `connection.svelte.ts:21` |
| Window | 1280x800 默认, 800x600 最小,可缩放 | `tauri.conf.json:13-22` |
| CSP | `null` (开发期关闭) | `tauri.conf.json:25` |
| Bundle targets | `appimage, deb, rpm, nsis, msi, app, dmg` (7 平台) | `tauri.conf.json:33-53` |
| Tauri 版本 | 2.x,固定 patch version | `Cargo.toml:26` |
| 凭据存储 | `<app_local_data_dir>/stronghold-snapshot.bin` | Tauri 决定 |

## 已知缺口

### P0 (端到端跑通核心)

- **P0-1**: 用户手动 E2E 验收(6 步清单)
- **P0-2**: `sub_session.sendToSubSession` 后端实现(RuntimeApi 加方法 + Tauri command + 替换 noop)
- **P0-3**: `list_plans` Tauri command 注册
- **P0-4**: `project.svelte.ts:loadAll` 后端实现

### P1 (影响体验和稳定性)

- **P1-1**: Plan 持久化(在 `runtime-state.md` 中)
- **P1-2**: desktop.db 路径(避免跟 daemon.db 锁竞争)
- **P1-4**: `connection.svelte.ts` 接真 `daemon_health_fetch`(当前仍用 mock)
- **P1-5**: Plan 决策逻辑 `chat.svelte.ts:103` 关键词正则移到后端

## 设计要点

1. **in-process library,不走 HTTP**: Tauri 2.x 启时 `setup` 同步构造 `RuntimeState`,后续 invoke 全部 0 网络 0 序列化(`Arc<RuntimeState>` 共享所有权)
2. **流式响应走 mpsc + emit**: 业务 (`send_message_impl`) 用 `mpsc::Receiver<SseEvent>`,Tauri 端 `spawn_event_emitter` 把它转成 `app.emit("session_event", payload)`。HTTP layer 包 SSE, Tauri 包 emit,业务零修改
3. **12-event 状态机前后端 1:1 镜像**: `SseEvent` enum 12 变体 ↔ `chat-stream.ts:79` 12-case switch,改一边必改另一边
4. **单 tab 多 session**: 后端 `AgentLoopHost` 管 `HashMap<SessionId, Arc<SessionRuntime>>`,上限 10。前端可切多个 session
5. **Tauri 2.x + 固定 patch version**: 跨小版本 API 可能微调,锁版本避免 build break

## 不在本文件范围

- Runtime 内部状态机和 SseEvent 细节 → `runtime-state.md`
- 旧 CLI 客户端 / ACP 协议 → 不在 "tauri → core" 主线
- 打包签名 / 跨平台 CI/CD → 后续单独决策

## 2026-06-09 重构记录

**端到端"新建任务 → 发送内容"打通**:

- 新增 `create_session` / `delete_session` / `pause_session` / `resume_session` 4 个 Tauri commands + RuntimeApi 对应方法
- 前端 `sessionStore.create()` 改异步, 调 invoke 拿后端真 ID (修复 `[NotFound] not found: session sess_xxx` 错误)
- 清理死代码: `layout/ChatView.svelte` + `chat/InputBox.svelte` + `layout/SessionList.svelte` 删除
- `RuntimeState.agent_host` 改 `pub(crate)`, 外部必须走 RuntimeApi trait
- 旧 daemon HTTP 3 个 handler 改用 RuntimeApi (create_session / delete_session / pause_session)
- `send_message` 加 paused 校验 (返 InvalidRequest → 409) + 空 messages 校验
- `paused_count` 暂用 0 占位 (router status endpoint, 后续 list_sessions 精确化)
- 新增 `RuntimeApiError::Conflict` (409) 变体, 区分 "already paused" / "not paused" / "not found" 错误
- **取消 max_sessions 硬编码 10** (第 2 次改动): session 是持久化状态,不是运行实例,5000 个也合理
  - `agent_host.create_session()` 移除上限检查
  - `state.rs:114, 163` 硬编码 10 → `usize::MAX`
  - 实际"运行中并发控制"移到 `processing_loop` 内部 Semaphore (后续 PR)
- 前端错误反馈: `sessionStore.create()` catch 后存 `lastError` + `uiStore.pushToast`, `NewTaskButton` 加 try/catch 防 uncaught promise rejection

**Provider 设置 UI 端到端打通 (第 3 次改动)**:
- 后端: `RuntimeApi::update_active_provider` + `Config::save_to_file` 原子写 `~/.qianxun/config.json` (5 个嵌套 struct 加 `Serialize` derive)
- Tauri: `update_active_provider` command + ipc `updateActiveProvider` 包装
- 前端: `settingsStore.setActiveProvider(name, config)` 异步调 invoke + 弹 toast 提示重启
- 新建 `SettingsModal.svelte` (Provider / Model / Base URL / API Key + 重启提示)
- `SidebarFooter` "设置" 按钮接通 `uiStore.openSettings()`, "Provider · DeepSeek" 改动态读 `settingsStore.activeProvider`
- `chat-stream.ts` 处理 `SseEvent::Error` 时调 `uiStore.pushToast` (用户立刻看到 401/auth 错误, 不再静默)

**P0 致命 bug 修复 + Lazy session create + 真项目列表 (第 4 次改动)**:
- **P0 致命 bug**: `chatStore.init()` 在生产代码从未被调,导致 Tauri `session_event` listener 从未注册,所有 SseEvent 被 Tauri 丢弃,用户"发消息没响应"。`+page.svelte:onMount` 调 `await chatStore.init()` 修复
- **send catch toast**: `chat.svelte.ts:184-189` catch 块加 `uiStore.pushToast` (invoke reject 走不到 chat-stream error 分支)
- **Lazy session create**: `NewTaskButton` 只切 `uiStore.switchToNew()` 不调 invoke;`chatStore.send(session_id: string | null, ...)` 检测 null 时先 `await sessionStore.create()` 拿后端真 ID,再 sendMessage
- **ChatView 'new' view**: 显示 `ChatStream` (空 messages + `onSend=sendNew` → `chatStore.send(null, text)`),取代静态空白 div
- **后端 `SessionInfo` 加 `project_root` 字段**: 透传到前端 `Session.project_id`
- **真项目列表**: `projectStore.loadAll` 调 `listSessions('all')` 按 `project_root` 去重,derive `Project[]` (取代 `buildSeed().sessions.length` mock)
- **ProjectSection**: 改用 `projectStore.all`,显示总 session 数,空态友好提示
- **+page.svelte onMount 补 2 个 init**: `sessionStore.init()` + `projectStore.loadAll()`

**启动 Splash 屏 + 启动优化 (第 5 次改动, 解决 "10+ 秒空白")**:

- **问题**: `+page.svelte` 模板直接渲染 `ThreeColumnLayout`,期间 `uiStore.activeView` 默认 `kind: 'session', session_id: 'sess_jwt_auth'` 引用不存在的 session, ChatView 落到"还没有会话"静态空态,用户看不到任何"正在加载"反馈,误以为卡死
- **根因** (5 个):
  1. `+page.svelte` 模板无 loading state,直接渲染主布局
  2. onMount 串行 await 3 个 init (chat/session/project)
  3. mock 残留 (1.5s toast, 8s experience modal)
  4. Tauri 2.x webview 启动本身 3-5s
  5. 硬编码 fallback session `'sess_jwt_auth'` 在 init 前显示
- **修复**:
  - 新建 `lib/components/layout/LoadingSplash.svelte` (4 step progress + 进度条 + 步骤点 + aria-live)
  - `+page.svelte` 加 `bootstrapped` / `bootStep` / `bootProgress` 状态,顶层条件渲染 splash 或主布局
  - 步骤 1 (chatStore.init) + 步骤 2 (并行 sessionStore.init + projectStore.loadAll) + 步骤 3 (切首个 session) + ready
  - 步骤 2 改 `Promise.all` 并行,比串行快 ~50%
  - 完成后 `setTimeout(200ms)` 让用户看到 "100% 就绪" 一瞬再隐藏 splash
  - 删除 mock 演示 (1.5s/8s setTimeout 注释掉的 ExperienceSuggestModal)
  - 后端 `state.rs:build()` 改用 `new_for_test()` 骨架 (in-memory, <100ms) + `ensure_restored()` lazy 模式,真正 provider 初始化 + restore_from_disk 走后台 async spawn
- **后端 lazy init**:
  - `state.rs` 加 `restored: Arc<AtomicBool>` + `ensure_restored()` 幂等方法
  - `send_message` 入口调 `state.ensure_restored().await` (Tauri command 层)
  - 关键: `list_sessions_impl` **不调** `ensure_restored()` (会 overwrite `cancel_session` 设的 paused=true)
- **测试基线**: 107/107 vitest ✅ + 260/260 cargo test ✅

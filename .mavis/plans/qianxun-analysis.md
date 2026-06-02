# 千寻 (Qianxun) 项目架构盘点

> 分析者: general worker | 时间: 2026-06-02 22:50 | 范围: 全 workspace
>
> **关键结论**: 当前文档 (CLAUDE.md / architecture.md / daemon-design.md) 严重低估 Daemon/VPS/Thin-Client 的实际进度, 同时略高估 Phase 3b AgentPattern 的完成度. 本报告以代码事实为准, 文档与代码不一致处一律标注.

---

## 1. 项目快照

千寻是 Rust 实现的个人 AI 系统, workspace 含 3 个 crate:

| Crate | 类型 | 行数 (Rust) | 角色 | 实际阶段 |
|---|---|---:|---|---|
| `qianxun-core` | lib | ~6500 | 核心类型 + Agent 引擎 + Provider + 工具 + Skills + MCP | 引擎稳定, Phase 3b pattern 仅类型 |
| `qianxun-memory` | lib | ~1800 | 记忆引擎 (SQLite + FTS5 + Vector) | **已闭环**, 8 表 + 3 FTS trigger, 18 集成测试 pass |
| `qianxun` (bin: `qx`) | bin | ~8400 | 单二进制, 4 入口 (cli/tui/acp/daemon/server) + **新增 client (thin)** | Daemon 已走完 Stage 1-7a; thin-client 已可用 |

**重要**: 还有一个 `qianxun/src/client/mod.rs` (1211 行) **不在 CLAUDE.md 模块结构图里**, 它是 `qx` 探测到本地 Daemon 时自动走 HTTP+SSE 远程调用的薄客户端 (Stage 4 / 6b 范围).

**实际阶段总览** (与 docs/CLAUDE.md 不同):

| Phase | docs 标记 | 实际代码状态 |
|---|---|---|
| 1-2 | ✅ | ✅ 引擎 + Provider + 工具 + CLI REPL + ACP + 工作区 |
| 3a | 🟡 | ✅ Memory 闭环 + ✅ MCP 完整 + ✅ Skills 闭环 (memory-state.md 标 ✅) |
| 3b | 🟡 | 🟡 **类型/模板/状态机**有, **dispatcher 未接** — 仍是 React 单路径 |
| 3c/4a | 🟡 | 🟡→✅ **Daemon Stage 1-7a 全部实现** (25+ 路由, JWT 鉴权, Session 持久化, SSE 12 事件, LLM provider CRUD, SvelteKit Web UI 已 build) — 但 `processing_loop_enabled: false`, `prompt_handler` 走 direct stream 而非 processing_loop |
| 3d | ✅ | ✅ TUI ratatui 1713 行, 性能 ~447µs/帧 |
| 4b | 📋 | 🟡→✅ **VPS Server Stage 1-6b 大部分实现** (5 表, WsHub, Team/Project CRUD, Admin RBAC, RateLimiter, Outbox, static login) — 仅 full Web UI 仍 TODO |

---

## 2. 子系统矩阵

| 子系统 | docs 状态 | 代码实际状态 | 关键文件 | 缺失 |
|---|---|---|---|---|
| **核心类型** | ✅ | ✅ | `qianxun-core/src/{types,config,output,workspace,lib}.rs` | — |
| **Agent 引擎** | ✅ | ✅ (React 单循环) | `qianxun-core/src/agent/{engine,conversation,message,system_prompt}.rs` | pattern dispatch 未接 |
| **Agent Pattern (3b)** | 🟡 | 🟡 仅有类型 | `qianxun-core/src/agent/{plan,reflect,workflow}.rs` | 没有 `handle_plan_and_execute` / `handle_reflective` / `handle_workflow` 函数 |
| **LLM Provider** | ✅ | ✅ | `qianxun-core/src/provider/{mod,anthropic_compat,deepseek,types}.rs` | — |
| **工具 (builtin)** | ✅ | ✅ | `qianxun-core/src/tools/{mod,builtin}.rs` (1347+352 行) | 8 个 builtin + ToolCategoryFilter + execute_async_with_filter |
| **工具 (MCP)** | ✅ | ✅ | `qianxun-core/src/mcp/{client,server_manager,tool_wrapper,transport,config,mod}.rs` | 文档说 HTTP/SSE 缺失 (mcp-state.md) — 代码未核实 [待确认] |
| **Skills** | ✅ | ✅ | `qianxun-core/src/skills/mod.rs` (397 行) | depends_on DAG / project_only 字段解析未实装 (skills-state.md) |
| **Memory** | 🟡 | ✅ (核心闭环) | `qianxun-memory/src/{lib,db,types,search,compressor,consolidation,privacy,slot,vector}.rs` | HybridSearch 未集成, consolidation 未挂到 session_end |
| **Daemon HTTP** | 🟡 | ✅ Stage 1-7a | `qianxun/src/daemon/{mod,router,agent_host,session_runtime,sse,persistence,llm_providers,service}.rs` (4800 行) | processing_loop 未接, tool execution 路径未走 |
| **Daemon Web UI** | 📋 | ✅ Stage 7a | `qianxun/src/daemon/ui/build/` (SvelteKit 已 build) | `/v1/tools` 等返回 stub JSON, UI 仅展示不能驱动 |
| **Thin Client** | 📋 (docs 未列) | ✅ | `qianxun/src/client/mod.rs` (1211 行) | 无 |
| **VPS Server** | 📋 | 🟡 Stage 1-6b | `qianxun/src/server/{mod,auth,admin,auth_ws,messages,ws_hub,outbox,rate_limit,team_db}.rs` (4500+ 行) | full Web UI (chat/team/node) 仍 TODO |
| **TUI** | ✅ | ✅ | `qianxun/src/tui/mod.rs` (1713 行) | Markdown 渲染, Diff 渲染, @ 文件搜索 |
| **ACP** | ✅ | ✅ | `qianxun/src/acp/{types,transport,session,output,prompt,forwarding_tools,handler,server}.rs` (1900 行) | thin-client mode (走 daemon) 未实现 |
| **CLI 旧 REPL** | 行将迁移 | ✅ | `qianxun/src/cli/{cli,config,output,run}.rs` | 长期不维护 |

---

## 3. Agent 核心结构

`qianxun-core/src/agent/mod.rs` 仅 12 行, 暴露 3 个类型: `Message`, `Conversation`, `AgentLoop`. 实际能力分散在 9 个文件中:

| 文件 | 行数 | 关键类型 / 函数 | 状态 |
|---|---:|---|---|
| `message.rs` | 134 | `ContentBlock` (text/tool_use/tool_result/thinking), `Message` enum (User/Assistant) | ✅ 完整 |
| `conversation.rs` | 166 | `Conversation` (Vec\<Message\> + system_prompt), `build_request()` 拼接 4 段 (base + memory + skills_catalog + skill_injections), `enforce_budget()` 简单裁剪, JSONL 序列化 | ✅ |
| `engine.rs` | 489 | `AgentLoop { state, turn_count, retry_count, config, accumulated_usage, compact_window }`, `processing_loop::handle_user_message()` (核心 React 循环, 含 normalize / compact / stream / tool_execute / build_turn) | ✅ |
| `system_prompt.rs` | 91 | `BASE_PROMPT` + `build_system_prompt(workspace, custom, mode)`, mode 仅 `"plan" / "auto"`, 没有 `AgentPattern` 注入 | ✅ 单模式 |
| `context/window.rs` | 251 | `AutoCompactWindow`, `CompactZone` (Safe/Warning/Danger/Blocked), L1/L2/L3/L4 压缩窗口 | ✅ |
| `context/compact.rs` | 409 | `snip_tool_results` / `micro_compact` / `attempt_compression` | ✅ |
| `context/normalize.rs` | 205 | `normalize_messages` 修复 tool_use/tool_result 配对 | ✅ |
| `plan.rs` | 56 | **仅类型**: `PlanResult`, `PlanStep`, `Effort`, `PlanState` enum, `plan_phase_filter()` / `execute_phase_filter()` | 🟡 骨架, 无循环 |
| `reflect.rs` | 72 | **仅辅助**: `ReflectState` enum, `ReviewResult`, `MAX_REVIEW_ROUNDS = 2`, `should_self_review()`, `build_review_prompt()` | 🟡 骨架 |
| `workflow.rs` | 223 | **数据 + 模板**: `WorkflowTemplate` / `WorkflowStage` / `WorkflowState` / `WorkflowManager` + 4 个内置模板 (code-review/bug-fix/release/refactor) | 🟡 模板完整, **无执行循环** |

**关键发现**: `agent-pattern-design.md` §2.4 描述的 `match self.config.pattern { React / PlanAndExecute / Reflective / Workflow { template } }` 分发器**在 engine.rs 不存在**. `processing_loop::handle_user_message` 签名也不含 `pattern` 字段, 整个 main loop 写死 React. 也就是说, `plan.rs` / `reflect.rs` / `workflow.rs` 是被 `mod.rs` `pub mod` 出来的孤儿模块, 没有任何调用方.

**`build_request` 4 段拼接** (`conversation.rs:51-89`):
```text
system = base + memory_context + skills_catalog + skill_injections
```
这意味着 system prompt 注入链已经搭好, 但 `memory_context` / `skills_catalog` / `skill_injections` 三个字符串由谁填充? 在 daemon 端 (`router.rs:674-680`) 全部传空串:
```rust
let request = conv.build_request(&[], "", "", "", &runtime.resolved.agent);
```
→ daemon prompt path **不接 memory, 不接 skills** (只有 builtin tools).

---

## 4. Daemon 形态 (qianxun/src/daemon/)

### 4.1 实际 HTTP 路由表 (`router.rs:81-125`)

| 路径 | 方法 | 状态 | handler |
|---|---|---|---|
| `/` | GET | ✅ 真实 | `root_handler` — 服务自描述 JSON |
| `/v1/system/health` | GET | ✅ 真实 | 跳过 auth, `{"status":"ok"}` |
| `/v1/system/status` | GET | ✅ 真实 | 跳过 auth, 状态摘要 |
| `/v1/chat/session` | POST | ✅ 真实 | 调 `agent_host.create_session()` |
| `/v1/chat/session/{id}` | GET/DELETE | ✅ 真实 | `session_exists` / `delete_session` |
| `/v1/chat/session/{id}/prompt` | POST | ⚠️ **SSE 真实但不走 processing_loop** | `prompt_handler` 直接调 `provider.stream_completion`, 用 `SseEventBuilder` 映射 12 事件; **不调 `processing_loop::handle_user_message`, 不执行 tool, 不接 memory/skills 上下文** |
| `/v1/chat/session/{id}/cancel` | POST | 🟡 占位 | thin-client 调用, server 端未实现 cancel |
| `/v1/tools` | GET | ⚠️ stub | **硬编码返回 8 个工具名**, 不查 ToolRegistry |
| `/v1/tools/{name}/invoke` | POST | ✅ | 直调 `tools.execute_async()` |
| `/v1/memory/sessions` | GET | 🟡 stub | `{"sessions":[]}` |
| `/v1/memory/search` | POST | 🟡 stub | `{"results":[]}` |
| `/v1/skills` | GET/POST | ⚠️ | GET stub, POST `reload_skills` 实际 reload |
| `/v1/skills/{name}/toggle` | POST | 🟡 stub | 返 status, 不真持久化 |
| `/v1/mcp/servers` | GET/POST | ⚠️ | GET stub, POST `add_mcp_server` 返 `not_implemented` |
| `/v1/mcp/servers/{id}` | DELETE | 🟡 简版 | 调 `tools.remove_mcp_client()` |
| `/v1/mcp/servers/{id}/test` | POST | 🟡 stub | `not_implemented` |
| `/v1/llm/providers` | GET/POST | ✅ Stage 7a | `llm_providers.rs` CRUD |
| `/v1/llm/providers/{id}` | GET/PUT/DELETE | ✅ | 同上 |
| `/v1/llm/providers/{id}/activate` | POST | ✅ | 同上 |
| `/v1/llm/providers/{id}/test` | POST | ✅ | 真实 HTTP ping |
| `/v1/config` | GET | 🟡 stub | 返 `daemon.host + agent.max_turns` |
| `/_ui/*` | GET | ✅ Stage 7a | `ServeDir` + SPA fallback (SvelteKit dist) |
| 全局 | - | ✅ Stage 6a | `auth_middleware` 验 HS256 JWT (env `QIANXUN_JWT_SECRET`), 跳过 `/`, `/v1/system/health`, `/v1/system/status`, `/_ui/*` |

**合计**: 25+ 路由 (不是 docs 写的 11 条).

### 4.2 是否接 AgentLoop?

**未接**. 证据:
- `qianxun/src/daemon/mod.rs:159` `processing_loop_enabled: false`
- `qianxun/src/daemon/router.rs:637-737` `prompt_handler` 注释明确: "**Stage 2 不接** `processing_loop::handle_user_message` (Stage 3 接入). 也不接 `tool_result` 事件 (Stage 3 在工具执行路径上发射)"
- `prompt_handler` 直接 `provider.stream_completion` → `SseEventBuilder::from_llm_event` → mpsc → SSE 帧
- 工具执行未跑: `SseEvent::ToolUseComplete` 发出去就完事, 没有 `SseEvent::ToolResult` 反馈, LLM 不会看到工具输出

### 4.3 AppState 与 SessionRuntime

- `AppState` (`mod.rs:35-57`): 持有 `agent_host` / `config` / `provider` / `tools` (空) / `memory` (in_memory) / `skills` (空) / `shared` / `store` / `llm_providers` / `shutdown_tx` / `processing_loop_enabled: false`
- `SessionRuntime` (`session_runtime.rs:37-76`): 持有 `agent_loop: AgentLoop` + `conversation: Conversation` + `provider/tools/memory/skills` 共享引用 + `last_active_at: RwLock<DateTime>`. **但 router 层不调用 agent_loop** — 每次 prompt 临时构造 `Conversation::new(None)`, 完全绕开 runtime 里的 state.
- `SessionStore` (`persistence.rs` 515 行): 3 张表 (`daemon_sessions` / `daemon_event_log` / `daemon_conversation_snapshots`), `create()` / `list_active()` / `load_latest_snapshot()` / `save_snapshot()` / `append_event()`. `restore_from_disk()` 在 `agent_host.rs:212-277` **存在但 conversation 反序列化为空** — 注释 "Stage 3 简化, conversation 字段无法从 JSON 还原".

### 4.4 瓶颈 / Gap

1. **prompt 路径不执行工具** (最严重) — `SseEvent::ToolUseComplete` 后无 `ToolResult` 反馈, 实际等于只读 LLM 文本流
2. **prompt 路径不接 memory / skills / workspace 上下文** — `build_request` 三个空串
3. **conversation 不持久** — `save_snapshot` 写的是 `{"messages":[],"stage":"stage3_placeholder"}` 占位 JSON
4. **AppState.tools 是空 ToolRegistry** — 注释 "Stage 1 = 空 registry, builtin register_all 留 Stage 2/3"
5. **AppState.skills 是空 SkillManager** — 同上
6. **AppState.memory 是 in_memory SQLite** — 注释 "真实 ~/.qianxun/mem.db 留 Stage 3"
7. **AppState.provider 是 active 单实例** — 多 provider 走 `LlmProviderManager` in-memory cache, 实际 LLM 推理仍用 `active_id` 那一个
8. **Web UI 是 SvelteKit build, 但后端 8 路由返回 stub** — UI 渲染了 LLM 配置 / Skills 列表 / MCP 列表 / Tools 列表, 数据是空数组

---

## 5. 前端 / 客户端矩阵

| 入口 | docs 状态 | 代码状态 | 文件 | 关键能力 |
|---|---|---|---|---|
| **`qx`** (CLI REPL) | ✅ Phase 1 | ✅ | `qianxun/src/cli/{cli,config,output,run}.rs` | rustyline REPL, 内嵌 AgentLoop |
| **`qx --acp-mode`** | ✅ Phase 2 | ✅ | `qianxun/src/acp/*.rs` (1900 行, 8 文件) | JSON-RPC 2.0 over stdio, session 管理, 双向请求, forwarding tools |
| **`qx daemon`** | 🟡 | 🟡→✅ Stage 1-7a | `qianxun/src/daemon/*.rs` (4800 行) | 25+ 路由, JWT, SSE 12 事件, SvelteKit Web UI |
| **`qx --daemon-url`** / 默认探测 | 📋 | ✅ | `qianxun/src/client/mod.rs` (1211 行) | thin REPL, 探测 `127.0.0.1:23900` health, 3s 超时, Bearer token, 3-30s 退避重连, 8 单元测试 |
| **TUI (Stage 3d)** | ✅ | ✅ | `qianxun/src/tui/mod.rs` (1713 行) | ratatui, 脏标记渲染, 流式 delta, 工具折叠, 性能 ~447µs/帧 |
| **VPS Web (Stage 4b/7)** | 📋 | 🟡 Stage 6b | `qianxun/src/server/static_ui/` (vanilla JS login + index) | 仅 login 页; chat/team/node 仍 TODO |
| **Daemon Web UI (Stage 7a)** | 📋 | ✅ 已 build | `qianxun/src/daemon/ui/build/` (SvelteKit) | 4 路由 (/llm, /mcp, /skills, /tools, /system, /settings) 已渲染, 走 REST |

**main.rs 路由逻辑** (`main.rs:248-336`):
```
--server     → server::run()       [VPS]
--daemon     → daemon::run()        [需 QIANXUN_JWT_SECRET]
--daemon-url → client::run_thin_repl(daemon_url, token)
默认 (无 --standalone): 探测 localhost:23900
  ├─ 探测成功 → client::run_thin_repl
  └─ 探测失败 → 回退 standalone
  - --acp-mode + 探测成功: 提示"ACP thin-client 模式尚未实现" → 回退 standalone
  - --acp-mode + 探测失败: 走 acp::run_acp_server (内嵌 AgentLoop)
  - 默认 + 探测失败: cli::run::run_repl (内嵌 AgentLoop)
```

**ACP thin-client 路径** (`main.rs:288-294`): **未实现**, 显式 print "ACP thin-client 模式尚未实现, 暂以 standalone 模式运行".

---

## 6. 当前缺口 — 距"多 Agent 协作"

距"多 Agent 协作"还差以下 8 个具体模块 (按重要性排序):

| # | 模块 | 现状 | 缺口 | 大致工作量 |
|---|---|---|---|---|
| 1 | **AgentLoop pattern dispatch** | engine.rs 写死 React, plan/reflect/workflow 仅类型 | 在 `processing_loop::handle_user_message` 加 `match self.config.pattern`, 写 `handle_plan_and_execute` / `handle_reflective` / `handle_workflow` | 1-2 周 |
| 2 | **Daemon prompt 路径接 processing_loop** | daemon `prompt_handler` 直接 stream, 不执行工具 | 改用 `OutputSink` adapter, 让 daemon SSE 12 事件覆盖 `tool_result`, 同时 `runtime.conversation` 持久化 | 1 周 |
| 3 | **Daemon memory/skills 注入** | `build_request(&[], "", "", "", agent)` 三个空串 | `prompt_handler` 调 `memory.build_context()` + `skills.build_catalog_prompt()` + `skills.build_injections()`, 接到 `build_request` | 0.5 周 |
| 4 | **多 Agent runtime 模型** | 每个 session 一个 AgentLoop, 各自 Conversation | 加 `AgentGroup` / `SubAgent` 概念: 父 Agent 派生子 Agent (子进程? 子 Session? 子 Conversation? 共享 memory?) | **架构决策**, 2-4 周 |
| 5 | **Agent 通信协议** | 单 Agent 内部工具调用, 无 Agent-to-Agent 消息 | 定义 AgentMessage {from, to, payload, correlation_id}, 经 memory bus 或 mpsc | 1-2 周 |
| 6 | **持久化的 Conversation 反序列化** | SessionStore 写的是占位 JSON, `restore_from_disk` 不真还原 | `Message` 全字段 derive Serialize/Deserialize, `restore_from_disk` 用 store.load_latest_snapshot 还原 conversation | 0.5 周 |
| 7 | **AppState 真实子系统** | tools / skills / memory 全是空 / in_memory | 启动时调 `ToolRegistry::register_builtin()` + `SkillManager::load_all()` + `MemoryCore::open(path)`, 让 daemon 真正能用 builtin / skills / 持久 memory | 0.5 周 |
| 8 | **Tool 权限门控 (ToolCategory)** | ToolCategoryFilter 已实现, 实际只有 plan/reflect/workflow 模板用, daemon 不接 | daemon `prompt_handler` 接受 `pattern` 参数, 按 pattern 选 filter, Plan 阶段拒绝 write 类 | 0.5 周 |

**额外 2 个底层** (Phase 4a 必经):
- **ACP thin-client** (`main.rs:288` 显式未实现)
- **Conversation 持久化 schema** (snapshot 序列化 / 恢复 / 迁移)

---

## 7. 硬约束清单

来自 `CLAUDE.md` + 实际代码观察:

| 类别 | 约束 | 来源 |
|---|---|---|
| 语言 | Rust 2024 edition, MSRV 1.85 | `CLAUDE.md:13` |
| 异步 | tokio (full) | `CLAUDE.md:14` |
| HTTP client | `reqwest` (`default-features=false`, features = `json,stream,rustls,webpki-roots`), 纯 Rust TLS, 无 OpenSSL/cmake | `CLAUDE.md:31` |
| 依赖策略 | 禁止传递依赖 > 100 个; 引入新 crate 必须评估传递依赖树 (30 阈值) | `CLAUDE.md:30, 32` |
| LLM SDK | 禁引入 Anthropic / OpenAI SDK, 直用协议 (`provider/anthropic_compat.rs`) | `CLAUDE.md:33` |
| MCP 实现 | 不引 MCP SDK, 手写 JSON-RPC 2.0 over `serde_json` (mcp/transport.rs 478 行) | `docs/mcp-design.md:485-491` |
| 部署 | 私有部署, Daemon 单机单实例, 监听 `127.0.0.1` | `docs/architecture.md:135-140, 374` |
| API Key | 磁盘加密 AES-GCM, 密钥来自系统 keychain (Stage 7a 简化为 in-memory, 进程重启会丢) | `docs/daemon-design.md:5, 461` |
| LLM 默认 | DeepSeek Anthropic 兼容 API, 模型 `deepseek-v4-flash`, env `DEEPSEEK_API_KEY` | `CLAUDE.md:23-26` |
| System Prompt | 4 段拼接 base + memory + skills_catalog + skill_injections (固定顺序) | `conversation.rs:51-89` |
| Token 估计 | 字符长度近似 (避免 tiktoken-rs) | `docs/architecture.md:655` |
| Session ID 格式 | `sess_YYYYMMDD_HHMMSS_microsec` (例 `sess_20260602_225000_123456`) | `agent_host.rs:117` |
| 数据库 | SQLite (rusqlite bundled, vtab + column_decltype features) | `qianxun-memory/src/db.rs:282` + `docs/memory-design.md:323` |
| Web 框架 | axum 0.8, tower 0.5, tower-http 0.6 (tokio 栈) | `docs/daemon-design.md:812-815` |
| Web UI | Svelte 5 + Vite + Tailwind + shadcn-svelte (daemon Web Admin Console) | user_profile (2026-06-02) |
| 移动端栈 (未来) | Flutter (不直接影响 web 端) | user_profile (2026-06-02) |

---

## 8. 借鉴适配性 (对接 hermes-agent 视角)

**天然适合借鉴**:
- `qianxun-core/src/agent/engine.rs::processing_loop::handle_user_message` — 489 行, 完整的 React 主循环 + context compression + tool execute + cancel, 可直接抽离为 "agent runtime kernel"
- `qianxun-core/src/agent/context/` (compact + normalize + window) — 上下文管理独立模块, 复用度高
- `qianxun-core/src/tools/{mod,builtin}.rs` — ToolCategoryFilter + execute_async_with_filter 是清晰的 "tool permission gate", 适合作为多 agent 共享基础设施
- `qianxun-core/src/skills/mod.rs` — SkillManager + SkillWatcher + auto_select (关键词匹配) + 四层注入, 是 "agent context augmentation" 的成熟实现
- `qianxun-memory/src/lib.rs` — `MemoryObserver` trait + MemoryCore (observe/remember/search/build_context/session_start/session_end) + 8 表 SQLite + FTS5 trigger + HybridSearch 权重, 已是完整 RAG 骨架
- `qianxun-core/src/mcp/server_manager.rs` — 进程生命周期 + 崩溃保护 + tool 注册, 可作为 "external capability bus" 给多 agent 用
- `qianxun/src/daemon/sse.rs` — 12 事件契约 (与 shared-contract 严格一致), client/server 端 SseEvent enum 字段一一对应 (`qianxun/src/client/mod.rs:108-174` vs `qianxun/src/daemon/sse.rs:24-86`), 是 multi-agent 通信协议的可用起点
- `qianxun/src/daemon/persistence.rs` — SessionStore 3 表 (sessions / event_log / snapshots) 是 agent session 持久化的成熟 schema, 可扩展为 multi-agent 共享 store
- `qianxun/src/daemon/llm_providers.rs` — in-memory LLM provider 池 + CRUD + activate + test, 是 multi-agent 共享 LLM 配额的基础
- `qianxun/src/daemon/router.rs::auth_middleware` — HS256 JWT + skip list, 可作 multi-agent 鉴权基础
- `qianxun/src/client/mod.rs` — 1211 行 thin client (探测 / 重连 / Bearer / SSE 解析 / mock HTTP 测试), 适合做 multi-agent 的 sidecar client

**需要绕开 / 不直接借鉴**:
- **`AgentPattern` 类型骨架 (plan.rs / reflect.rs / workflow.rs)** — 类型完整但 dispatcher 未接, 直接借鉴会得到"假像支持 4 模式实际只跑 React"的反模式; 要么重写 dispatcher, 要么不引入
- **`system_prompt.rs::build_system_prompt` 的 `mode: &str` 参数** — 简陋的 `"plan" / "auto"` 二元切换, 真正多 agent 应基于完整 `AgentPattern` enum 构造
- **`AppState.tools` / `skills` / `memory` 在 daemon 启动时是空 / in_memory** — 文档说"已接", 代码说"留 Stage 2/3", 借鉴时**必须**先把这些子系统从空初始化改成真实初始化
- **`daemon/router.rs::prompt_handler` 写死不执行工具** — 这是 Stage 2 的临时实现, 借鉴时**必须**改用 `OutputSink` adapter 走 `processing_loop`
- **VPS Server 走 5 阶段增量 (Stage 1-6b), 大部分 API 已被 `team_db` / `ws_hub` / `outbox` 覆盖, 但**未与本地 Daemon 联调** — 文档的 "Daemon ↔ VPS WS" 路径未做 e2e 测试
- **TUI 性能数据** — 1MB 流 ~447µs/帧 是**单线程 80×24 视口**下的数字, multi-agent 多 session 并发时未必能保持, 不要直接外推
- **CLI `qianxun/src/cli/`** — 旧 REPL, "行将迁移" 状态, 不要在 multi-agent 路径上扩展它
- **Provider 池实际是 active 单实例** — `LlmProviderManager` 维护多 provider 但 daemon 用的是 `state.provider` (一个 Arc), 多 agent 真正负载分担需要先把 `state.provider` 改成 provider 池查询

---

## 9. 证据索引 (核心 file:line 引用)

| 论断 | 引用 |
|---|---|
| workspace 3 crate | `Cargo.toml` ([未读, 推断]) + `CLAUDE.md:37-82` |
| 实际阶段远超 docs | `qianxun/src/daemon/mod.rs:30-34` 注释 "Stage 6a / 7a 已实现", `qianxun/src/server/mod.rs:9-46` 注释 "Stage 1-6b 范围已实现" |
| plan/reflect/workflow 未接 | `qianxun-core/src/agent/{plan,reflect,workflow}.rs` 全文 + `engine.rs:83-94` `handle_user_message` 签名不含 `pattern` |
| daemon `prompt_handler` 不走 processing_loop | `qianxun/src/daemon/router.rs:637-737` + 注释 line 635 "**Stage 2 不接** `processing_loop::handle_user_message`" |
| `AppState.processing_loop_enabled: false` | `qianxun/src/daemon/mod.rs:159` |
| `AppState.tools` / `memory` / `skills` 是空 / in_memory | `qianxun/src/daemon/mod.rs:100-104` + 注释 "Stage 1 = 空 / in_memory" |
| Daemon 25+ 路由 | `qianxun/src/daemon/router.rs:81-125` |
| SessionStore 3 表 | `qianxun/src/daemon/persistence.rs:1-50` (file 515 行) |
| `restore_from_disk` 不真反序列化 | `qianxun/src/daemon/agent_host.rs:243-249` 注释 + `let _ = conversation_json;` line 272 |
| thin client 完整 | `qianxun/src/client/mod.rs:1-1211` (12 个 test functions, Bearer + 自动重连 + URL normalize) |
| main.rs 路由 | `qianxun/src/main.rs:248-336` |
| ACP thin-client 未实现 | `qianxun/src/main.rs:288-294` print "ACP thin-client 模式尚未实现" |
| JWT auth | `qianxun/src/daemon/router.rs:177-284` Claims + auth_middleware |
| Web UI 已 build | `qianxun/src/daemon/ui/build/` (SvelteKit), `screenshots/01-llm.png..04-tools.png` |
| Memory 8 表 + FTS trigger | `qianxun-memory/src/db.rs:282` (file 283 行) + `memory-state.md:35` "3 个 trigger" |
| Memory 18 集成测试 pass | `memory-state.md:42-51` |
| Skills 397 行完整 | `qianxun-core/src/skills/mod.rs` (16 pub 方法, 全文 397 行) |
| MCP 完整 | `qianxun-core/src/mcp/{client,server_manager,tool_wrapper,transport,config,mod}.rs` 6 文件, 1100+ 行 |
| TUI 1713 行 + 性能 | `qianxun/src/tui/mod.rs` (1713 行) + `tui-architecture.md:43-47` |
| VPS Server 4500+ 行 Stage 1-6b | `qianxun/src/server/*.rs` 行数合计 (mod 1002 + auth 147 + admin 176 + auth_ws 212 + messages 315 + outbox 317 + rate_limit 583 + team_db 588 + ws_hub 1175) = 4515 行 |
| 4 内置 workflow 模板 | `qianxun-core/src/agent/workflow.rs:50-54` (`WorkflowManager::new`) |
| LLM ProviderManager CRUD | `qianxun/src/daemon/llm_providers.rs:1-600` |
| ToolCategoryFilter | `qianxun-core/src/tools/mod.rs:15-66` |
| `processing_loop::handle_user_message` 签名 | `qianxun-core/src/agent/engine.rs:83-94` (10 个参数, 无 pattern) |
| 4 段 system prompt 拼接 | `qianxun-core/src/agent/conversation.rs:51-89` |
| Session ID 格式 | `qianxun/src/daemon/agent_host.rs:117` `format!("sess_{}", now.format("%Y%m%d_%H%M%S_%6f"))` |
| 阶段路线 A-G | `docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md:10-220` |

---

## 10. 总结判断

1. **docs/CLAUDE.md 严重低估 Daemon/VPS/Thin-Client** 实际进度 — 应回写 docs/architecture.md 和 docs/CLAUDE.md, 把 Stage 1-7a 实际完成的事写清楚, 把"`processing_loop` 未接"作为唯一硬缺口标注
2. **docs 高估 Phase 3b** — plan/reflect/workflow 是孤儿模块, 抄过来会"看起来支持 4 模式实际跑 React". `agent-pattern-design.md` §2.4 的 dispatcher 描述与代码不符
3. **多 Agent 协作的 8 个缺口** (见 §6): 1 个架构决策 (multi-agent runtime 模型) + 7 个工程任务
4. **对 hermes-agent 借鉴**: 大部分核心模块 (engine / tools / skills / memory / mcp / sse / persistence / client) 天然适合, 需绕开的是 phase 3b 假象、daemon 空初始化子系统和 ACP thin-client 缺失
5. **最低成本演进路径**: 先把 `AppState.tools/memory/skills` 从空改成真实初始化 + prompt_handler 改用 OutputSink adapter 走 processing_loop + 修 conversation 持久化 → 即可得到一个能执行工具的完整 daemon → 在此基础上加 `AgentGroup` / `SubAgent` 抽象即可支持多 agent

> [待确认] 1) `qianxun-core/src/mcp/transport.rs` 是否真支持 HTTP/SSE 传输, 还是仅 stdio (mcp-state.md:35 写"无") — 未读全文核实. 2) `qianxun-core/src/provider/anthropic_compat.rs` 是否真的覆盖了 DeepSeek 之外的所有 Anthropic 兼容 API — 未读全文. 3) `qianxun/src/daemon/ui/build/` 实际渲染的页面是否真的能驱动 4 个 /v1/* 端点 — 截图存在但 runtime 8 路由返 stub, 实际 UI 上可能是空状态 — 需运行 daemon 验证.

---

报告路径: `E:\git\maxu\qianxun\.mavis\plans\qianxun-analysis.md`
deliverable: `C:\Users\maxu\.mavis\plans\plan_d5916b74\outputs\qianxun-analysis\deliverable.md`

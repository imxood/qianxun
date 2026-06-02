# Hermes Agent Kanban / Team / Multi-Agents 架构分析

> 范围: `E:/git/ai/hermes-agent` (Python 实现)
> 目的: 为 qianxun (Rust) 的多 Agent 架构设计提供参考
> 分析时间: 2026-06-02

---

## 1. 执行摘要

Hermes 把"多 Agent 协作"实现成了一个**任务看板驱动的多 Profile 实例**模型: 看板 (Kanban) 是唯一的真相源, 任务以 DAG 形式组织, 每个任务绑定一个 Profile (隔离的工作目录 + 自己的 gateway + 自己的 skills) 作为执行单元; 团队 (Team) **不是一个独立类型**, 而是一组同时活跃的 Profile 实例通过共享 Kanban 数据库协作; 多 Agent 调度则由 `delegate_task` (子 Agent 树) + `kanban_swarm` (并行 worker DAG) 两种正交机制组合实现. 这种"一切以任务为中心"的设计让 Hermes 用相对克制的代码量覆盖了从单聊到 50+ 工人并行的全谱场景, 是 qianxun 借鉴的主要价值点.

---

## 2. 项目快照

| 维度 | 数值/状态 |
|---|---|
| 语言 | Python 3.12, 大量 `from __future__ import annotations`, type hints 完善 |
| 规模 | 50+ 工具模块, 50+ agent 内部模块, 14+ Kanban 专用文件 |
| LLM 抽象 | OpenAI 兼容 + Anthropic 适配 + 自家 auxiliary client |
| 存储 | SQLite (Kanban), 文件 (workspaces, attachments, profiles) |
| UI | TUI (`ui-tui/`), Web (`web/`), Website (`website/`), Dashboard plugin (`plugins/kanban/dashboard/`) |
| 入口 | `cli.py` (718KB 单文件) + 多个 gateway 后台进程 |
| 部署 | Docker + s6-overlay, 支持多 Profile 隔离 |

关键目录:
- `hermes_cli/kanban_*.py` — 看板内核 (db, swarm, decompose, specify, diagnostics)
- `hermes_cli/profiles.py` (60KB) — Profile 管理
- `tools/kanban_tools.py` (56KB) — 看板的 agent 工具表面
- `tools/delegate_tool.py` (120KB) — 子 Agent 委派
- `gateway/run.py` (925KB) — 单实例 gateway 运行时
- `agent/conversation_loop.py` (264KB) — 顶层 agent loop
- `plugins/kanban/dashboard/plugin_api.py` (93KB) — 看板 dashboard 插件
- `ui-tui/` — TUI 客户端

---

## 3. Kanban 系统

### 3.1 数据模型 (看板内核)

看板是一个**多表 SQLite 数据库**, 核心表 6 张 (`hermes_cli/kanban_db.py:917-1069`):

| 表 | 关键字段 | 作用 |
|---|---|---|
| `tasks` | id, title, body, assignee, status, priority, workspace_kind, workspace_path, started_at, completed_at, last_heartbeat_at | 任务主体 |
| `task_links` | parent_id, child_id (PK) | DAG 父子关系 |
| `task_comments` | id, task_id, author, body, created_at | 评论, 也用作"黑板" |
| `task_events` | id, task_id, run_id, kind, payload, created_at | 事件流 (审计 + 实时推送) |
| `task_runs` | id, task_id, profile, step_key, status, claim_lock, claim_expires, worker_pid, max_runtime_seconds, last_heartbeat_at, started_at, ended_at, outcome, summary, metadata, error | **执行历史** — 每次重试产生新 row |
| `task_attachments` | id, task_id, filename, stored_path, content_type, size, uploaded_by | 附件 |

两个关键设计:
1. **任务与运行解耦**: 同一个 task 可以有多个 `task_runs` 记录, 用于重试/崩溃恢复. 任务级字段 (status, started_at) 表达"逻辑状态", 运行级字段 (claim_lock, heartbeat, worker_pid) 表达"物理执行状态" (hermes_cli/kanban_db.py:1009-1015 注释明确说明此意图).
2. **黑板用评论实现**: 共享状态不引入新表, 而是用带前缀的 `task_comments` (如 `[swarm:blackboard] {"key":"x","value":y}`) (hermes_cli/kanban_swarm.py:26, 226-240). 简单粗暴, 复用已有通知/审计/dashboard.

### 3.2 状态机

任务状态 (`task_runs.status` 注释在 `hermes_cli/kanban_db.py:1022`):
```
running | done | blocked | crashed | timed_out | failed | released
```

任务级状态 (`tasks.status`) 是更高层的逻辑状态: `triage | ready | in_progress | done | blocked`. 看板通过 `recompute_ready` (hermes_cli/kanban_db.py: 函数列表) 在 children 状态变化时重新计算 parent 状态.

### 3.3 工具表面 (给 Agent)

`tools/kanban_tools.py` 把看板操作包装成 LLM tool calls, 关键分类 (hermes_cli kanban_tools.py:49-90):

- **Worker 工具** (要求 `HERMES_KANBAN_TASK` 环境变量): `kanban_complete`, `kanban_block`, `kanban_heartbeat`, `kanban_comment` — 一个 worker 只能操作自己绑定的那个 task
- **Orchestrator 工具** (要求 `kanban` toolset 启用): `kanban_list`, `kanban_unblock`, `kanban_create`, `kanban_link`, `kanban_assign` — 调度者才能看全局看板
- **CLI 路径** (`hermes kanban ...`): 人类直接操作, 绕过 agent

**安全护栏** (kanban_tools.py:132-161): worker 调用 `kanban_complete` 时强制校验 `task_id` 必须等于环境变量里的 `HERMES_KANBAN_TASK`, 防止 prompt-injected worker 篡改兄弟任务.

**自动心跳桥** (kanban_tools.py:200-260): worker 内部 tick `_touch_activity` 时, 自动 60s 内最多一次写 `tasks.last_heartbeat_at`, 避免 dispatcher 看错成 stale. 这是一个非常细节但实用的设计.

### 3.4 Kanban as multi-board

Hermes 支持**多个 Kanban 看板同时存在** (hermes_cli/kanban_db.py 函数列表中的 `boards_root`, `create_board`, `set_current_board`, `current_board_path` 等), 通过 symlink + per-board sqlite 文件实现多租户/多项目隔离. 每个 board 独立 `~/.hermes/boards/<slug>/kanban.db`.

---

## 4. Team 系统

### 4.1 核心洞察: "Team" 不是类型, 是 Profile 集合

Hermes **没有独立的 Team 类**. "Team" 的语义是由一组**同时活跃的 Profile 实例** + **共享 Kanban DB** 共同表达. 关键证据:

1. `hermes_cli/profiles.py` (60KB) 是 Profile 的真理源. Profile 包含: name, description, 独立的工作目录 (`~/.hermes/profiles/<name>/`), 独立的 gateway service (s6-overlay), 独立的 skills, 独立的 home directory.
2. `ProfileInfo` 类 (profiles.py 函数列表) 表达元数据.
3. `_maybe_register_gateway_service` / `_maybe_unregister_gateway_service` (profiles.py 函数列表) 说明每个 Profile 启动时会注册为独立的 s6 服务, 多 Profile 即可同时运行.
4. **任务通过 `assignee` 字段绑定到 Profile name** (hermes_cli/kanban_db.py:921, kanban_tools.py 中 `_normalize_profile` 函数), 没有 team_id, 没有 role_id.

所以 "team" 在 Hermes 是**横切关注点**而非第一类对象. 团队成员就是 Profile, 团队协作就是它们各自领取 Kanban 任务并写结果. 这种设计让"团队规模"和"任务编排"完全解耦.

### 4.2 Profile 的隔离边界

每个 Profile 拥有:
- 独立 `~/.hermes/profiles/<name>/` 工作目录
- 独立 s6 服务 + 端口 (gateway 端口)
- 独立 skills 集合
- 独立 skills-hub 缓存 (`skills_hub.py` 141KB)
- 独立 OAuth credentials (`credential_pool.py` 99KB)
- 独立 MCP OAuth (`mcp_oauth.py` 30KB)

跨 Profile 通信**只能通过 Kanban** (写 task_comments, 创建子任务, 转移 assignee). 没有直接的 RPC 通道.

### 4.3 Decompose / Specify: 把模糊任务变成图

`hermes_cli/kanban_decompose.py` (16KB, 477 行) 是团队协作的入口:
- 输入: 一个 `triage` 状态的任务
- 处理: 调 auxiliary LLM, 传入"可用 profile 名册 + 每个的描述 + 默认 fallback", 让 LLM 返回 JSON: `{fanout, rationale, tasks: [{title, body, assignee, parents}]}`
- 输出: 把 LLM 决定的图**原子地**写入 Kanban, 链接到 root, 然后 root 状态 `triage -> todo`. root 的 assignee 永远是 orchestrator profile.

`kanban_specify.py` (8KB) 是 fanout=false 时的简化版, 只把 task body 收紧.

关键不变量 (kanban_decompose.py:9-14 注释): "root task stays alive and becomes the parent of every leaf child, so when the whole graph completes the root wakes back up — its assignee (the orchestrator profile) gets a chance to judge completion and add more tasks if the work isn't done yet." 即**只有 orchestrator 能决定工作流是否真的结束**, 这是个非常优雅的设计.

### 4.4 Swarm: 并行 worker DAG

`hermes_cli/kanban_swarm.py` (9KB, 279 行) 实现了**显式的并行 + 验证 + 综合**模式:

```
planning root (completed immediately) ← 共享黑板
├── parallel specialist workers (ready)
└── verifier (todo until all workers done)
    └── synthesizer (todo until verifier done)
```

特征:
- **没有第二个调度器**, 完全复用 Kanban. 写一堆 task + 用 task_links 表达依赖即可 (hermes_cli/kanban_swarm.py:2-15 注释).
- 黑板是 root 任务上的**结构化 JSON 评论** (`[swarm:blackboard] {"key":..,"value":..}`). 后续评论覆盖前一个 (hermes_cli/kanban_swarm.py:243-267 `latest_blackboard`).
- Verifier 是个**门控 gate**, 必须在 metadata 里写 `{"gate":"pass"}` 才能让 synthesizer 解锁 (hermes_cli/kanban_swarm.py:178-179 注释).
- 幂等性: 通过 `idempotency_key` 参数 + 黑板恢复实现重入 (hermes_cli/kanban_swarm.py:130-144).
- 可观察性: 一个 Swarm 出来后 root 任务的 `task_events` 流能完整看到 worker 状态切换.

---

## 5. Multi-Agents (子 Agent 与委派)

### 5.1 `delegate_task` 工具

`tools/delegate_tool.py` (120KB) 是 LLM 调用的"开子进程跑任务"工具, 关键能力 (函数列表):

| 能力 | 函数/常量 | 说明 |
|---|---|---|
| 暂停开子 | `set_spawn_paused` / `is_spawn_paused` | 紧急刹车, 防止递归展开 |
| 深度限制 | `_get_max_spawn_depth` | 防止无限递归委派 |
| 并发限制 | `_get_max_concurrent_children` | 同时在飞的子任务数 |
| 子超时 | `_get_child_timeout` | 单个子任务最大时长 |
| 委派总开关 | `_get_orchestrator_enabled` | 一键关闭委派 |
| MCP 工具继承 | `_get_inherit_mcp_toolsets` + `_expand_parent_toolsets` | 子 agent 自动继承父的 MCP |
| 子 agent 注册 | `_register_subagent` / `_unregister_subagent` / `list_active_subagents` | 实时跟踪 |
| 中断 | `interrupt_subagent` | 用户取消 |
| 系统提示 | `_build_child_system_prompt` | 自动拼装 |
| 进度回调 | `_build_child_progress_callback` | 流式回传 |
| 事件 | `DelegateEvent` (str enum) | typed 事件用于流 |

`DelegateEvent` 是关键的流式设计 — 子 agent 的进度/中间结果/工具调用/错误都能通过 typed 事件实时回传给父 agent 渲染.

### 5.2 Subagent 模型

- 子 agent 与父 agent **共享环境变量 + 凭据池 + MCP OAuth**, 但工作目录独立.
- 子 agent **不直接接 Kanban** (除非显式 set `HERMES_KANBAN_TASK`). 它们靠 `delegate_task` 的入参/出参通信.
- 父 agent 可以**中断**子 agent, 但子 agent 之间不能直接对话 (只能通过 Kanban 间接协作).

### 5.3 三种协作模式组合

Hermes 的多 Agent 协作是**三种正交机制**的组合:
1. **Delegate** (子进程) — 短任务, 父子紧耦合, 同步/流式回传
2. **Swarm** (Kanban DAG) — 中长任务, workers 并行, verifier 门控, synthesizer 综合
3. **Multi-Profile** (独立 gateway 实例) — 长期角色 (如 "techlead", "researcher"), 各自独立服务, 共享 Kanban

这三种模式在**任务生命周期**, **通信机制**, **失败恢复** 上都不同, 适合不同场景. qianxun 可以直接借鉴这种"分层 + 正交"的设计.

---

## 6. UI 集成

### 6.1 TUI 端 (`ui-tui/`)

TypeScript 写的终端客户端. 通过 HTTP + SSE 与 daemon 通信, 渲染聊天/任务流. 关键组件:
- `app/spawnHistoryStore.ts` (5KB) — 跟踪子 agent 派生历史
- 各种 Svelte 组件用于展示消息流

### 6.2 Web Dashboard 插件 (`plugins/kanban/dashboard/plugin_api.py`, 93KB)

- 作为 s6 进程运行, 暴露 HTTP
- 实时订阅 `task_events` 表的更新 (用 `kanban_notify_subs` 表)
- 提供 Kanban 看板的拖拽、评论、状态切换
- 与 TUI 共享同一份 SQLite 数据

### 6.3 Gateway 通知

`gateway/` 通过 `kanban_notify_subs` 表实现多平台通知订阅 (Telegram, Discord, Feishu, Slack 等), task 状态变化时通过 watcher 推送.

---

## 7. 关键设计模式 (qianxun 可借鉴的)

### 模式 1: 任务/运行解耦 (Task / Run Decoupling)

**场景**: 长任务需要重试/崩溃恢复, 但不能丢失任务级历史.
**Hermes 实现**: `tasks` 表存"逻辑状态", `task_runs` 表存"物理执行" (hermes_cli/kanban_db.py:1009-1015). 每次委派新建 run, 失败时新建 run 重试, 任务级状态由 recompute 触发.
**可移植性**: ⭐⭐⭐⭐⭐ 直接适用. qianxun 的 `AgentRun` 概念可以原样借鉴 — 把"任务是什么"和"任务被谁在跑"分到两张表/一个 struct 的两个独立生命周期.
**qianxun 落地**: 在 `qianxun-core` 加 `kanban` 模块, 数据模型直接照搬.

### 模式 2: DAG 黑板 (Structured Comments as Blackboard)

**场景**: 并行 worker 之间的共享状态, 不想引新服务.
**Hermes 实现**: 用 `[prefix] json` 格式的 task_comments 实现, 带前缀避免污染普通评论 (hermes_cli/kanban_swarm.py:26, 226-240).
**可移植性**: ⭐⭐⭐⭐ 适用, 但 Rust 实现里建议用**单独的 `blackboard` 表**而非塞到 `comments` 里, 类型安全更好.
**qianxun 落地**: `blackboard(key TEXT, value JSONB, author TEXT, updated_at INTEGER, task_id)`, 配上一个 `latest_blackboard(task_id) -> HashMap` 函数.

### 模式 3: Worker 工具 vs Orchestrator 工具 (Capability Gating)

**场景**: 防止子 agent 篡改全局状态.
**Hermes 实现**: `HERMES_KANBAN_TASK` 环境变量 + profile toolset 配置双闸门, 工具内部用 `_check_kanban_mode()` / `_check_kanban_orchestrator_mode()` 区分 (kanban_tools.py:49-90). Worker 调用 `kanban_complete` 时强制校验 task_id 归属.
**可移植性**: ⭐⭐⭐⭐⭐ 强烈建议照搬. 这条护栏比"RBAC 角色"简单得多, 但防住了最常见的 prompt injection 攻击 (子 agent 写脏其他任务).
**qianxun 落地**: 在 `qianxun-core` 的 Kanban trait 里加 `fn scope(&self) -> WorkerScope { Worker/Orchestrator }`, 工具调用前置检查.

### 模式 4: LLM-Driven Decompose (图生成交给 LLM)

**场景**: 用户甩个模糊需求, 系统自动拆解成可执行图.
**Hermes 实现**: `kanban_decompose.py:52-109` 是一段精心设计的 system prompt, 让 auxiliary LLM 返回结构化 JSON (`fanout`, `tasks[].title/body/assignee/parents`), 然后原子写入. 包含 idempotency, 默认 assignee 回退, parent 索引清洗 (kanban_decompose.py:432-433).
**可移植性**: ⭐⭐⭐⭐ 适用, 但需要小改: Rust 这边把 LLM 输出的 JSON 严格 schema 校验 (kanban_decompose.py:349 `_extract_json_blob` 太宽松, Rust 应当用 typed 解析 + 报错).
**qianxun 落地**: `qianxun-core::kanban::decompose(prompt, roster) -> Vec<TaskNode>`, LLM 调用复用现有 `LlmProvider` trait.

### 模式 5: 黑板+Verilifier 门控 (Swarm Pattern)

**场景**: 一组并行 worker 各自出结论, 需要一个"裁判"决定整体是否通过, 然后一个"综合者"汇总.
**Hermes 实现**: Swarm 内置 root → workers → verifier → synthesizer 的固定 DAG, verifier 必须写 `{"gate":"pass"}` 才能让 synthesizer 解锁 (hermes_cli/kanban_swarm.py:178-179).
**可移植性**: ⭐⭐⭐⭐⭐ 强烈建议照搬. 这是一个"5 行代码级"的设计, 但解决了 80% 的"并行 AI 结果不可信"问题.
**qianxun 落地**: 直接对应到 `qianxun-core::agent::workflow.rs` 的 `WorkflowTemplate::Swarm` (如果存在) 或新增 `TeamRole::Verifier` / `TeamRole::Synthesizer`.

### 模式 6: 派生深度+并发限制 (Spawn Budget)

**场景**: 防止 LLM 自己递归开子 agent 把资源打爆.
**Hermes 实现**: `_get_max_spawn_depth`, `_get_max_concurrent_children`, `_get_child_timeout` 三个独立上限 (delegate_tool.py 函数列表), 还有总开关 `_get_orchestrator_enabled`.
**可移植性**: ⭐⭐⭐⭐⭐ 直接照搬. qianxun 现有的 `WorkflowTemplate` + `max_runtime_seconds` 已经有部分, 但需要补"per-team 并发"和"递归深度"两个独立维度.
**qianxun 落地**: `TeamConfig { max_depth, max_concurrent_children, child_timeout, orchestrator_enabled }`.

### 模式 7: 自动心跳桥 (Activity → Heartbeat)

**场景**: dispatcher 用 `last_heartbeat_at` 判断 worker 是否 stale, 但 worker 忙于 chunk 不会显式调 heartbeat tool.
**Hermes 实现**: `heartbeat_current_worker_from_env()` 在 agent loop 内部被 tick, 60s 限频, 失败也不抛 (kanban_tools.py:200-260). 完全是 best-effort.
**可移植性**: ⭐⭐⭐⭐ 适用, 改写为 tokio task 即可.
**qianxun 落地**: 在 `qianxun-core` 的 AgentLoop 里, 每次 LLM chunk / tool result 之后 spawn 一个轻量 task 写 heartbeat, 60s dedup.

### 模式 8 (补充): 多 Profile = 多独立进程 + 共享 Kanban

**场景**: 长期角色 (techlead, researcher) 需要持久化状态, 不应该每次"被叫醒"都从零开始.
**Hermes 实现**: 每个 Profile 是独立 s6 服务, 独立工作目录, 但通过 Kanban 通信. 类似"员工 + 工单系统"模型.
**可移植性**: ⭐⭐⭐ 适合, 但 Rust/qianxun 私有部署场景下, 独立进程的运维成本高, 可以**用 in-process Channel + 共享 State 模拟**, 效果差不多.
**qianxun 落地**: 在 Daemon 内部用 `tokio::sync::mpsc` + `Arc<ProfileRegistry>` 模拟; 真要跨进程时再上 s6.

---

## 8. 不推荐照搬的部分

1. **多 Profile + 多 Gateway 实例**: 运维复杂度高, 对私有部署不友好. qianxun 用单 daemon + 内部 profile registry 即可.
2. **插件系统 (`plugins/`)**: 93KB 的 dashboard plugin_api, 通过 s6 进程间通信. Rust 这边用 WebSocket + SvelteKit 即可, 没必要再发明.
3. **Lark/Telegram/Discord/Feishu 多平台适配器**: 250KB+ 的 auxiliary_client.py, 90+ 适配器. qianxun 应只先做 Feishu (用户已在用), 其他按需.
4. **OAuth 凭据池** (`credential_pool.py` 99KB): 多 provider 轮询的成熟方案, 但 qianxun 默认只 DeepSeek 一个, 后期再扩.
5. **ACPI 集成 (Zed)**: hermes-acp 是大块, qianxun 的 acp 模式自己实现即可.
6. **s6-overlay 多进程管理**: Docker 内才需要, qianxun 单 binary 部署更简单.
7. **大批量 preset skills (`skills/`, `optional-skills/`)**: 大量 MCP 适配 skill (calendar, spotify, video gen, browser). qianxun 应该让用户自己写, 不预装.

---

## 9. 端到端数据流

### 9.1 单 Profile 单任务流 (简单路径)

```
用户 / Telegram 消息
   ↓
gateway/run.py 接收
   ↓
agent/conversation_loop.py 处理
   ↓ (触发 kanban tool: kanban_create)
hermes_cli/kanban_db.py:create_task → SQLite
   ↓
gateway 通知 dispatcher (s6 进程)
   ↓
dispatcher 选 Profile + 启 worker
   ↓
worker 跑 AgentLoop → 调工具
   ↓
worker 调 kanban_complete → 更新 task.status, 写 task_runs.ended_at
   ↓
task_events 触发 webhook → dashboard / TUI / Telegram 推送
```

### 9.2 Swarm 路径 (并行多 Agent)

```
TechLead Profile 收到 "调研 X" 任务
   ↓
kanban_decompose.py → LLM 决定 3 个子任务
   ↓
写入 Kanban: root (assignee=techlead) + 3 个 child (assignee=researcher-{1,2,3})
   ↓
root 状态: triage → todo (root 继续作为审计/共享黑板)
   ↓
dispatcher 并行 fan out: 3 个 worker 启动
   ↓
worker 各自完成, 写 task_comments 上报进展
   ↓
verifier 任务 (depends_on=3 worker) 解锁
   ↓
verifier Profile 跑 → 写 metadata.gate=pass 或 block
   ↓
synthesizer 任务 (depends_on=verifier) 解锁
   ↓
synthesizer 综合所有 worker 结果 → 写最终产物
   ↓
root 唤醒 (所有 children 完成), techlead 决定是否接受, 或加新任务
```

### 9.3 关键不变量

- **Kanban 是唯一真相源**: 没有别的地方存任务状态. 重启 gateway / 重启 worker 都能从 Kanban 恢复.
- **每个变更都有 task_event**: 用于审计 + 实时推送.
- **失败 = 新 run, 不是覆盖 task**: 重试历史完整保留.
- **跨 Profile 通信只能通过 Kanban**: 强制走异步 + 可观察路径, 避免直接 RPC 的隐式耦合.

---

## 10. 证据索引

| 主题 | 文件 | 行号 |
|---|---|---|
| Kanban 数据模型 | `hermes_cli/kanban_db.py` | 917-1069 (SCHEMA_SQL), 986 (task_links), 992 (task_comments), 1000 (task_events), 1016 (task_runs), 1044 (task_attachments), 1059 (kanban_notify_subs) |
| Task/Run 注释 (设计意图) | `hermes_cli/kanban_db.py` | 1009-1015 |
| Swarm 实现 | `hermes_cli/kanban_swarm.py` | 全文 (279 行), 关键: 26 (BLACKBOARD_PREFIX), 77-223 (create_swarm), 226-240 (post_blackboard_update), 243-267 (latest_blackboard) |
| Decompose LLM 协议 | `hermes_cli/kanban_decompose.py` | 52-109 (system prompt), 110-121 (user template), 271-465 (decompose_task 主函数) |
| Worker vs Orchestrator 工具分流 | `tools/kanban_tools.py` | 49-90 (`_check_kanban_mode`, `_check_kanban_orchestrator_mode`), 132-161 (`_enforce_worker_task_ownership`), 200-260 (auto heartbeat) |
| Profile 是 team 抽象 | `hermes_cli/profiles.py` | 全文 (60KB), 关键: `ProfileInfo`, `list_profiles`, `_maybe_register_gateway_service` |
| Delegate 子 agent | `tools/delegate_tool.py` | 关键: `DelegateEvent` enum, `_get_max_spawn_depth`, `_get_max_concurrent_children`, `_get_child_timeout`, `_get_orchestrator_enabled`, `set_spawn_paused`, `interrupt_subagent` |
| Dashboard 插件 | `plugins/kanban/dashboard/plugin_api.py` | 全文 (93KB) |
| Gateway 入口 | `gateway/run.py` | 925KB 单文件, 包含 dispatcher 逻辑 |
| TUI | `ui-tui/src/app/spawnHistoryStore.ts` | 5KB |
| 黑板格式 | `hermes_cli/kanban_swarm.py` | 26, 226-240 |

---

## 11. 一句话总结 (给 qianxun)

**Hermes 用"任务为中心"取代"Agent 为中心": 任务有 DAG 关系, 每个任务绑定一个 Profile (隔离执行环境) 作为执行单元, 多 Agent 协作是"一组活跃 Profile + 共享 Kanban"自然涌现的结果. 这种"克制地引入新概念"的设计, 比"先设计 Team 类再设计 Task 类"更接近真实工作流, 也更适合 qianxun 沿 Phase 4a (Daemon 唯一 runtime) 的方向继续.**

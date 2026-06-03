# 千寻 (Qianxun) 多 Agent / Kanban / Team 协作架构设计

> 范围: 综合 `hermes-analysis.md` (A) + `qianxun-analysis.md` (B) 两份分析报告, 为千寻 (qianxun) 设计完整的多 Agent 协作架构, 覆盖数据模型 / Kanban / Team / 调度 / 通信 / 持久化 / UI / 配置 / 错误 / 可观测性 / 演进路线.
> 模式: `deep-engineering-handbook` (deep) — 每个核心设计都带 (a) 借鉴来源 (b) 千寻现状 (c) 理由 (d) 风险 + 不确定性标注 `[A]` 分析 / `[F]` 事实 / `[待确认]`.
> 时间: 2026-06-02 (首版) / 2026-06-03 (文件丢失重建版) / 2026-06-03 (v3 术语规范化) / 2026-06-03 (v4 加 §3.5 三种交互模式) / 2026-06-03 (v5 加 §3.6 项目/Session/并行模型) / 2026-06-03 (v6 整体梳理: Glossary 增 v4/v5 词, §6.1/§6.5/§8.5/§10.1/§11.1 交叉引用 §3.6). 状态: 提案, 待 PM 决策.

---

## 文档信息

> 本文档是 v6 设计报告的稳定归档副本, 源文件保留为工作副本.

- **源文件路径**: `E:/git/maxu/qianxun/.mavis/plans/qianxun-multi-agent-architecture.md`
- **同步日期**: 2026-06-03
- **版本号**: v6
- **维护说明**: 本文档跟源文件保持同步; 源文件是工作副本, 本文件是稳定归档. 如发现两处不一致, 以 `docs/30_子项目规划/04-kanban-design.md` 这边为准, 源文件顶部的 "## 迁移说明" 会指回这里.
- **关键变更摘要 (v3→v6)**: v3 规范化术语 (小A / 角色 / techlead 等), v4 加 §3.5 三种交互模式 (chat / 单任务 / 多任务), v5 加 §3.6 项目 (Project) / Session / 并行模型 + 新表 `kanban_projects`, v6 整体梳理: Glossary 增 v4/v5 词, §6.1/§6.5/§8.5/§10.1/§11.1 交叉引用 §3.6.

---

## 0. 术语表 (Glossary)

> v3 规范化 → v4 加模式术语 → v5 加项目/Session 术语. 所有配置项可改, 见 [§11.1](#111-角色-注册).

| 用户面向名 | 内/外 | 说明 |
|---|---|---|
| **小A** | 编排者 | 千寻顶层编排者. 接需求, dispatch 成任务, 看 Kanban 决定下一步. 配置标识: `xiaoA`. |
| **项目 (Project)** | 顶层组织 | 千寻的"一个完整工作目标". 1 项目 = 1 KanbanBoard (v1) + N Session. 默认绑 1 个文件夹 root, 可扩展. 配置标识: `proj_xxx`. |
| **Session** | 对话/工作块 | 1 次完整对话/工作流, 绑 1 个 LLM conversation. 跨天可重连. 配置标识: `sess_xxx`. |
| **Kanban** | UI 概念 | 看板. 持久化任务现场, 1 项目 = 1 board (v1). |
| **任务** (Task) | 数据 | DAG 节点, 状态 Triage/Ready/InProgress/Done/Blocked/Cancelled/Failed. |
| **子任务** (Subtask) | 数据 | DAG 边连接的任务, 依赖父任务. |
| **角色** (Role / Profile) | 概念 | 一组有名字有特长的虚拟员工. Hermes 借鉴概念 (profile). |
| **角色提示词** (Role Prompt) | 配置 | 角色的 system_prompt 片段. 写在 `~/.qianxun/roles/<role>.toml` 的 `prompt_suffix` 字段. |
| **techlead** | 默认角色 | 拆活 (decompose), 调度, 接受结果. |
| **coder** | 默认角色 | 写代码, 跑测试. |
| **verifier** | 默认角色 | 验收, 写 `gate=pass\|block`. |
| **researcher** | 默认角色 | 读代码, 写黑板, 综合产出. |
| **模式 1 (chat)** | 交互模式 | 纯聊. LLM 直接答, 不调任何工具. §3.5. |
| **模式 2 (单任务)** | 交互模式 | LLM 调 `kanban_create` 派 1 个角色干 1 件事. §3.5. |
| **模式 3 (多任务)** | 交互模式 | LLM 调 `kanban_decompose` 拆 N 个子任务派 N 个角色并行干. §3.5. |
| **dispatch** | 动作 | 把任务派给角色 (小A 或 techlead 角色 干这事). |
| **decompose** | 动作 | 拆活, 把 1 个任务拆成 N 个子任务 (techlead 角色 干这事). |
| **Worker scope** | 护栏 | 工具护栏模式: 跑活的角色只能动自己那个 task, 不能看全局 Kanban. |
| **Orchestrator scope** | 护栏 | 工具护栏模式: 调度的角色能看全局, 能新建/重派任务. |
| **ModeDecision 事件** | 观测 | `kanban_events` 里新加的事件类型, 记录每次 LLM 决定走哪个模式, 用于审计/调优. §3.5.4. |
| **`/dispatch`** | slash 命令 | 用户在 chat 框输 `/dispatch <需求>`, 强制走 Kanban 模式 (模式 2/3), 跳过 LLM 判模式. §3.5.5. |

**沿用英文 (技术术语, 不翻译)**:
- `Profile` — Rust struct (`qianxun-core/src/agent/team.rs`), 千寻对 Hermes profile 概念的内化. 用户面向词用 "角色", 代码用 "Profile".
- `KanbanScope` / `WorkerScope` — 工具 scope 上下文.
- `kanban_*` — 数据库表/字段前缀 (含新加的 `kanban_projects`).
- `Role` — Rust struct, 模板定义.
- `decompose` / `dispatch` — 动作动词, 保持英文.
- `gate=pass|block` — verifier 协议输出.
- `techlead` / `coder` / `verifier` / `researcher` — 4 默认角色的名字, 用户坚持保留英文.

**会话内简称 (本报告 §后用)**:
- 4 个默认角色合称 "**4 默认**", 单说时直接叫名字 (techlead/coder/verifier/researcher)
- "**小A**" 永远是编排者, 不与角色混用
- "**Hermes 角色**" 指 Hermes-agent 里的 profile 概念, 区别于千寻的"角色"
- "**VPS team**" 指 server/team_db.rs 的 team 概念, 跟"千寻角色"1:1 映射但语义不同
- "**项目 (Project)**" = 千寻顶层组织单位, 跟 KanbanBoard 1:1 概念上, v1 物理上独立存
- "**Session**" = 1 次对话/工作流, 跟项目是 1:N 关系

---

## 1. 执行摘要

千寻当前已是"单 Agent + 多前端 (TUI/ACP/Thin-Client/CLI/Server)"架构, Daemon Stage 1-7a 已经走完, 25+ 路由 + JWT + SSE 12 事件 + SvelteKit Web Admin Console 全部就绪, 但 `prompt_handler` 仍直接 `provider.stream_completion` 不接 `processing_loop`, AppState 持有的 tools/memory/skills 全是空/in_memory 占位, AgentPattern 仅有类型骨架, 与多 Agent 协作还差 8 个工程缺口. Hermes-agent (Python) 给我们 8 个高价值模式: 任务/运行解耦, 黑板表, Worker vs Orchestrator 工具护栏, LLM-Driven Decompose, Swarm root→workers→verifier→synthesizer, 派生深度+并发预算, 自动心跳桥, 多 角色隔离. 本方案把这些模式**收紧到千寻现有分层**: 在 `qianxun-core` 新建 `kanban` + `agent::pattern` + `agent::team` + `agent::blackboard` 四个独立模块, 在 `qianxun/src/daemon` 新建 `kanban_host` + `team_registry` + SSE 事件扩展 (5 个新 variant) + 8 个 HTTP 端点; **不引入新 crate, 不破坏 core/bin 分层, 不复制 Hermes 的多角色 独立进程** (改用 tokio mpsc + Arc<ProfileRegistry> 模拟). 团队 (Team) **不**做成第一类类型, 而是"一组 Role 模板 + 共享 Kanban"涌现出的协作视图, 这与 Hermes 的发现同构. 数据层复用 `qianxun-memory` 现有的 SQLite (8 表) + Daemon `SessionStore` (3 表) 的混合栈, 新增 7 张 `kanban_*` 表 + 1 张 `kanban_blackboard`. 演进分 MVP/v2/v3 三阶段, MVP 阶段 (8 周) 只新增 ~3500 行 + 改 ~600 行代码, 即可在 Phase 4a 路径上把"单 Agent 工具执行"延伸到"2-5 个 worker 协作".

---

## 2. 设计目标与非目标

### 2.1 目标 (in-scope)

| # | 目标 | 度量 |
|---|---|---|
| G1 | **让 Daemon 成为唯一 Agent runtime** (沿 Phase 4a) | `processing_loop_enabled: true`, prompt_handler 走 OutputSink adapter, TUI/ACP 改 thin-client |
| G2 | **原生支持 2-5 个 Agent 并行协作** (Kanban + 任务 DAG) | 用户发"调研 X" 触发 root + N 个子任务 DAG 自动分解 |
| G3 | **复用现有 trait 风格** | `AgentTool` / `LlmProvider` / `ContextProvider` / `MemoryObserver` / `OutputSink` 不变, 新增 `KanbanScope` / `TeamRole` / `Profile` |
| G4 | **不引入新 crate, 不破坏 core/bin 分层** | `qianxun-core` 加 4 个模块 (kanban/team/blackboard/pattern), `qianxun` 加 2 个 daemon 子模块 |
| G5 | **可观测** | 每个 task 有 audit event, 每个 run 有 token 用量, 失败/重试/取消都进入统一 event log |
| G6 | **私有部署 + 单 binary** | 不引 s6-overlay / Docker / K8s, 单 `qx daemon` 起, 多角色 用进程内 mpsc |
| G7 | **角色可插拔** | role 模板以 TOML 形式存在 `~/.qianxun/roles/*.toml`, 不写死在代码 |

### 2.2 非目标 (out-of-scope)

| # | 不做 | 理由 |
|---|---|---|
| N1 | 跨机器多角色 独立进程 | 私有部署单 VPS 单机, 跨进程需要 RPC + s6, 维护成本高. 用 in-process `Arc<ProfileRegistry>` + mpsc 模拟足够 [A] |
| N2 | 自动发现/注册外部 Agent 平台 (A2A 协议) | 千寻是闭源个人助手, 跨厂商互操作需求未出现 |
| N3 | LLM-driven 完全自主 Team 组装 (MetaGPT 风格) | 输出质量难控, 调试性差, 不符合"克制引入新概念"原则 |
| N4 | Kanban 多人协作 UI (类似 Trello 实时多用户) | 千寻是单用户, 同一时刻只有本人 + Daemon, 多人协作由 VPS 端承载 (留作 v3) |
| N5 | 自动从历史会话重建 Team (clustering) | 需要 embeddings, 涉及 v3 的 vector index 闭环, 不在 MVP 范围 |
| N6 | 跨语言 SDK (Python/Node 调千寻) | 跨语言会破坏"单 binary 部署"前提, 暂不开放 |

---

## 3. 现状综合 (从 A/B 提炼)

> 不复述两份原文, 只提炼对设计有约束力的硬事实.

### 3.1 千寻现有 (千寻-analysis.md §1-5, 8 个事实)

| 编号 | 事实 | 引用 | 对设计的影响 |
|---|---|---|---|
| F1 | 3 个 crate: `qianxun-core` (lib, ~6500 行) / `qianxun-memory` (lib, ~1800 行, **已闭环** 8 表) / `qianxun` (bin: `qx`, ~8400 行) | `Cargo.toml` [未读, 推断] | 任何新模块都进 `core` 或 `bin`, 不再增 crate |
| F2 | `AgentLoop` (engine.rs:38-72) + `processing_loop::handle_user_message` (engine.rs:83-462, 489 行) 是**完整可用**的 React 单循环, 内置 cancel / context compression (L1-L4) / tool execute / status 输出 | `qianxun-core/src/agent/engine.rs:38-72, 83-462` | 这是 Agent runtime kernel, 直接当作"worker", 复用到 Team |
| F3 | `WorkflowManager` (workflow.rs:43-223) 已有 4 个内置模板 (code-review / bug-fix / release / refactor), 每个模板含 3-4 个 `WorkflowStage` (含 `allowed_tools: ToolCategoryFilter` + `exit_marker`) | `qianxun-core/src/agent/workflow.rs:46-54, 70-216` | Workflow 已经成熟, 直接当 Swarm 模板复用 |
| F4 | `ToolCategoryFilter` (tools/mod.rs:25-62) + `ToolRegistry` (mod.rs:119-352) 提供 builtin / mcp / skill 三层工具调度, 已有 `execute_async_with_filter` 权限门控 | `qianxun-core/src/tools/mod.rs:25-62, 252-267` | 完美的 capability gating 基础, 不需重写 |
| F5 | `MemoryCore` (memory/src/lib.rs:35-320) 实现 `MemoryObserver` trait, 8 表 SQLite + FTS5 trigger, 18 集成测试 pass | `qianxun-memory/src/lib.rs:35-320`, `memory-state.md:42-51` | Kanban 黑板可以**复用** Memory FTS5, 或用独立表 (决策见 §7) |
| F6 | `SessionStore` (daemon/persistence.rs:260-293) 已 3 表 (`daemon_sessions` / `daemon_event_log` / `daemon_conversation_snapshots`), 都有 FK CASCADE | `qianxun/src/daemon/persistence.rs:260-293` | Kanban 表直接挂同一 SQLite 文件, FK 关联 session_id |
| F7 | `SseEvent` 12 事件 (daemon/sse.rs:24-86) + `processing_loop_enabled: false` (daemon/mod.rs:159), `prompt_handler` 直接 `provider.stream_completion`, 不接 processing_loop, 不执行 tool, 不接 memory/skills | `qianxun/src/daemon/sse.rs:24-86`, `qianxun/src/daemon/router.rs:1059-1159` | 这是 MVP 必须修复的 1 号缺口 (见 §14 路线) |
| F8 | `team_db` (server/team_db.rs:94-588) 已有 4 表 (`team_teams` / `team_members` / `team_projects` / `team_project_assignments`) + `devices`, 588 行, Stage 3 简化但 schema 完整 | `qianxun/src/server/team_db.rs:94-588` | **关键发现**: VPS Server 已经定义 Team/Project 概念, **本地 Daemon 的 Kanban 应与 VPS Team 保持 1:1 映射**, 不要发明两套并行语义 |

### 3.2 千寻缺口 (千寻-analysis.md §6 8 个缺口, 1 个架构决策)

| # | 缺口 | 修复路径 |
|---|---|---|
| 缺口 1 | `AgentLoop pattern dispatch` 未接 (engine.rs 写死 React, plan/reflect/workflow 孤儿) | 见 §6 数据模型 + §14 路线 MVP-2 |
| 缺口 2 | Daemon `prompt_handler` 不走 `processing_loop` | 见 §9 UI 集成, MVP-1 修复 |
| 缺口 3 | Daemon `memory/skills` 注入为空串 | MVP-1 修复, 同时建 memory bus |
| 缺口 4 | **多 Agent runtime 模型** (架构决策) | 本报告核心, 选 "in-process Profile + 共享 Kanban" |
| 缺口 5 | Agent-to-Agent 通信协议 | 见 §8, 选 "mpsc + 共享 Kanban" 组合 |
| 缺口 6 | Conversation 持久化是占位 JSON | MVP-1 修复 (沿用 `Conversation::save_to` JSONL) |
| 缺口 7 | `AppState.tools` / `skills` / `memory` 是空 / in_memory | MVP-0 修复 (启动时 register_builtin + load_all + open 真路径) |
| 缺口 8 | Tool 权限门控未接到 daemon | MVP-2 修复 (按 pattern 选 filter) |

### 3.3 Hermes 高价值发现 (hermes-analysis.md §7-8, 7 个可借鉴 + 1 个补充)

- **M1 任务/运行解耦** (⭐⭐⭐⭐⭐): `tasks` 表 + `task_runs` 表, 重试不丢历史 [A]
- **M2 DAG 黑板** (⭐⭐⭐⭐): Hermes 用 `[prefix]json` 评论实现, Rust 建议独立 `blackboard` 表 [A]
- **M3 Worker vs Orchestrator 工具护栏** (⭐⭐⭐⭐⭐): 环境变量 `HERMES_KANBAN_TASK` + toolset 双闸门 [A]
- **M4 LLM-Driven Decompose** (⭐⭐⭐⭐): 模糊任务让 LLM 返回 `{fanout, rationale, tasks[]}` JSON, 原子写图 [A]
- **M5 黑板+Verifier 门控 Swarm** (⭐⭐⭐⭐⭐): root → workers → verifier(metadata.gate=pass) → synthesizer, 这是"5 行代码级"的设计 [A]
- **M6 派生深度+并发预算** (⭐⭐⭐⭐⭐): `_get_max_spawn_depth` / `_get_max_concurrent_children` / `_get_child_timeout` 三个独立上限 [A]
- **M7 自动心跳桥** (⭐⭐⭐⭐): 60s 限频, 失败不抛, 改 tokio task [A]
- **M8 多角色 = 多独立进程 + 共享 Kanban** (⭐⭐⭐): 私有部署不抄, 改 in-process Channel [A]

### 3.4 Hermes 不推荐照搬 (hermes-analysis.md §8, 7 个)

- 多 Profile + 多 Gateway 实例 → 用单 daemon + 内部 profile registry
- 插件系统 (93KB dashboard plugin_api) → 用 WebSocket + SvelteKit
- Lark/Telegram/Discord/Feishu 多平台适配器 → 先只做 Feishu
- OAuth 凭据池 (99KB credential_pool.py) → 默认只 DeepSeek, 后期再扩
- ACPI 集成 (Zed) → 千寻 acp 模式自己实现
- s6-overlay 多进程管理 → 千寻 单 binary
- 大批量 preset skills → 用户自己写, 不预装

### 3.5 三种交互模式 (chat / 单任务 / 多任务)

> **核心原则**: 千寻的 LLM 编排**默认是纯聊天**, 80% 对话不动 Kanban. 只有用户需求是"动手做事"时才进任务流. 任务流分两层: 单任务 (1 角色干) 和多任务 (decompose 后多角色并行). LLM 通过**工具描述** + **角色 prompt** 双引导判模式, 不是硬规则.

#### 3.5.1 三种模式定义

| 模式 | 触发 | 走 Kanban? | LLM 动作 | 典型场景 |
|---|---|---|---|---|
| **1. 纯聊 (chat)** | 用户随便说一句, LLM 答 | ❌ | 直接答, 不调任何工具 | "今天天气怎么样" / "这段代码啥意思" / "你好" |
| **2. 单任务 (single-task)** | LLM 调 `kanban_create` 创建 1 个任务, dispatch 给 1 个角色 | ✅ | 1 次 tool call | "读 daemon/mod.rs 给我讲" / "把 foo.rs 改了" |
| **3. 多任务 (decompose)** | LLM 调 `kanban_decompose` 拆成 N 个子任务, dispatch 给 N 个角色 | ✅ | 1 次 tool call | "调研 Rust 2025 生态" / "把 daemon 升级 Phase 4a" |

#### 3.5.2 模式判定的归属: LLM 自己

**不是硬规则, 是 LLM 自己判**. 判的依据 (3 个杠杆):

**杠杆 1**: kanban 工具描述明确写"何时用 / 何时不用"

```rust
// qianxun-core/src/tools/builtin/kanban.rs
impl KanbanCreate {
    fn description(&self) -> &str {
        "创建一个持久化任务并 dispatch 给一个角色. \
         **何时调**: 用户请求需要 1 个角色干活, 你希望留可追溯记录. \
         **何时不要调**: 纯聊天 / 单步问答 / 已经在另一个 tool call 里做了的. \
         **任务复杂 (>=2 步, 多角色) 时, 用 kanban_decompose 而不是这个."
    }
}
impl KanbanDecompose {
    fn description(&self) -> &str {
        "把一个模糊需求拆成 3-5 个子任务, 每个派一个角色并行干. \
         **何时调**: 用户需求是探索性的 / 多面的 / 涉及研究+实施+验证. \
         **何时不要调**: 单一动作 — 用 kanban_create 即可. \
         **保守使用**: 一次对话 0-1 次足够."
    }
}
```

**杠杆 2**: techlead 角色的 prompt_suffix 写明拆解门槛

```toml
# ~/.qianxun/roles/techlead.toml
[role]
name = "techlead"
prompt_suffix = """
你是 techlead 角色, 干两件事:
1. **拆活 (decompose)**: 用户需求模糊时, 拆成可执行子任务
2. **调度 (dispatch)**: 把任务派给 coder / verifier / researcher

**拆解的判断标准** (4 个, 满足任一就拆):
- 涉及多个文件/模块/角色
- 需要并行 (研究 + 实施能同时跑)
- 用户需求模糊, 说不清楚
- 跨多步 (先调研再改再验)

**不拆, 直接派单任务** (用 kanban_create):
- 单一动作, 一两个 tool call 能搞定
- 用户问得清楚, 你不需要"补全"

**不调 Kanban** (直接回答):
- 纯问答 (直接答, 不动代码/文件)
- 需要先看用户输入再决定 (信息不够, 不该匆忙建任务)
"""
```

**杠杆 3**: 小A 顶层 prompt 也加引导

```toml
# ~/.qianxun/roles/xiaoA.toml (新, MVP-0 时建)
[role]
name = "xiaoA"
prompt_suffix = """
你是小A, 千寻顶层编排者.

**你的默认行为**: 跟用户正常聊天. **大多数对话不动 Kanban**.

**什么时候 dispatch 成 Kanban 任务**:
- 用户说"调研 X" / "实现 Y" / "修这个 bug" → 走 Kanban
- 单一动作 (读一个文件, 改一行) → 走 Kanban 单任务
- 复杂任务 (3+ 步, 涉及多面) → 走 Kanban decompose

**什么时候不调 Kanban** (默认, 80% 情况):
- 用户问问题 (你直接答)
- 用户聊想法 (你接着聊)
- 用户说"谢谢" / "早" (礼貌回复)
- 闲聊 (笑话, 天气)
"""
```

#### 3.5.3 降级路径 (LLM 判错时)

| LLM 判的模式 | 失败 | 降级到 |
|---|---|---|
| 模式 1 (纯聊) | 用户说"看不到结果" | 用户用 `/dispatch` slash command 强制走 Kanban (V1 加, §14) |
| 模式 2 (单任务) | role 跑失败 | 任务级 retry 3 次, 仍失败 → 任务 status=failed, 通知小A, 小A 决定加 fallback task |
| 模式 3 (decompose) | LLM 拆错 / 拆出环依赖 | 严格 schema 校验失败 → **自动降级到模式 2** (单任务, 1 个角色干), 写 warn 日志 (O2) |
| 任意 | 角色跑超时 | run outcome=TimedOut, 任务级 retry 2 次 |
| 任意 | Kanban 整个不可用 (DB 锁) | 退回模式 1, 错误事件写到 `kanban_events.kind=Error` |

#### 3.5.4 可观测性: ModeDecision 事件

每次对话的"模式判定"写到 `kanban_events.kind=ModeDecision` (新事件类型, §6.3 enum 加 1 种):

```rust
// 扩展 §6.3 KanbanEventKind
pub enum KanbanEventKind {
    // ... 现有 23 种 ...
    /// 小A 或 techlead 角色 决定走哪个模式 (mode=1|2|3)
    ModeDecision,
}

// payload 结构 (serde_json::Value):
{
    "session_id": "sess_xxx",
    "user_message_preview": "前 200 chars",
    "chosen_mode": 2,                        // 1, 2, 3
    "rationale": "用户要读一个文件, 1 个 tool call 够, 不需要拆",
    "roles_involved": ["researcher"]         // [] = 模式 1
}
```

用户在 TUI 看到: "小A 决定走模式 2 (单任务), 派给 researcher, 理由: 用户要读一个文件" — 完全可追溯.

#### 3.5.5 MVP 简化: 先上模式 1+2, 模式 3 留 v2

**理由**: 模式 3 (decompose) 强依赖 LLM 拆解质量, 需要 prompt 调优 + 真实案例训练. MVP 阶段 (8 周) 不一定有时间打磨好. 建议分两阶段:

- **v1 (MVP 8 周内)**: 模式 1 + 模式 2 上线, 模式 3 stub
  - LLM 看到"调研 X" → 走模式 2, 派给 researcher 单任务干 (这角色干完, 不会自动 decompose 给 coder)
  - 用户可用 `/decompose` slash command 强制走模式 3 (v1 不优化, 但能跑, MVP-2 启用)
- **v2 (8-16 周)**: 模式 3 调优
  - 跑 20+ 真实 case 调 decompose prompt
  - 成功率 > 80% 才算模式 3 "正式可用"
  - 同时把 §4 模式 4 (LLM-Driven Decompose) 的"严格 schema 校验"完整跑起来

**对应到 §14 路线**: 模式 1 走 MVP-0/MVP-1 (现有), 模式 2 走 MVP-2/MVP-3, 模式 3 stub 走 MVP-2, 模式 3 完整 v2.

### 3.6 项目 (Project) / Session / 并行模型

> **核心**: 千寻的"组织单位"分三层, 各管各的事, 互不混淆. 跟用户日常用 IDE / Notion / Linear 的直觉对齐.

#### 3.6.1 三层概念对照

| 概念 | 层 | 性质 | 持久性 | 例子 |
|---|---|---|---|---|
| **文件夹** | 物理层 | OS 目录, 放代码/数据 | 永久 (OS 管) | `E:\git\maxu\qianxun\` |
| **项目 (Project)** | 逻辑层 | 千寻顶层组织单位, 有 name/description/状态/默认 root | 长期 (跨 session 持续) | "千寻重构", "调研 hermes 架构" |
| **Kanban Board** | 任务现场 | 项目下的任务持久化层, 1 项目 = 1 board (v1) | 项目生命周期 | "千寻重构"项目下 47 个 task |
| **Session** | 对话/工作块 | 1 次完整对话/工作流, 绑定 1 个 LLM conversation | 可恢复 (历史保留) | 周一问"daemon 怎么升级" |

**关键原则**: **项目 ≠ 文件夹**. 项目是逻辑概念, 文件夹是物理概念. 1 个项目**默认**绑 1 个 root 文件夹, **但可扩展到 N 个** (调研类项目经常跨多个仓库).

#### 3.6.2 数据模型

```rust
// qianxun-core/src/kanban/types.rs (新增, MVP-2 时写)
pub struct Project {
    pub id: String,                          // "proj_xxx"
    pub name: String,                        // "千寻重构" (用户起)
    pub description: String,                 // 用户描述
    pub default_root: PathBuf,               // 默认 workspace, e.g. ~/code/qianxun
    pub extra_roots: Vec<PathBuf>,           // 跨仓库项目用, e.g. [qianxun-core/, qianxun-memory/]
    pub status: ProjectStatus,
    pub owner: String,                       // user_id (VPS 端) 或 "local" (单机)
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum ProjectStatus { Active, Archived }

// KanbanBoard 1:1 关联到 Project, 不独立存在
pub struct KanbanBoard {
    pub id: String,                          // = "kb_<project_id>", 等同
    pub project_id: String,                  // FK projects.id (新增, MVP-2)
    pub name: String,
    pub project_root: PathBuf,               // 从 Project.default_root 同步
    pub default_role: String,
    pub status: BoardStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Session 关联到 Project
pub struct Session {
    pub id: String,                          // "sess_xxx" (现有 daemon_sessions 表)
    pub project_id: String,                  // FK projects.id (新增)
    pub title: String,                       // "调研 daemon 升级", 自动从首条消息生成
    pub kanban_scope: Option<Arc<KanbanScope>>,  // 现有字段, 跟 Project 关联
    pub kanban_run_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}
```

**SQL Schema 增量** (在 §6.5 基础上加):

```sql
CREATE TABLE IF NOT EXISTS kanban_projects (
    id            TEXT PRIMARY KEY,         -- "proj_xxx"
    name          TEXT NOT NULL,
    description   TEXT NOT NULL,
    default_root  TEXT NOT NULL,
    extra_roots   TEXT NOT NULL DEFAULT '[]',  -- JSON array
    status        TEXT NOT NULL DEFAULT 'active',
    owner         TEXT NOT NULL DEFAULT 'local',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

-- kanban_boards 加 project_id 列 (迁移, 老 board 默认归到一个 "default" project)
ALTER TABLE kanban_boards ADD COLUMN project_id TEXT REFERENCES kanban_projects(id);

-- daemon_sessions 加 project_id 列
ALTER TABLE daemon_sessions ADD COLUMN project_id TEXT REFERENCES kanban_projects(id);
```

#### 3.6.3 Project / Session / KanbanBoard 关系

```
Project (顶层, 长期)
├── KanbanBoard (1:1, 任务现场, 项目生命周期)
│   ├── Task 1 ── Session A (这条 task 由 Session A 创建)
│   ├── Task 2 ── Session A
│   ├── Task 3 ── Session B
│   └── Task 4 ── Session C
├── Session A (历史)
├── Session B (历史)
└── Session C (当前活跃)
```

**1 : 1 : N : N 关系**:
- 1 个 Project = 1 个 KanbanBoard (v1; v2 可多)
- 1 个 Project = N 个 Session
- 1 个 Session = N 个 Task (跨 board 累积)
- 1 个 Task = 1 个 Session (创建它的 session)

**这是关键不变量**: **Task 知道它属于哪个 session, session 知道它属于哪个 project**. 用户能从 task 追溯到 session 再到 project, 完整审计链.

#### 3.6.4 Session 持久化 (历史可重连)

**现状** [F]: `daemon_sessions` 表 (3 表之一, daemon/persistence.rs:260-293) 已有 session_id / created_at / last_active_at. **没存** session 跟 project 的关系, **没存** session 跟 task 的关系.

**新增**:
- `daemon_sessions.project_id` (FK) — 哪个项目
- `kanban_tasks.session_id` (FK) — 哪个 session 创建的 (可选, 不强制)
- `daemon_sessions.title` — 自动从首条消息生成 (前 60 chars)

**重连 UX**:
- TUI 主屏加 `[Projects]` 视图, 列出所有 active project
- 选 project → 看到该项目下所有 session (按 last_active_at 倒序)
- 选 session → 进入该 session 的 chat, **加载历史 conversation**, 跟 session 中断时一样的状态
- 用户**不丢任何上下文**, 跨天跨周继续

#### 3.6.5 双轨并行模型 (chat 串行 + Kanban 并行)

| 你做的事 | 模式 | 行为 | UI 表现 |
|---|---|---|---|
| **发聊天消息** (默认) | 模式 1 (chat, §3.5) | 加到**当前 session** 的 conversation 末尾, **串行**: LLM 走完当前 step 才处理新消息 | 在 chat 框打字, 一句接一句, LLM 实时答 |
| **发任务** (LLM 判, 或 `/dispatch` 强制) | 模式 2/3 (Kanban, §3.5) | 走 Kanban, **跟当前 session 解耦**, 多任务可同时跑 | 切到 Tasks 视图, 看其他正在跑的 task |
| **一边等 chat 一边派 task** | 两轨并行 | chat 框照常打字, Kanban 同时跑 worker | TUI 左右分屏 (左 chat 右 Kanban), 或一上一下 |

**配置**: `TeamConfig.max_concurrent_children` (默认 5) 限制同时跑的 worker 数. 超了排队.

**端到端示例** (你一天的工作流):

```
09:00  你开 TUI, 选项目 "千寻重构", 选最近 session 继续
09:05  你发 "调研 daemon 升级到 Phase 4a" → 小A 走模式 3, techlead 拆 3 个 task
       → Kanban 视图立刻看到 3 个橙色"进行中"
       → 你**不阻塞**, 立刻切回 chat 框

09:08  你发 "这段代码啥意思" (指 mod.rs:159) → 小A 走模式 1, 走 chat
       → LLM 读文件, 答你 (串行, 跟 09:05 的 task 不冲突)
       → 你**马上看到答案**, 同时后台 worker 还在跑

09:15  你发 "/dispatch 写个 e2e 测试, 跑 cargo test" → 走模式 2, 派给 coder
       → Kanban 视图再加 1 个橙色"进行中", 现在共 4 个 worker 并行
       → 你继续发 chat 消息也 OK, 互不阻塞

10:00  你回 TUI, 看到:
       - 4 个 task 全绿, 含 token 用量 + 耗时
       - chat 历史 50 条, 3 段对话无缝连接
       - session 标题自动更新为 "千寻重构 09:00-10:00"
       - 你点 "新建 session" → 下一段干净开始, 共享项目 Kanban
```

#### 3.6.6 对现有设计的影响

- **§6.1 数据模型**: 加 `Project` / 增 `Session.project_id` / 增 `KanbanBoard.project_id`
- **§6.5 SQL Schema**: 加 `kanban_projects` 表 + 2 个 ALTER TABLE
- **§8.5 新增 3 端点**:
  - `POST /v1/projects` — 创建项目
  - `GET /v1/projects/{id}/sessions` — 列项目下所有 session
  - `PATCH /v1/projects/{id}` — 改项目元数据 (default_root, extra_roots 等)
- **§10.1 TUI**: 加 `[Projects]` 视图, session 选择器, 跨 session 重连
- **§11.1 配置**: 加项目注册表 (默认 1 个 "default" 项目, 老 board 归它)
- **§14 路线 MVP-2**: 实现 project + session 关联, ~150 行
- **§18 开放问题**: O10 新增 — "1 个 daemon 跑多项目是否 MVP?" (建议: MVP 单项目, v2 多项目切换)

**MVP 简化**: v1 (8 周内) daemon 跑**单项目**, 默认叫 "default", 现有 board 自动归它. v2 加多项目切换 UI.

#### 3.6.7 跟前两层 (Project / KanbanBoard) 的关系澄清

**对设计读者重要的一点**: 报告前面把"项目 = 看板"作为 v1 简化. 本节正式把两者拆开:
- **v1 (MVP)**: 1 Project = 1 KanbanBoard, 概念上 1:1, 物理上独立存. 用户不感知差异.
- **v2**: 1 Project 可以有 N 个 KanbanBoard (多业务线并行), 1 Board 可被跨项目关联 (e.g. 调研项目引用"重构"项目的 board)
- **v3 (VPS)**: VPS Team 1:N 同步到 N 个本地 Project, 每个 Project 独立 Kanban 现场

---

## 4. 借鉴的 Hermes 模式 (7 个, 含可移植性)

> 不重复 hermes-analysis.md 全文, 只写"在千寻如何落地"的部分.

### 模式 1: 任务/运行解耦 (M1, 借鉴强度 ⭐⭐⭐⭐⭐)

**Hermes 实现**: `hermes_cli/kanban_db.py:917-1069`, 6 表 SQLite, `tasks` + `task_runs` 双层. `task_runs` 每次重试一行新 row, 含 `claim_lock` / `worker_pid` / `last_heartbeat_at` / `outcome` / `error` (kanban_db.py:1016-1043) [A].

**千寻落地**:
- 数据模型: `kanban_tasks` + `kanban_runs` 双表, 见 §6.2
- 关键差异: 千寻没有 s6 进程, `worker_pid` 改为 `worker_kind: enum {Local, Remote}`; `claim_lock` 改为 `claim_id: uuid`, 取消时可直接重置
- **风险 [A]**: 任务级字段 (status) 与运行级字段 (status) 名字相同, SQL JOIN 时容易混. 建议所有 task 级字段加 `t_` 前缀 (如 `t_status`, `t_started_at`), run 级用 `r_` 前缀 (如 `r_status`, `r_heartbeat_at`)

### 模式 2: 黑板表 (M2, ⭐⭐⭐⭐)

**Hermes 实现**: 用 `[prefix] json` task_comments 实现黑板 (hermes_cli/kanban_swarm.py:26, 226-240) [A].

**千寻落地**:
- **决策**: 用**独立 `kanban_blackboard` 表**而非塞到 task_comments, 理由:
  1. Rust 强类型, JSON 字段单独走 `serde_json::Value` 比塞到通用 comments 里更易校验
  2. comments 表未来要支持 UI 评论 (用户给 LLM 留言), 跟黑板混用会污染 UX
  3. 单条 SELECT 拉黑板比 LIKE prefix 查询快
- 表 schema: `kanban_blackboard(task_id, key, value_json, author, updated_at)` 主键 `(task_id, key)`, 带 `idx_bb_task` 索引
- 辅助函数: `latest_blackboard(task_id) -> HashMap<String, Value>`, `post_blackboard(task_id, key, value, author)`
- **风险 [A]**: 黑板写入并发 (多个 worker 同时写) 需要乐观锁, 用 `updated_at` + 简单 "last writer wins" 即可, 不要 CAS (过度设计)

### 模式 3: Worker vs Orchestrator 工具护栏 (M3, ⭐⭐⭐⭐⭐)

**Hermes 实现**: `HERMES_KANBAN_TASK` 环境变量 + `_check_kanban_mode()` / `_check_kanban_orchestrator_mode()` (kanban_tools.py:49-90, 132-161) [A].

**千寻落地**:
- 在 `KanbanTool::execute()` 前置 `fn scope(&self) -> WorkerScope { Worker / Orchestrator }` 校验
- `Worker` 只能调 6 个工具: `kanban_complete` / `kanban_block` / `kanban_heartbeat` / `kanban_comment` / `kanban_read_blackboard` / `kanban_write_blackboard` (自己 task_id)
- `Orchestrator` 可调 9 个工具: 上面 6 个 + `kanban_list` / `kanban_unblock` / `kanban_create` / `kanban_link` / `kanban_assign`
- 决定 scope 的不变量: 每个 worker 启动时**只能看到** `task_id` 字段 (从 Kanban scope 上下文拿), 不暴露全局 task 列表 → 防 prompt injection 篡改兄弟任务
- **风险 [A]**: Worker 工具的 task_id 必须从**结构化上下文**取, 不能从 LLM 输出取 (LLM 可被 prompt 改字段), 千寻可以走 "在 system_prompt 注入 `[CURRENT_TASK_ID]` 占位符 + 工具实现里从 `Arc<KanbanScope>` 读"

### 模式 4: LLM-Driven Decompose (M4, ⭐⭐⭐⭐)

**Hermes 实现**: `kanban_decompose.py:52-109` 是精心设计的 system prompt, 让 LLM 返回 `{fanout, rationale, tasks[]}` [A].

**千寻落地**:
- 复用现有 `LlmProvider` trait, 调一次非流式 completion
- **关键改进**: Rust 这边用 `serde_json::from_str` 严格校验 (Hermes 的 `_extract_json_blob` 太宽松, kanban_decompose.py:349)
- schema 严格定义:
  ```rust
  #[derive(Deserialize)]
  struct DecomposeOutput {
      fanout: bool,
      rationale: String,
      tasks: Vec<DecomposeTask>,
  }
  #[derive(Deserialize)]
  struct DecomposeTask {
      title: String,
      body: String,
      assignee_role: String,           // 必须匹配已知 role
      depends_on_idx: Vec<usize>,       // 指向 tasks[] 的索引, 不是 id
      effort_estimate: String,          // "small" | "medium" | "large"
  }
  ```
- 失败处理: LLM 输出不合法 schema → 退回**单任务执行** (user input 直接当 single-task, 不强行 fan out), 记 warn 日志
- **风险 [A]**: 复杂度: 调 LLM 拆任务, 万一 LLM 拆错 (循环依赖 / assignee 不存在), 验证逻辑要完善, MVP 暂只做 "assignee_role 白名单 + depends_on_idx 范围检查"

### 模式 5: Swarm + Verifier 门控 (M5, ⭐⭐⭐⭐⭐)

**Hermes 实现**: Swarm 是 Kanban 之上的 DAG: root → workers → verifier(metadata.gate=pass) → synthesizer (kanban_swarm.py:178-179) [A].

**千寻落地**:
- **直接映射到现有 `WorkflowTemplate`**: 千寻已有 4 个 workflow 模板 (workflow.rs:46-54), 把"verifier"作为 stage 的一种角色加进去即可
- 新增 `TeamRole` enum: `Techlead | Coder | Verifier | Researcher` (跟 Stage 角色挂钩)
- Verifier 任务解锁条件: 所有 `depends_on` workers 已 `done` → Verifier 跑 → Verifier 输出必须包含 `[Review: OK]` 或 `[Review: Issues]` (复用 `reflect.rs::build_review_prompt` 的 prompt 协议) → 写 task metadata `gate=pass|block` → Synthesizer 解锁条件变为 `depends_on=verifier AND metadata.gate=pass`
- **风险 [A]**: Verifier 是个**软门控**, 不阻塞, 只是 gate metadata. 如果 Verifier 写 `gate=block`, Synthesizer 不解锁, 但 root 不挂起, 仍可手动加新任务

### 模式 6: 派生深度+并发预算 (M6, ⭐⭐⭐⭐⭐)

**Hermes 实现**: `_get_max_spawn_depth` / `_get_max_concurrent_children` / `_get_child_timeout` 三个独立上限 + `_get_orchestrator_enabled` 总开关 (delegate_tool.py 函数列表) [A].

**千寻落地**:
- 三个独立维度, 全部进 `TeamConfig`:
  ```rust
  pub struct TeamConfig {
      pub max_spawn_depth: u8,           // 默认 3 (含 orchestrator 本身)
      pub max_concurrent_children: u16,  // 默认 5
      pub child_timeout: Duration,       // 默认 5min
      pub orchestrator_enabled: bool,    // 总开关, 紧急刹车
  }
  ```
- 实现: 在 `KanbanHost::dispatch()` 入口检查
  1. `orchestrator_enabled == false` → 拒绝 spawn
  2. 当前 depth >= max_spawn_depth → 拒绝 spawn
  3. 活跃子任务数 >= max_concurrent_children → 排队 (不拒绝)
  4. spawn 时启动 `tokio::time::timeout` 包裹 child task
- 紧急刹车: 配置文件改 `orchestrator_enabled = false` → Daemon 热加载 (Phase 7c 已开 `/v1/config` PUT)
- **风险 [A]**: 深度限制只防"无限递归", 不防"宽 fan-out". 1000 个 worker 并发不在本机制内, 应由用户显式控制 (e.g. "调研 X" 任务 fanout 限制 ≤10)

### 模式 7: 自动心跳桥 (M7, ⭐⭐⭐⭐)

**Hermes 实现**: `heartbeat_current_worker_from_env()` 在 agent loop 内部被 tick, 60s 限频, 失败不抛 (kanban_tools.py:200-260) [A].

**千寻落地**:
- 在 `qianxun-core/src/agent/engine.rs::processing_loop` 里, 每次 `LlmStreamEvent::Text` / `ToolCall` 之后 spawn 一个轻量 task 写 heartbeat
- 限频: 用 `Arc<Mutex<Instant>>` 记录上次写时间, 差 < 60s 则 skip
- 写失败: warn 日志, 不抛 (best-effort, 跟 Hermes 一致)
- 字段: `kanban_runs.r_heartbeat_at` (TEXT, RFC3339)
- **风险 [A]**: 60s 阈值对快速任务 (10s 完成) 不生效, 这是预期的 — heartbeat 是"长任务未死"的信号, 短任务不写也行

---

## 5. 核心架构图 (ASCII 描述模块关系)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                       qianxun-core (lib)                                     │
│                                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │
│  │   agent/      │  │   kanban/    │  │   agent/     │  │  blackboard/ │   │
│  │   engine.rs   │←→│  (新)        │  │   team.rs    │  │  (新)        │   │
│  │  (已有 React) │  │              │  │  (新)        │  │              │   │
│  │              │  │  - Board     │  │              │  │  - Table     │   │
│  │  - AgentLoop │  │  - Task      │  │  - Profile   │  │  - Cell<K,V> │   │
│  │  - handle_   │  │  - Run       │  │  - Role      │  │  - Lock      │   │
│  │    user_msg  │  │  - KanbanScope│ │  - TeamConfig│  │              │   │
│  │              │  │              │  │  - dispatch()│  │              │   │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘   │
│         │                  │                 │                 │             │
│         │   复用            │                 │                 │             │
│  ┌──────▼──────────────────▼─────────────────▼─────────────────▼──────┐   │
│  │  现有 trait 层:                                                     │   │
│  │   AgentTool  (tools/mod.rs:99-110)                                  │   │
│  │   LlmProvider (provider/mod.rs)                                      │   │
│  │   ContextProvider / MemoryObserver (context/, memory/)              │   │
│  │   OutputSink (output.rs:5-19)                                       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────┬───────────┘
                                                                   │
                                                                   │ bin 引用
                                                                   ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                       qianxun (bin: qx)                                       │
│                                                                              │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                │
│  │ daemon/        │  │ daemon/        │  │ daemon/        │                │
│  │ agent_host.rs  │←→│ kanban_host.rs │←→│ team_registry. │                │
│  │ (现有)         │  │ (新)           │  │ rs (新)        │                │
│  │                │  │                │  │                │                │
│  │ - SessionRun   │  │ - KanbanDb     │  │ - Profile[]    │                │
│  │   time         │  │ - Dispatcher   │  │ - spawn()      │                │
│  │ - restore_     │  │ - SSE 5 新事件 │  │ - shutdown()   │                │
│  │   from_disk    │  │ - 8 新端点    │  │ - heartbeat    │                │
│  └────────────────┘  └────────┬───────┘  └────────┬───────┘                │
│                               │                   │                         │
│  ┌────────────────────────────▼───────────────────▼────────────────────┐  │
│  │  现有: router.rs (25+ 路由) + persistence.rs (3 表) + sse.rs (12 事件)│  │
│  └────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                │
│  │ client/        │  │ tui/            │  │ acp/           │                │
│  │ mod.rs (现有)  │  │ mod.rs (现有)   │  │ *.rs (现有)    │                │
│  │ thin-client    │  │ ratatui         │  │ JSON-RPC       │                │
│  │ 1211 行        │  │                 │  │                 │                │
│  └────────────────┘  └────────────────┘  └────────────────┘                │
│                                                                              │
│  ┌────────────────┐  ┌────────────────┐                                     │
│  │ server/        │  │ server/        │  ← VPS Server (Phase 4b)            │
│  │ team_db.rs     │  │ ws_hub.rs      │    Team/Project 已有 schema,         │
│  │ (现有 588 行)  │  │ 4515 行        │    Daemon Kanban 1:1 映射到此        │
│  └────────────────┘  └────────────────┘                                     │
└──────────────────────────────────────────────────────────────────────────────┘
                                           │
                                           │ HTTP+SSE
                                           ▼
                  ┌──────────────────────────────────────┐
                  │  TUI / ACP / Web Admin Console       │
                  │  (Svelte 5 SPA at /_ui/*)            │
                  │  + 后续 Kanban View                  │
                  └──────────────────────────────────────┘
```

**关键解读**:
- `qianxun-core` 加 4 个新模块 (kanban / team / blackboard / pattern dispatcher), **不增 crate**, 严格遵守"新增 crate 必须评估传递依赖树"约束
- `qianxun` daemon 加 2 个新子模块 (kanban_host / team_registry), **复用**现有 router.rs / persistence.rs / sse.rs
- VPS server 已有的 `team_db` 表是**上游**定义, Daemon Kanban 与之 1:1 映射 (避免两套并行语义)
- TUI / ACP / Web 全部走 `client/mod.rs` 已有的 thin-client 模式, 不修改入口

---

## 6. 数据模型定义

### 6.1 核心类型总览

| 类型 | 字段摘要 | 含义 | 持久化 | 模块位置 |
|---|---|---|---|---|
| `Project` | id, name, description, default_root, extra_roots, status, owner | 千寻顶层组织单位, 1 Project = 1 Board (v1) + N Session (见 §3.6) | `kanban_projects` 表 | `core::kanban` |
| `KanbanBoard` | id, name, project_id, project_root, default_role, status | 看板 (项目级, 一项目一板) | `kanban_boards` 表 | `core::kanban` |
| `Task` | id, board_id, parent_id, title, body, assignee_role, status, priority, deadline, metadata | 任务节点 (DAG 节点) | `kanban_tasks` 表 | `core::kanban` |
| `TaskLink` | parent_id, child_id, dep_type (DAG 边) | 任务依赖关系 | `kanban_task_links` 表 | `core::kanban` |
| `AgentRun` | id, task_id, profile_id, status, claim_id, heartbeat_at, started_at, ended_at, outcome, summary, error | 执行实例 (重试时新建 row) | `kanban_runs` 表 | `core::kanban` |
| `BlackboardCell` | task_id, key, value (JSON), author, updated_at | 黑板条目 | `kanban_blackboard` 表 | `core::blackboard` |
| `Profile` | id, name, kind, working_dir, tool_filter, max_turns, model | Agent 实例定义 (Hermes 角色 概念的内化) | `kanban_profiles` 表 | `core::agent::team` |
| `Role` | id, name, instructions, allowed_tools, default_profile_id, system_prompt_template | 角色模板 (与 Profile 1:N) | `~/.qianxun/roles/*.toml` + `kanban_role_defs` 表 | `core::agent::team` |
| `TeamConfig` | max_spawn_depth, max_concurrent_children, child_timeout, orchestrator_enabled | Team 调度预算 | `~/.qianxun/config.toml` `[team]` 段 | `core::agent::team` |
| `KanbanEvent` | id, task_id, run_id, kind (Create/Assign/Heartbeat/Complete/...), payload, created_at | 事件流 (审计 + 实时推送) | `kanban_events` 表 | `core::kanban` |
| `KanbanScope` | board_id, role, assigned_task_id, profiles_available | 工具 scope 上下文 (模式 3 护栏) | 内存 (Arc) | `core::kanban` |
| `Session` | id, project_id, title, kanban_scope, kanban_run_id, created_at, last_active_at | 用户 session (v5 加 project_id 关联, 见 §3.6.4) | `daemon_sessions` 表 (现有 + 1 列) | `daemon/persistence.rs` |

### 6.2 详细字段定义 (千寻风格)

```rust
// === qianxun-core/src/kanban/types.rs (新文件, ~180 行) ===

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// 看板 (一个项目 = 一个看板)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanBoard {
    pub id: String,                          // "kb_xxx" UUID v4
    pub name: String,                        // e.g. "千寻项目重构"
    pub project_root: PathBuf,               // workspace 根
    pub default_role: String,                // fallback role, 缺省 "techlead"
    pub status: BoardStatus,                 // Active | Archived
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum BoardStatus { Active, Archived }

/// 任务 (DAG 节点)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,                          // "task_xxx"
    pub board_id: String,                    // FK kanban_boards.id
    pub parent_id: Option<String>,           // FK kanban_tasks.id (root 任务无父)
    pub title: String,                       // <=120 chars
    pub body: String,                        // 任务描述, 可 Markdown
    pub assignee_role: String,              // FK kanban_role_defs.id
    pub status: TaskStatus,
    pub priority: u8,                        // 0=low, 255=urgent
    pub deadline: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,         // {gate: "pass"|"block", effort: "small", ...}
    pub created_at: DateTime<Utc>,
    pub t_started_at: Option<DateTime<Utc>>,
    pub t_completed_at: Option<DateTime<Utc>>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Triage,       // 刚创建, 未分配
    Ready,        // 父任务 done, 等待依赖解
    InProgress,   // 有活跃 run
    Done,         // 全部 children done + verifier gate=pass
    Blocked,      // 显式 block
    Cancelled,
    Failed,
}

/// DAG 边 (支持多类型)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLink {
    pub parent_id: String,
    pub child_id: String,
    pub dep_type: DependencyKind,           // default Sequential
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum DependencyKind {
    Sequential,    // 父 done 才能开始
    Soft,          // 父 done 是建议, 不强制
    Verifier,      // 父 done + metadata.gate=pass 才解锁
    Synthesizer,   // 父 done + Verifier's child gate=pass 才解锁
}

/// 执行实例 (重试时新建 row)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: String,                          // "run_xxx"
    pub task_id: String,                     // FK kanban_tasks.id
    pub profile_id: String,                  // FK kanban_profiles.id
    pub status: RunStatus,
    pub claim_id: Uuid,                      // 取消/重认领时新建 uuid
    pub r_heartbeat_at: Option<DateTime<Utc>>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: RunOutcome,
    pub summary: String,                     // LLM 总结
    pub error: Option<String>,
    pub token_input: u64,
    pub token_output: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum RunStatus { Pending, Running, Done, Crashed, TimedOut, Failed, Cancelled }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum RunOutcome { Success, PartialSuccess, Failure, Skipped, GateBlocked }

/// 黑板 (Hermes [prefix]json 模式的独立表化)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardCell {
    pub task_id: String,                     // FK kanban_tasks.id
    pub key: String,                         // "current_focus" | "user_constraints" | ...
    pub value: serde_json::Value,
    pub author: String,                      // profile_id | "user" | "system"
    pub updated_at: DateTime<Utc>,
}

/// 事件流 (审计 + 实时推送)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanEvent {
    pub id: i64,                             // auto-increment
    pub task_id: String,                     // FK kanban_tasks.id
    pub run_id: Option<String>,              // FK kanban_runs.id
    pub kind: KanbanEventKind,               // 23 种 variant, 见 §6.3
    pub payload: serde_json::Value,          // 事件具体内容
    pub created_at: DateTime<Utc>,
}

/// 工具 scope (护栏, 模式 3)
#[derive(Debug, Clone)]
pub struct KanbanScope {
    pub board_id: String,
    pub role: WorkerScope,
    pub assigned_task_id: Option<String>,    // Worker 只能动这个
    pub profiles_available: Vec<String>,     // Orchestrator 可见
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorkerScope { Worker, Orchestrator }
```

### 6.3 事件类型枚举 (23 种, 用于 audit + 实时推送)

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum KanbanEventKind {
    // Task lifecycle
    TaskCreated, TaskAssigned, TaskStarted, TaskPaused, TaskResumed,
    TaskCompleted, TaskBlocked, TaskUnblocked, TaskCancelled, TaskFailed,
    // Run lifecycle
    RunCreated, RunClaimed, RunHeartbeat, RunCompleted, RunCrashed, RunTimedOut,
    // Dependency
    DependencyUnblocked,
    // Blackboard
    BlackboardWrite, BlackboardRead,
    // Swarm
    GatePass, GateBlock, VerifierRun, SynthesizerRun,
    // System
    ConfigChanged, Error,
}
```

### 6.4 角色 / Profile / 配置 (与 VPS team_db 1:1 映射)

```rust
// === qianxun-core/src/agent/team.rs (新, ~250 行) ===

/// Profile (Hermes 角色 概念的内化) — 隔离执行单元
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,                          // "prof_xxx"
    pub name: String,                        // "techlead" | "researcher-1" | "verifier" | "coder"
    pub kind: ProfileKind,                   // Local (in-process) | Remote (VPS 转发, v3)
    pub working_dir: PathBuf,                // 隔离目录 (现千寻 daemon 共享 cwd, v2 改为子目录)
    pub tool_filter: ToolCategoryFilter,     // 默认 all, Role 可覆盖
    pub max_turns: u32,                      // 默认 32
    pub model: Option<String>,               // override 默认 model
    pub system_prompt_template: String,      // 占位符: {{role_instructions}} {{user_input}}
}

/// Role 模板 (用户可注册)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: String,                          // "role_xxx"
    pub name: String,                        // "researcher" | "verifier" | "coder" | "techlead"
    pub description: String,
    pub instructions: String,                // "你是研究员, 关注 X, 产出 Y 格式"
    pub default_profile_id: String,          // 默认绑定的 Profile
    pub allowed_tool_categories: ToolCategoryFilter,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ProfileKind { Local, Remote }      // Remote 用于 v3 跨机器

/// Team 调度预算 (模式 6)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub max_spawn_depth: u8,                 // 默认 3
    pub max_concurrent_children: u16,        // 默认 5
    pub child_timeout: Duration,             // 默认 5min
    pub orchestrator_enabled: bool,          // 默认 true
    pub auto_decompose: bool,                // 是否对模糊任务自动 LLM Decompose
    pub verifier_required: bool,             // Swarm 必跑 Verifier?
}
```

### 6.5 SQL Schema (千寻风格, 复用 daemon.db, 加 kanban_ 前缀)

> 沿用 `daemon/persistence.rs:260-293` 的 SCHEMA_SQL 风格, 加 PRAGMA 兼容.

```sql
-- 复用 daemon.db, 跟 daemon_sessions / daemon_event_log / daemon_conversation_snapshots 同一个文件
-- ~/.qianxun/daemon.db (现有路径)

PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;  -- 跟 team_db.rs:120 一致

CREATE TABLE IF NOT EXISTS kanban_projects (    -- §3.6.2 v5 新增
    id            TEXT PRIMARY KEY,         -- "proj_xxx"
    name          TEXT NOT NULL,
    description   TEXT NOT NULL,
    default_root  TEXT NOT NULL,
    extra_roots   TEXT NOT NULL DEFAULT '[]',  -- JSON array
    status        TEXT NOT NULL DEFAULT 'active',  -- active | archived
    owner         TEXT NOT NULL DEFAULT 'local',
    created_at    TEXT NOT NULL,            -- RFC3339
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_kanban_projects_status ON kanban_projects(status);

CREATE TABLE IF NOT EXISTS kanban_boards (
    id            TEXT PRIMARY KEY,         -- "kb_xxx"
    project_id    TEXT REFERENCES kanban_projects(id) ON DELETE CASCADE,  -- §3.6.2 v5 新增
    name          TEXT NOT NULL,
    project_root  TEXT NOT NULL,
    default_role  TEXT NOT NULL DEFAULT 'coordinator',
    status        TEXT NOT NULL DEFAULT 'active',  -- active | archived
    created_at    TEXT NOT NULL,            -- RFC3339
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_kanban_boards_status ON kanban_boards(status);
CREATE INDEX IF NOT EXISTS idx_kanban_boards_project ON kanban_boards(project_id);  -- §3.6.2 v5 新增

CREATE TABLE IF NOT EXISTS kanban_role_defs (
    id            TEXT PRIMARY KEY,         -- "role_xxx"
    name          TEXT NOT NULL UNIQUE,
    description   TEXT NOT NULL,
    instructions  TEXT NOT NULL,
    default_profile_id TEXT,                -- FK kanban_profiles.id
    allowed_tool_categories TEXT NOT NULL,  -- JSON: ["Read", "Search", ...]
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS kanban_profiles (
    id            TEXT PRIMARY KEY,         -- "prof_xxx"
    name          TEXT NOT NULL UNIQUE,
    kind          TEXT NOT NULL DEFAULT 'local',  -- local | remote
    working_dir   TEXT NOT NULL,
    tool_filter   TEXT NOT NULL,            -- JSON
    max_turns     INTEGER NOT NULL DEFAULT 32,
    model         TEXT,
    system_prompt_template TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS kanban_tasks (
    id            TEXT PRIMARY KEY,         -- "task_xxx"
    board_id      TEXT NOT NULL REFERENCES kanban_boards(id) ON DELETE CASCADE,
    parent_id     TEXT REFERENCES kanban_tasks(id) ON DELETE CASCADE,
    title         TEXT NOT NULL,
    body          TEXT NOT NULL,
    assignee_role TEXT NOT NULL,            -- FK kanban_role_defs.id (逻辑引用, 不强约束)
    status        TEXT NOT NULL DEFAULT 'triage',  -- 8 种 variant
    priority      INTEGER NOT NULL DEFAULT 128,
    deadline      TEXT,
    metadata      TEXT NOT NULL DEFAULT '{}',  -- JSON
    created_at    TEXT NOT NULL,
    t_started_at  TEXT,
    t_completed_at TEXT,
    last_heartbeat_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_kanban_tasks_board ON kanban_tasks(board_id);
CREATE INDEX IF NOT EXISTS idx_kanban_tasks_parent ON kanban_tasks(parent_id);
CREATE INDEX IF NOT EXISTS idx_kanban_tasks_status ON kanban_tasks(status);
CREATE INDEX IF NOT EXISTS idx_kanban_tasks_assignee ON kanban_tasks(assignee_role);

CREATE TABLE IF NOT EXISTS kanban_task_links (
    parent_id     TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
    child_id      TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
    dep_type      TEXT NOT NULL DEFAULT 'sequential',  -- 5 种
    created_at    TEXT NOT NULL,
    PRIMARY KEY (parent_id, child_id)
);
CREATE INDEX IF NOT EXISTS idx_kanban_task_links_child ON kanban_task_links(child_id);

CREATE TABLE IF NOT EXISTS kanban_runs (
    id            TEXT PRIMARY KEY,         -- "run_xxx"
    task_id       TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
    profile_id    TEXT NOT NULL REFERENCES kanban_profiles(id),
    status        TEXT NOT NULL DEFAULT 'pending',  -- 7 种
    claim_id      TEXT NOT NULL,            -- UUID v4, 重新认领时换
    r_heartbeat_at TEXT,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    outcome       TEXT NOT NULL DEFAULT 'success',  -- 5 种
    summary       TEXT NOT NULL DEFAULT '',
    error         TEXT,
    token_input   INTEGER NOT NULL DEFAULT 0,
    token_output  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_kanban_runs_task ON kanban_runs(task_id);
CREATE INDEX IF NOT EXISTS idx_kanban_runs_status ON kanban_runs(status);

CREATE TABLE IF NOT EXISTS kanban_blackboard (
    task_id       TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
    key           TEXT NOT NULL,
    value         TEXT NOT NULL,            -- JSON
    author        TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (task_id, key)
);

CREATE TABLE IF NOT EXISTS kanban_events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id       TEXT,                     -- NULL 表示 board 级事件
    run_id        TEXT,
    kind          TEXT NOT NULL,            -- 23 种
    payload       TEXT NOT NULL,            -- JSON
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_kanban_events_task ON kanban_events(task_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_kanban_events_kind ON kanban_events(kind);
```

**关键决策**: 与 `daemon_sessions` 共用一个 SQLite 文件 (`~/.qianxun/daemon.db`), 沿用现有 `Arc<Mutex<Connection>>` 单连接模式 (跟 `team_db.rs:97-99` 一致) [F]. 不引 r2d2 / sqlx, 因为千寻单 daemon 实例并发写本就不高.

---

## 7. Kanban 子系统

### 7.1 状态机

```
                    TaskCreated
                        │
                        ▼
                    Triage ─────────► Cancelled (user abort)
                        │ (assigned)
                        ▼
                    Ready  ◄──┐
                        │ (parent_done OR no_parent)
                        ▼
                    InProgress
                        │      ┌─► Blocked ─► Cancelled
                        │      │
                        ▼      ▼
                    Done    Failed
                        │
                        │ (compute "all children done" → recompute parent)
                        ▼
                    (parent task wakes up)
```

**转换规则** (Rust pseudo, 在 `kanban::state_machine.rs`):
- `Triage → Ready`: 必须 `assignee_role != null` AND 父任务 `Done` (root 任务无父, 自动 ready)
- `Ready → InProgress`: Dispatcher 拾取, 新建 `AgentRun`, 更新 `last_heartbeat_at`
- `InProgress → Done`: run 写 `outcome=Success` AND (有 children? 全部 children done : 无)
- `InProgress → Failed`: run 写 `outcome=Failure` AND retry_count < max
- `InProgress → Cancelled`: 显式 cancel API
- 任何 → `Blocked`: User 调 `kanban_block` API
- **`recompute_parent` 规则** (Hermes 关键不变量): child 状态变化时, 重新计算 parent 的可执行性, 若 parent 因 child done 而 unblock, 自动设 parent 为 `Ready`

**状态机代码** (在 `qianxun-core/src/kanban/state_machine.rs`, 新 ~150 行):

```rust
/// 检查 task 状态转换是否合法
pub fn check_transition(from: TaskStatus, to: TaskStatus) -> Result<(), KanbanError> {
    use TaskStatus::*;
    let allowed = match (from, to) {
        (Triage, Ready) | (Triage, Cancelled) | (Triage, Blocked) => true,
        (Ready, InProgress) | (Ready, Blocked) | (Ready, Cancelled) => true,
        (InProgress, Done) | (InProgress, Failed) | (InProgress, Blocked)
            | (InProgress, Cancelled) => true,
        (Blocked, Ready) | (Blocked, Cancelled) => true,
        (Failed, Ready) => true,  // 重试
        _ => false,
    };
    if allowed { Ok(()) } else {
        Err(KanbanError::InvalidStateTransition(
            format!("{from:?}"), stringify!(to),
        ))
    }
}

/// 重算父任务状态 (Hermes recompute_ready 的千寻版)
pub fn recompute_parent(conn: &Connection, task_id: &str) -> Result<(), KanbanError> {
    let mut stmt = conn.prepare("SELECT parent_id FROM kanban_tasks WHERE id = ?1")?;
    let parent_id: Option<String> = stmt.query_row(params![task_id], |r| r.get(0)).ok();
    let Some(parent_id) = parent_id else { return Ok(()) };

    // 1. 取父所有 children 的 status
    let mut stmt = conn.prepare(
        "SELECT status FROM kanban_tasks WHERE parent_id = ?1
         OR id IN (SELECT child_id FROM kanban_task_links WHERE parent_id = ?1)"
    )?;
    let children_status: Vec<String> = stmt
        .query_map(params![parent_id], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    // 2. 全 done → 父变 ready (等待 verifier 门控)
    let all_done = !children_status.is_empty()
        && children_status.iter().all(|s| s == "done");
    if all_done {
        conn.execute(
            "UPDATE kanban_tasks SET status = 'ready' WHERE id = ?1 AND status = 'in_progress'",
            params![parent_id],
        )?;
    }
    Ok(())
}
```

### 7.2 Kanban API (给 Agent + 给 Daemon HTTP)

**给 Agent 调用的工具** (在 `qianxun-core/src/tools/builtin/kanban.rs`, 新 ~250 行):

| 工具名 | 接受 scope | 行为 |
|---|---|---|
| `kanban_create` | Orchestrator | 创建 task, 返回 task_id, 立即 triage |
| `kanban_link` | Orchestrator | 创建 parent/child 边 |
| `kanban_assign` | Orchestrator | 设置 assignee_role, 触发 triage→ready |
| `kanban_list` | Orchestrator | 列出 board 内 task, 支持 status/assignee 过滤 |
| `kanban_unblock` | Orchestrator | 解除 Blocked 状态 |
| `kanban_complete` | Worker | 当前 task 标记 done, 写 summary |
| `kanban_block` | Worker | 当前 task 标记 blocked, 写 reason |
| `kanban_heartbeat` | Worker | 更新 last_heartbeat_at (60s 限频, 模式 7) |
| `kanban_comment` | Worker + Orchestrator | 写 task comment (审计用) |
| `kanban_read_blackboard` | Worker + Orchestrator | 读黑板 (按 task_id) |
| `kanban_write_blackboard` | Worker + Orchestrator | 写黑板 (last writer wins) |
| `kanban_read_blackboard_global` | Orchestrator | 读 board 级共享黑板 (root 任务) |

**关键护栏** (模式 3 落地):
```rust
// 伪代码, 实际在 KanbanTool::execute()
impl AgentTool for KanbanCreate {
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        // 1. scope check
        let scope = self.scope.read().await;
        if scope.role != WorkerScope::Orchestrator {
            return Err(ToolError::NotAllowedInCurrentMode { ... });
        }
        // 2. 校验 assignee_role 在白名单
        if !self.role_defs.iter().any(|r| r.id == args["assignee_role"]) {
            return Err(ToolError::InvalidArguments(format!(
                "assignee_role '{}' not registered", args["assignee_role"]
            )));
        }
        // 3. 写 SQLite
        let task = self.db.create_task(&args, scope.board_id).await?;
        // 4. 写 event
        self.db.append_event(task.id, None, KanbanEventKind::TaskCreated, json!({...})).await?;
        Ok(ToolOutput { content: json!({"task_id": task.id}), .. })
    }
}
```

### 7.3 持久化 (复用 daemon.db)

**决策** [A]: 用**现有** `~/.qianxun/daemon.db` 文件, 新增 7 张 `kanban_*` 表. 不另起 `kanban.db`, 理由:
1. 跨表查询便利 (e.g. "kanban task 的 runs 在过去 N 个 session 里的 token 用量")
2. 备份简单 (一个文件)
3. WAL 模式 + 单连接 + 千寻低写并发, 无性能问题
4. 跟 `team_db.rs:104` 的 `Arc<Mutex<Connection>>` 模式一致

**连接模型**:
- `KanbanDb { conn: Arc<Mutex<Connection>> }` (跟 `team_db.rs:97-99` 一致)
- 所有方法 `&self + lock().unwrap()`, 跟 team_db 风格一致
- 启动时在 `daemon/mod.rs::run()` 调 `kanban_host::init(&store_path)` 一次, 跟 `state.store = Arc::new(SessionStore::new(&store_path)?)` (mod.rs:124) 一样

### 7.4 跟 AgentLoop 挂钩

**核心问题**: `processing_loop::handle_user_message` (engine.rs:83) 当前是单 session 单 conversation, 怎么跟 Kanban 任务挂钩?

**答案**: 在 SessionRuntime 里加一个 `kanban_scope: Option<Arc<KanbanScope>>` 字段, 当 session 是从"领取 Kanban 任务"而来时设置 scope. 这样:
- 工具调用 (kanban_*) 通过 scope 自动校验
- `handle_user_message` 完成后, 调 `kanban_host::on_run_completed(run_id, outcome, summary)` 更新 task 状态

**新增字段** (在 `qianxun/src/daemon/session_runtime.rs:38-81` 加):
```rust
pub struct SessionRuntime {
    // ... 现有字段 ...
    /// 若此 session 是为某个 Kanban 任务而启, 持有 scope
    pub kanban_scope: Option<Arc<KanbanScope>>,
    /// 对应 run_id (用于回写)
    pub kanban_run_id: Option<String>,
}
```

**新增方法** (在 `qianxun-core/src/kanban/dispatcher.rs`):
```rust
impl KanbanDispatcher {
    /// 拾取 ready 任务, 找到 idle profile, 启动 AgentLoop
    pub async fn dispatch_once(&self) -> Result<Option<DispatchedRun>, KanbanError> {
        // 1. SELECT next ready task (LIMIT 1, FOR UPDATE in transaction)
        // 2. 找到空闲 profile (max_concurrent_children 检查)
        // 3. 新建 kanban_runs row (status=running, claim_id=uuid)
        // 4. 调 AgentLoopHost::spawn_session_for_task(task_id, run_id, profile)
        //    → 返回 SessionRuntime (含 kanban_scope)
        // 5. spawn 后台 task 跑 handle_user_message
        // 6. 完成后写 kanban_runs.ended_at + outcome, 调 recompute_parent
    }

    /// 后台循环: 每 N 秒调 dispatch_once (MVP 用 tokio::time::interval)
    pub async fn run_forever(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            if let Err(e) = self.dispatch_once().await {
                tracing::warn!("[kanban] dispatch error: {e}");
            }
        }
    }
}
```

### 7.5 不引入独立 Kanban binary

**明确决策** [A]: 不学 Hermes 把 Kanban 拆成 s6-managed 子进程. 千寻是单 binary 部署, Kanban 是 Daemon 内部子模块, 状态共享同一进程内存 + SQLite. 理由:
- 单用户单机, 没有"独立扩展"需求
- 避免进程间序列化
- 重启 Daemon = 重启 Kanban (在 `daemon/mod.rs::run()` 末尾启动 dispatcher tokio task 即可)

---

## 8. Team / Multi-agent 调度

### 8.1 调度策略: 选 supervisor 模式 (中央调度), 不选 DAG 纯声明式

**对比**:
- **DAG 纯声明式** (类似 Airflow): 用户提交完整 DAG, 调度器只看依赖图, 适合批处理, 但 LLM 输出不可预测, 难静态分析
- **Supervisor 模式** (类似 MetaGPT / Hermes): 一个 techlead 角色实时观察 + 决策, 其他 worker 只接受委派, 适合 LLM 主导
- **混合** (推荐): 用 Kanban DAG 表达"任务依赖" (静态), 用 supervisor techlead 角色表达"派谁/何时/重试" (动态)

**千寻选择**: **混合 (supervisor + DAG)**. techlead 角色是个**常驻 Profile**, 持续观察 `kanban_events` 表 (新 event 触发), 决定:
- 新 triage 任务 → 调 LLM Decompose (模式 4) → 拆成子 DAG
- 子 DAG 都 done → 唤醒 root → 决定"接受 / 加新任务"
- 子 DAG 中有 failed → 重试 (新建 run row) / 加 escalation task

**实现**: techlead 角色 不是特殊进程, 而是个 `Profile { name: "techlead", system_prompt: "你是 techlead 角色, 看到新任务做 X...", working_dir: <board root>, tool_filter: all }`. 它由 `KanbanDispatcher` 当 board 创建时自动 spawn.

### 8.2 委派协议 (mpsc + 共享 Kanban 组合)

**问题**: 子 agent 怎么"接受任务"? 父 agent 怎么知道子完成?

**千寻答案** (跟 Hermes 同构):
1. **不直接 RPC**. 父 agent 调 `kanban_create` 工具 (自己 board), 写 SQLite row, 返 task_id
2. `KanbanDispatcher` 看到新 row, 找空闲 profile, spawn `SessionRuntime { kanban_scope: Worker }`
3. Worker session 跑 `handle_user_message`, system_prompt 注入 "你是 worker, 当前 task_id = X, 完成后必须调 `kanban_complete`"
4. Worker 调 `kanban_complete` → 写 run outcome → `recompute_parent` 触发
5. 父 agent 怎么知道子完成? **不主动通知**, 父 agent 在新一轮 prompt (用户输入) 时 `kanban_list` 自己查; 或 daemon 主动把"子任务 done"作为一条 system event 注入父 conversation (模式 7 心跳的扩展)

**为什么不直接 RPC** [A]:
- 跟 Hermes 同理: 强制走异步 + 可观察路径, 避免直接 RPC 的隐式耦合
- 父 agent 不用等子 agent, 可继续接受其他用户输入
- 子 agent 崩溃不丢数据 (Kanban 状态保留)

### 8.3 通信机制对比

| 方式 | 千寻选? | 理由 |
|---|---|---|
| 进程内 mpsc channel | ✅ **辅助** | Worker → Dispatcher 回执 / 父 ↔ 子 cancel signal |
| HTTP | ✅ **跨 daemon** | 未来 v3 跨机器 (VPS 转发 worker) |
| WebSocket | ✅ **前端推送** | 已有 ws_hub (server/ws_hub.rs), 用于 VPS 控制面 |
| Event Bus (内存) | ✅ **进程内** | kanban_events 表是真相源, in-process 走 Arc<dyn EventBus> trait |
| 直接共享内存 | ❌ | 多并发不可控 |
| 第三方 broker (NATS/Redis) | ❌ | 私有部署, 增复杂度 |

**结论**: MVP 用 mpsc + 共享 SQLite, 跟千寻现有 Daemon 风格完全一致. v3 跨机器时再加 HTTP.

### 8.4 AgentMessage 协议 (扩展 SSE 12 事件 → 17 事件)

**当前 SSE 12 事件** (daemon/sse.rs:24-86) 全是单 Agent 视角. 多 Agent 需要加 5 种:

```rust
// qianxun/src/daemon/sse.rs (扩展)
pub enum SseEvent {
    // === 现有 12 种 (Stage 7a) ===
    MessageStart { ... }, ContentBlockStart { ... }, TextDelta { ... },
    ThinkingDelta { ... }, ToolUseDelta { ... }, ToolUseComplete { ... },
    ToolResult { ... }, ContentBlockStop { ... }, Usage { ... },
    MessageDelta { ... }, MessageStop, Error { ... },

    // === 新增 5 种 (MVP) ===
    /// Kanban 任务分配 (前端可以"切到 worker 视图")
    KanbanTaskAssigned {
        task_id: String,
        run_id: String,
        profile_name: String,
        title: String,
    },
    /// Kanban 任务进度 (worker 调 kanban_comment 或 kanban_write_blackboard 时发)
    KanbanTaskProgress {
        task_id: String,
        run_id: String,
        event_kind: String,                  // "comment" | "blackboard_write" | "tool_call"
        preview: String,                     // 前 200 chars
    },
    /// Kanban 任务完成 (worker 调 kanban_complete 时发)
    KanbanTaskCompleted {
        task_id: String,
        run_id: String,
        outcome: String,                     // "success" | "failure" | "gate_block"
        summary: String,
        token_input: u64,
        token_output: u64,
        elapsed_ms: u64,
    },
    /// 派生子任务 (techlead 角色调 kanban_create 时发, 前端展示 DAG 更新)
    KanbanTaskSpawned {
        parent_task_id: Option<String>,
        child_task_id: String,
        title: String,
        assignee_role: String,
    },
    /// 黑板变更 (techlead 角色实时观察)
    KanbanBlackboardUpdate {
        task_id: String,
        key: String,
        value_preview: String,               // 截前 200 chars
    },
}
```

**SseEventBuilder** 也要扩展: 在 `processing_loop::handle_user_message` 末尾, 调 `kanban_host::on_run_completed` 时同步 emit 上述事件.

### 8.5 新增 HTTP 端点 (8 个)

沿用 shared-contract §3.1 风格:

```
# === Projects (§3.6 v5 新增) ===
GET    /v1/projects                              → { projects: [...] }
POST   /v1/projects                              → { project_id, name, default_root, extra_roots }
GET    /v1/projects/{id}                         → { project, boards, sessions, stats }
PATCH  /v1/projects/{id}                         → { project_id, name, default_root, ... }
POST   /v1/projects/{id}/archive                 → { status: "archived" }
GET    /v1/projects/{id}/sessions                → { sessions: [...] }   # 列项目下所有 session
GET    /v1/sessions/{id}                         → { session, messages, tasks, mode_history }  # session 重连

# === Kanban ===
GET    /v1/kanban/boards                          → { boards: [...] }
POST   /v1/kanban/boards                          → { board_id, name, project_root, project_id }
GET    /v1/kanban/boards/{id}                     → { board, tasks: [...], runs: [...] }
POST   /v1/kanban/boards/{id}/archive             → { status: "archived" }

# === Tasks ===
GET    /v1/kanban/boards/{id}/tasks?status=&role=  → { tasks: [...] }
POST   /v1/kanban/boards/{id}/tasks               → { task_id, ... } (Orchestrator scope)
GET    /v1/kanban/tasks/{id}                      → { task, runs, events, blackboard }
POST   /v1/kanban/tasks/{id}/cancel               → { status: "cancelled" }
POST   /v1/kanban/tasks/{id}/retry                → { run_id, ... } (新建 run row)

# === Teams ===
GET    /v1/kanban/profiles                        → { profiles: [...] }
POST   /v1/kanban/profiles                        → { profile_id, ... }
PUT    /v1/kanban/profiles/{id}                   → { profile_id, ... }

GET    /v1/kanban/roles                           → { roles: [...] }
POST   /v1/kanban/roles                           → { role_id, ... }
PUT    /v1/kanban/roles/{id}                      → { role_id, ... }

# === Events (SSE 替代, 走 polling) ===
GET    /v1/kanban/boards/{id}/events?since=seq    → { events: [...], next_seq: N }
```

**关键点**: `/v1/kanban/boards/{id}/events?since=seq` 是给前端**非 WebSocket 场景**的 polling 端点, 避免每个 client 都要起 WS 链接. Web 用户走 ws_hub.

### 8.6 跟 VPS Server team_db 的关系

**关键决策** [A] (跟千寻-analysis §8 推断对齐):
- VPS Server 已有 `team_teams` / `team_members` / `team_projects` (server/team_db.rs:385-422) → 表达"**多用户共享** Team" (跨设备)
- Daemon Kanban 的 `kanban_boards` / `kanban_role_defs` / `kanban_profiles` → 表达"**单用户** Board" (本地)
- 映射关系 (v3 阶段): VPS 端 `team_id` ↔ Daemon 端 `board_id` (一对多, 一个 VPS team 可同步到多 device 的 board)
- **不发明并行语义**, MVP 阶段 Daemon 端独立, v3 加 sync

---

## 9. Task Lifecycle (端到端状态流转)

### 9.1 端到端示例: 用户发"调研 Rust 2025 生态"

```
[1] 用户在 TUI 输入
    ↓ POST /v1/chat/session/{id}/prompt
[2] daemon prompt_handler 收消息
    - 验 session, runtime.touch()
    - 构造 Conversation + push user message
    ↓
[3] system_prompt 注入 (4 段: base + memory + skills + skill_injections)
    ↓ LlmProvider.stream_completion()
[4] LLM 流式返回, 同时 OutputSink (SseEventBuilder) 把 12 事件推 SSE
    ↓ LLM 决定: 调 kanban_create (Orchestrator scope)
[5] ToolRegistry 调 kanban_create.execute()
    - 校验 scope == Orchestrator ✓
    - 校验 assignee_role == "researcher" 在白名单 ✓
    - 写 kanban_tasks row (status=triage)
    - 写 kanban_events row (kind=TaskCreated)
    ↓
[6] kanban_create 返 { task_id: "task_xxx" }
    ↓ LLM 拿到结果, 继续流式 (可能再调 kanban_link, 写黑板)
[7] LLM 完成, finish_reason=end_turn
    ↓ build_turn, conversation 保存到 memory
[8] 同时 KanbanDispatcher 后台 loop 看到新 task
    - SELECT next ready task (新 task 是 triage, 自动 ready)
    - 找空闲 profile "researcher-1"
    - 新建 kanban_runs row
    - spawn session: AgentLoopHost::spawn_for_task(task_id, run_id, profile)
    - kanban_tasks.status = in_progress
    ↓ emit SseEvent::KanbanTaskAssigned (前端切视图)
[9] Worker session 跑 handle_user_message
    - system_prompt 注入 "你是 researcher, task_id=task_xxx, 必须 kanban_complete 收尾"
    - LLM 跑 (调 read_file / web_search 等)
    - 调 kanban_write_blackboard 写过程产物
    ↓ emit SseEvent::KanbanTaskProgress (前端显示进度)
[10] LLM 完成, 调 kanban_complete
    - 写 kanban_runs.ended_at, outcome=success, summary="..."
    - 调 recompute_parent: 这是 root task, 自动 done
    ↓ emit SseEvent::KanbanTaskCompleted
[11] (可选) techlead 角色 看到 root done, 决定是否要 synthesis
    - 调 LLM Decompose: "给我合成" → 调 kanban_create 新任务
    - 重复 [8]-[10]
```

### 9.2 重试 / 失败路径

```
[1] Worker session 跑, LLM 反复出错 (rate limit / tool error)
[2] handle_user_message 内置 max_retries (来自 AgentConfig)
    - 超 → sink.on_error, return
[3] 调用方 (KanbanDispatcher::run_loop) 看到 error
    - 写 kanban_runs.ended_at, outcome=failure
    - 查 retry_count (新加字段, in metadata) < max_retry_per_task (默认 3)
    - 若是: 新建 run row, 重新 dispatch
    - 若否: kanban_tasks.status = failed, emit SseEvent::Error
[4] (可选) 父任务 / techlead 角色看到 child failed, 决定 retry / 加 fallback
```

### 9.3 取消路径

```
[1] 用户在 TUI 调 /cancel (POST /v1/chat/session/{id}/cancel)
[2] daemon → AgentLoopHost::cancel_session
    - 找到对应 runtime, set cancel_flag (AtomicBool)
    - 已在跑的 handle_user_message 下个 .await 看到 cancel_flag, return
[3] 若 session 是 worker (kanban_run_id != None)
    - 写 kanban_runs.ended_at, outcome=cancelled
    - kanban_tasks.status = cancelled (NOT failed, 区别)
[4] 父任务: kanban_list 看到 child cancelled, 决定补 task
```

### 9.4 持久化时机

| 时机 | 写什么 | 写哪里 |
|---|---|---|
| Task 创建 | `kanban_tasks` + `kanban_events` | 立即同步 |
| Run 开始 | `kanban_runs` (status=running) + `kanban_tasks.status=in_progress` | 立即 |
| 每次 tool call 结束 | `kanban_runs.r_heartbeat_at` (60s 限频) | best-effort, 失败 warn |
| Run 完成 | `kanban_runs.ended_at + outcome` + `kanban_tasks.status=done/failed/cancelled` | 立即 |
| Conversation 完成 | 复用 `daemon_conversation_snapshots` 表 (持久化全部 messages JSON) | 沿用现有 (缺口 6 修复) |
| Board 关闭 | `kanban_boards.status=archived` | 显式 |

---

## 10. UI 集成

### 10.1 TUI 端 (qianxun/src/tui/mod.rs, 1713 行, 已完成)

**现状 [F]**: ratatui + 脏标记渲染 + 增量行缓存 + 性能 ~447µs/帧. 已有 chat 视图 + tool 折叠 + @ 文件搜索.

**多 Agent 增强 (MVP-2)**:
- 新增 `Tab` 切换: `[Projects] [Chat] [Tasks] [角色]` 四个视图 (§3.6 v5: Projects tab 是入口, 选项目后才有 Chat/Tasks/角色 视图)
  - `[Projects]` 视图: 列出所有 active project, 选一个进入, 看到该项目下所有 session
  - `[Chat]` 视图: 当前 session 的 conversation, 模式 1 (默认) / 模式 2 (用户可手动 `/dispatch` 切)
  - `[Tasks]` 视图: 当前项目 board 的 task, 状态用色块, 选中回车进入详情
  - `[角色]` 视图: 列出当前活跃角色 / 调度预算
- `[Tasks]` 视图: 列表展示当前 board 的 task, 状态用色块, 选中回车进入详情 (run timeline + blackboard)
- `[角色]` 视图: 列出当前活跃角色 / 调度预算
- 关键交互:
  - 选中 task → `c` cancel / `r` retry / `b` block / `d` 查看详情
  - 实时刷新: 监听 `kanban_events` 表 (轮询 1s, MVP 简单方案; v2 改 push)

**文件位置**:
- 新 `qianxun/src/tui/kanban_view.rs` (~300 行)
- 新 `qianxun/src/tui/team_view.rs` (~150 行)
- `qianxun/src/tui/mod.rs` 加 tab 切换逻辑 (~50 行 diff)

**TUI Tasks 视图布局示意**:

```
┌─ 千寻 ──────────────────────────────────────────────────────────────┐
│  [Projects*] [Chat] [Tasks] [角色]   项目: 千寻重构   session: 当前   conns: 3  0:32:14  │
├──────────────────────────────────────────────────────────────────────┤
│ Triage (1)   Ready (2)   InProgress (1)   Done (12)   Blocked (0)    │
│                                                                       │
│ ▸ task_abc1  [in_progress]  调研 Rust 2025 生态                        │
│   └─ prof_researcher-1  run_run_001  12 turns  1.2K+0.3K tok        │
│   └─ heartbeat 12s ago, depth=1/3, concurrent=1/5                    │
│                                                                       │
│   task_def2  [ready]  写综合报告  → prof_synthesizer                  │
│   task_ghi3  [ready]  验证子任务  → prof_verifier                     │
│                                                                       │
│ [c]ancel [r]etry [b]lock [d]etail [/]filter  [Tab]switch  [q]uit     │
└──────────────────────────────────────────────────────────────────────┘
```

### 10.2 Web Admin Console (qianxun/src/daemon/ui/, Svelte 5, 已 build)

**现状 [F]**: SvelteKit dist 已存在, 4 路由 (`/llm` `/mcp` `/skills` `/tools` `/system` `/settings`), 但 8 个 `/v1/*` 端点返 stub JSON, UI 只能展示不能驱动 (千寻-analysis §4.4 缺口 7 修复后即可驱动).

**多 Agent 增强 (MVP-3 + v2)**:
- 新增 3 路由:
  - `/_ui/kanban` — Kanban 看板 (列: Triage | Ready | InProgress | Done | Blocked), 卡片拖拽
  - `/_ui/kanban/{task_id}` — Task 详情 (run timeline + blackboard + events)
  - `/_ui/roles` — 角色 列表 + 创建表单
- 实时性: 走 SSE 5 新事件 (`KanbanTaskAssigned` / `Progress` / `Completed` / `Spawned` / `BlackboardUpdate`), 跟现有 chat SSE 共用 EventSource
- 拖拽: KanbanBoard 列移动 = 调 `POST /v1/kanban/tasks/{id}/status` (v2 加, MVP 只读)

### 10.3 VPS Server 控制面 (qianxun/src/server/, Stage 6b)

**现状 [F]**: `ws_hub.rs` 1175 行 (4515 行 total), `outbox.rs` 317 行, `team_db.rs` 588 行, 已有 WebSocket hub 推送.

**多 Agent 增强 (v3)**: 跨设备同步
- Daemon 把本地 board 状态增量 sync 到 VPS (`outbox` 写入 → ws_hub 推送)
- VPS 端其他设备收到, 在 `/_ui/kanban` 展示
- **不做**实时多人编辑 (out of scope N4)

### 10.4 通知 (Slack / Feishu) — v3

**决策 [A]**: 不在 MVP 范围, 千寻用户当前是单设备单客户端, 通知场景未出现. v3 加 Outbox → Feishu webhook (复用 server/outbox.rs 的 outbox pattern).

---

## 11. 配置与角色系统

### 11.1 角色 注册 (跟 11.1a 项目注册 配合)

**11.1a 项目注册 (v5 新增, §3.6)**:
- 项目存 `~/.qianxun/projects.json` (轻量级, 类似 `package.json`)
- daemon 启动时 `project_registry::load_all()` 加载, 跟角色 TOML 风格一致
- 改完 hot-reload (5s 检测 mtime)
- 默认项目: daemon 首次启动自动建一个叫 "default" 的项目, root = `~/.qianxun`, 老 board 自动归它 (零迁移成本)
- 用户在 TUI 可: 新建 / 切 / 重命名 / 设默认 root / 加 extra_roots / archive

### 11.1 角色 注册

**两套注册表, 1:1 映射**:

```toml
# ~/.qianxun/roles/researcher.toml (用户可编辑)
[role]
id = "role_researcher"
name = "researcher"
description = "负责信息收集和初步分析"
instructions = """
你是研究员. 任务: 收集信息, 阅读指定文件, 产出结构化报告.
约束: 不要写文件, 只读 + 搜索 + 写黑板.
"""
allowed_tool_categories = ["Read", "Search", "Think"]
default_profile = "prof_researcher"
```

```toml
# ~/.qianxun/profiles/researcher.toml
[profile]
id = "prof_researcher"
name = "researcher-1"
kind = "local"
working_dir = "/path/to/board/researcher-1"  # MVP 用 board root 子目录
max_turns = 32
model = "deepseek-v4-flash"  # 覆盖默认
system_prompt_template = """
{{ role.instructions }}

## 当前任务
{{ task.title }}
{{ task.body }}

## 黑板
{{ blackboard }}
"""
```

**加载**: Daemon 启动时 `team_registry::load_all()`, 跟 `skills/mod.rs:59-78` 的 `SkillManager::load_all` 同构. 改动写回时 hot-reload (`/v1/kanban/roles` POST).

### 11.2 Team 调度预算 (配置)

```toml
# ~/.qianxun/config.toml
[team]
max_spawn_depth = 3
max_concurrent_children = 5
child_timeout_secs = 300
orchestrator_enabled = true
auto_decompose = true
verifier_required = true
```

**热加载**: `/v1/config` PUT (Stage 7b 已有, router.rs:836) 加 `[team]` 段处理.

### 11.3 鉴权 (沿用现有)

- Kanban API 跟现有 `/v1/chat/*` 一样走 JWT middleware (router.rs:177-284)
- 多用户场景 (VPS): 现有 `team_db.rs:393-398` 的 `team_members.role` 继续用, Kanban 跟 Team 1:1 映射
- **不发明新角色系统**, 沿用 `owner | admin | member`

### 11.4 角色模板内置 (MVP 起步)

千寻内置 4 个 role, 跟 `workflow.rs:50-54` 的 4 个 workflow 模板呼应:

| Role | 默认 profile | 工具 | 用途 |
|---|---|---|---|
| `techlead` | `prof_techlead` | all | 拆分任务, 调度 worker, 接受结果 |
| `coder` | `prof_worker` | all | 执行具体子任务 |
| `verifier` | `prof_verifier` | read + search + think | 验证 worker 输出, 写 gate |
| `researcher` | `prof_synthesizer` | read + search + think | 综合 worker 结果 |

**Verifier 跟 Synthesizer 跟 `reflect.rs::ReviewResult` 协议对接**: Verifier 输出必须含 `[Review: OK]` 或 `[Review: Issues]`, 复用现有 `build_review_prompt` 协议 (reflect.rs:44-71).

### 11.5 启动加载流程 (跟 `SkillManager::load_all` 风格一致)

```rust
// qianxun-core/src/agent/team.rs (新)
impl TeamRegistry {
    pub fn load_all() -> Self {
        let mut registry = Self::new();

        // 1. 全局: ~/.qianxun/roles/*.toml
        if let Some(dir) = Self::global_roles_dir() {
            registry.load_from_dir(&dir, RoleSource::Global);
        }
        if let Some(dir) = Self::global_profiles_dir() {
            registry.load_profiles_from_dir(&dir);
        }

        // 2. 内置 (4 role + 4 profile, 跟 workflow.rs 4 模板呼应)
        registry.register_builtin(builtin_techlead_role());
        registry.register_builtin(builtin_worker_role());
        registry.register_builtin(builtin_verifier_role());
        registry.register_builtin(builtin_synthesizer_role());

        registry
    }

    pub fn global_roles_dir() -> Option<PathBuf> {
        crate::workspace::qianxun_dir().map(|d| d.join("roles"))
    }

    pub fn global_profiles_dir() -> Option<PathBuf> {
        crate::workspace::qianxun_dir().map(|d| d.join("profiles"))
    }
}
```

**Hot reload**: `/v1/kanban/roles` POST 调 `TeamRegistry::reload()`, 跟 `qianxun-core/src/skills/mod.rs:200-260` 的 `SkillManager::reload` 风格一致.

---

## 12. 错误处理 / 重试 / 取消

### 12.1 错误分类 (千寻风格, thiserror)

```rust
// qianxun-core/src/kanban/error.rs (新)
#[derive(Debug, thiserror::Error)]
pub enum KanbanError {
    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("task '{0}' is not in a state that allows {1}")]
    InvalidStateTransition(String, &'static str),  // e.g. "task_xxx: complete (current: blocked)"

    #[error("assignee role '{0}' not registered")]
    UnknownRole(String),

    #[error("dependency cycle detected: task {0}")]
    DependencyCycle(String),

    #[error("run '{0}' already claimed by another worker")]
    AlreadyClaimed(String),

    #[error("depth limit reached (current: {current}, max: {max})")]
    DepthLimitReached { current: u8, max: u8 },

    #[error("concurrent children limit reached ({max})")]
    ConcurrencyLimitReached { max: u16 },

    #[error("orchestrator disabled in config")]
    OrchestratorDisabled,

    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
```

### 12.2 重试策略 (跟现有 LlmError 重试一致)

**任务级重试** (Kanban 层):
- 默认 `max_retry_per_task: 3` (写在 `kanban_runs.metadata`)
- 重试 = 新建 `kanban_runs` row (status=pending, claim_id=new uuid), 任务本身 status 不变 (仍是 in_progress)
- 重试触发: outcome=failure AND retry_count < max
- 重试 backoff: 指数退避, 1s / 4s / 16s

**Worker 层重试** (AgentLoop 内, 已有):
- `engine.rs:222-235` 已处理 `LlmError::RateLimitExceeded`, retry_count 来自 `AgentConfig.max_retries`
- 跟任务级重试不冲突: Worker 层是"LLM 调用失败"重试, 任务级是"任务 outcome=failure"重试

### 12.3 取消 (沿用现有 AtomicBool + 扩展)

**现有 [F]**:
- `AgentLoop.cancel_flag: Arc<AtomicBool>` (engine.rs:93)
- `daemon::SessionRuntime.paused: AtomicBool` (session_runtime.rs:80)
- `POST /v1/chat/session/{id}/cancel` 调 `agent_host.cancel_session` (router.rs:776-789)

**扩展**:
- Kanban 任务取消 = 调 `POST /v1/kanban/tasks/{id}/cancel`
- 实现: `kanban_host::cancel_task(task_id)` → 查 `kanban_runs WHERE task_id = X AND status = 'running'` → 调 `agent_host.cancel_session(run_id_to_session_id)` (新增映射) → 写 `kanban_runs.outcome=cancelled` → 写 `kanban_tasks.status=cancelled`
- 子任务 cancel 触发: 父任务 (techlead 角色) 在下个 dispatch tick 看到 child cancelled, 决定补 task

### 12.4 错误处理原则

| 错误类型 | 处理 | 例子 |
|---|---|---|
| 瞬时错误 (LLM rate limit, network) | 自动重试, backoff | `LlmError::RateLimitExceeded` 已实现 |
| 持久错误 (LLM 拒绝, task invalid) | 写到 task status=failed, 触发任务级重试或 escalate | LLM 一直返不合规 JSON |
| 用户错误 (请求不存在, scope 越界) | HTTP 4xx, 不重试 | 404 / 403 |
| 系统错误 (DB 锁死, OOM) | HTTP 5xx, 写 `kanban_events.kind=Error`, Daemon 日志 | `rusqlite::Error::DatabaseBusy` |
| 数据错误 (DAG 环) | 写 `kanban_events.kind=Error`, 标记 task blocked | 启动时 sanity check |

### 12.5 用户介入 (长任务中)

| 场景 | 用户动作 | 系统响应 |
|---|---|---|
| 看任务卡死 | TUI 切 Tasks 视图, 选 task, `d` 看心跳时间 | 超过 5min 无心跳 → 标 stale, 提示用户 cancel / retry |
| 想加新 task | TUI 调 `/kanban add "..." ` (新 slash command) | Orchestrator scope 调 kanban_create |
| 想改子任务 | TUI 选 task, `e` 编辑 body | 写新 metadata, 不删原 task (审计) |
| 想中止全部 | TUI 调 `/cancel-all` | 调所有 in_progress run 的 cancel, 写 board-level cancel event |

### 12.6 错误处理代码示例 (千寻风格, 实际写在 `kanban/dispatcher.rs`)

```rust
async fn run_with_protection(
    dispatcher: Arc<KanbanDispatcher>,
    run: AgentRun,
) -> Result<RunOutcome, KanbanError> {
    // 1. 启动 timeout (模式 6 child_timeout)
    let timeout = dispatcher.config.child_timeout;
    let result = tokio::time::timeout(
        timeout,
        async {
            // 2. 启动 AgentLoop (调 processing_loop::handle_user_message)
            let mut agent = AgentLoop::new(dispatcher.agent_config.clone());
            let mut conv = Conversation::new(None);
            // 3. 构造 sink 把事件同时推 SSE 客户端 + 写 kanban_events
            let sink = KanbanEventSink::new(dispatcher.clone(), run.id.clone());
            // 4. 跑主循环, 内置 rate limit 重试 (engine.rs:222-235)
            processing_loop::handle_user_message(
                &mut agent,
                &mut conv,
                &*dispatcher.provider,
                &dispatcher.tools,
                ToolCategoryFilter::all(),
                &sink,
                "",
                "",
                "",
                dispatcher.cancel_flag.clone(),
            ).await;
            // 5. 提取 run outcome
            match agent.state {
                AgentState::Idle => Ok(RunOutcome::Success),
                AgentState::Stopping => Ok(RunOutcome::PartialSuccess),
                AgentState::Error(msg) => Ok(RunOutcome::Failure),
                _ => Ok(RunOutcome::Failure),
            }
        }
    ).await;

    match result {
        Ok(outcome) => outcome,
        Err(_elapsed) => {
            // 6. 超时: cancel 内部 + 写 outcome=TimedOut
            dispatcher.cancel_flag.store(true, Ordering::SeqCst);
            RunOutcome::Failure  // 写库时转成 TimedOut
        }
    }
}
```

---

## 13. 可观测性

### 13.1 三个观测面

| 面 | 指标 | 存储 | 暴露 |
|---|---|---|---|
| Token 用量 | 每 run / 每 task / 每 board / 每 session 累计 | `kanban_runs.token_input/output` | `/v1/system/metrics` (Stage 7b 已有) + Kanban 详情 API |
| 任务统计 | ready/in_progress/done/failed 数, 平均完成时间 | `kanban_events` 聚合 | `/v1/kanban/boards/{id}/stats` (新) |
| 失败日志 | 每次失败 / 重试 / 取消的 reason | `kanban_runs.error` + `kanban_events` | TUI Tasks 视图 + Kanban detail |

### 13.2 Metrics endpoint 扩展

```json
// GET /v1/system/metrics (扩展, Stage 7b 基础 + Kanban 段)
{
  "uptime_s": 3600,
  "cpu": 0.05,
  "mem_mb": 120,
  "conns": 3,
  "kanban": {
    "boards_active": 2,
    "tasks_total": 47,
    "tasks_in_progress": 3,
    "tasks_done_24h": 12,
    "tasks_failed_24h": 1,
    "avg_task_duration_s": 18.5,
    "runs_total": 89,
    "runs_crashed_24h": 0,
    "concurrent_children_peak": 4
  },
  "token_usage_24h": {
    "input": 1_200_000,
    "output": 380_000
  }
}
```

### 13.3 TUI 可视化 (Phase B 已开性能基准, 复用)

- Tasks 视图顶部一行: 活跃 board / in-progress / 完成率
- 选 task 后, 底部 status bar 显示 token 用量 + 已用时长

### 13.4 审计 (kanban_events 表是真相源)

- 23 种事件, 全部走 `append_event`, 永不删 (除非 board archive)
- 给 VPS 控制面用, 暂不直接给用户

### 13.5 Structured Logging (跟千寻现有 tracing 栈一致)

`kanban_host` 内部所有关键路径加 `tracing::info!` / `tracing::warn!`, 字段标准化:

```rust
tracing::info!(
    target: "qianxun.kanban",
    task_id = %task.id,
    run_id = %run.id,
    profile = %profile.name,
    board_id = %board.id,
    "task started"
);

tracing::warn!(
    target: "qianxun.kanban",
    task_id = %task.id,
    retry_count = metadata.retry_count,
    max_retry = metadata.max_retry,
    "task retry"
);
```

`/v1/system/logs?lines=N` 端点 (Stage 7b 已有, router.rs:1083) 自动收集到, TUI / Web 直接展示. 跟现有 `qianxun-core` 的 `tracing::info!` 风格一致 (engine.rs:97 已有).

### 13.6 OpenTelemetry (v3 评估)

v3 阶段评估是否引入 OpenTelemetry (`opentelemetry` crate, 评估传递依赖, 阈值 30). 千寻当前 tracing 已够用, v3 跨机器 worker 派发时再加 (那时确实需要分布式 trace).

---

## 14. 演进路线 (MVP / v2 / v3 三阶段, 对齐 Phase 4a/4b)

### 14.1 MVP (目标: 8 周, 跟 Phase 4a 同步)

| 周 | 任务 | 验收 | 改动范围 |
|---|---|---|---|
| **MVP-0** (W1) | 修复缺口 7: `AppState.tools/skills/memory` 从空改成真实初始化 | `cargo run` 启动 daemon, `/v1/tools` 返真实 builtin 列表 | `daemon/mod.rs:100-117` + `agent_host.rs` 启动调 `register_builtin` / `load_all` / `open(~/.qianxun/mem.db)` |
| **MVP-1** (W2) | 修复缺口 2 + 3 + 6: `prompt_handler` 改走 `processing_loop`, 注入 memory + skills, 持久化 conversation | 真实调模型, 工具能执行, session 重启能恢复 | `daemon/router.rs:1059-1270` 重写 + `daemon/persistence.rs:save_snapshot` 用真 Conversation JSONL |
| **MVP-2** (W3-4) | 新建 `qianxun-core/src/kanban/` 完整模块, 加 7 张表 (含 v5 加的 `kanban_projects`), 实现 12 个 kanban_* 工具 (**v1 上线: 模式 1 + 模式 2, 见 §3.5.5**, 模式 3 stub); 同时实现 §3.6 的 Project + Session 关联 (单项目即可, N 项目 v2) | `cargo test` 新模块 20+ 单测 pass, 集成测试能在真 Daemon 里创建 task 并被 worker 完成, `/dispatch` slash command 强制走 Kanban 模式, TUI 切到 `[Projects]` 视图看到默认项目 | 新增 ~1700 行 (`kanban/types.rs` + `db.rs` + `state_machine.rs` + `dispatcher.rs` + `tools/builtin/kanban.rs` + `error.rs` + `project.rs` + `session_link.rs` ~200 行) |
| **MVP-3** (W5) | 新建 `qianxun/src/daemon/kanban_host.rs` + `team_registry.rs`, 改 `daemon/mod.rs` 启动 dispatcher loop, 新增 8 HTTP 端点 + ModeDecision 事件流 (§3.5.4) | `POST /v1/kanban/boards` → 创建 board, 自动 spawn techlead 角色, `/dispatch` 工作流; 用户发"调研 X" 自动走模式 2 (单任务) | 新增 ~800 行 daemon + 改 router.rs 加 8 路由 |
| **MVP-4** (W6) | SSE 5 新事件 + `output.rs` `on_event` 扩展, 让前端能实时看到 task 进度 | TUI 接 SSE 能切到 Tasks 视图, Web Admin Console 新增 `/_ui/kanban` | `daemon/sse.rs` 扩 5 variant + `tui/kanban_view.rs` ~300 行 + `daemon/ui/src/routes/kanban/` ~200 行 |
| **MVP-5** (W7) | 模式 1-7 全部端到端测试 + `cargo clippy --all -- -D warnings` 0 警告 | `cargo test --workspace` pass, 端到端 1 个 task 完成能进 Kanban detail | `kanban/tests/e2e_*.rs` ~400 行 |
| **MVP-6** (W8) | docs 更新: 写 `docs/30_子项目规划/04-kanban-design.md` (类似 `01b-daemon-web-console.md`), `CLAUDE.md` 加 kanban 模块结构, `_shared-contract.md` 加 8 端点 + 5 SSE 事件 | 文档与代码一致 | 3 doc 文件改动 |

**MVP 总改动**: 新增 ~3500 行 + 改 ~600 行 = ~4100 行, 严格不引新 crate.

### 14.2 v2 (8-16 周, 跟 Phase 4a 完成后)

- **Tauri Desktop** (Track C): Kanban 视图进 Tauri 桌面 (`docs/30_子项目规划/03-tauri-desktop.md` 已规划)
- **Decompose 优化**: LLM 拆任务, 用真实业务跑 20+ 案例调 prompt
- **Verifier / Synthesizer 模板化**: 跟 `WorkflowTemplate` 深度集成
- **Web Kanban 拖拽**: `/_ui/kanban` 改 SvelteKit 拖拽
- **多 board**: 一个 daemon 可有多个 board (多项目并行)
- **跨 session conversation 共享**: 子 worker 的对话可被父 context 引用 (黑板的扩展用法)

### 14.3 v3 (16+ 周, 跟 Phase 4b 同步)

- **VPS Server ↔ Daemon 同步**: `team_db.rs` ↔ `kanban_boards` 双向 sync
- **跨设备 worker 派发**: Profile.kind = Remote, worker 在另一台 device 跑
- **Notifications**: Feishu / Slack 推送 task 状态变化 (用现有 server/outbox.rs)
- **历史会话聚类 → Team 模板**: 用 memory 现有 Vector 索引 (memory/src/vector.rs) 跑 clustering
- **多用户协作 UI**: Web Kanban 实时多用户 (引入 CRDT 或 last-writer-wins)

### 14.4 显式不引入 (从 Hermes 抛弃)

- ❌ 独立 Kanban binary + s6-overlay 管理
- ❌ 多角色 独立 Gateway 进程 + 独立端口
- ❌ 插件系统 (93KB dashboard plugin_api)
- ❌ 多平台适配器 (Telegram / Discord / Slack / Feishu 全套)
- ❌ OAuth 凭据池 (99KB credential_pool.py)
- ❌ ACPI 集成 (hermes-acp)
- ❌ 大批量 preset skills / MCP 适配 skill

理由: 私有部署单 binary 原则, 跟 N1-N6 一致.

---

## 15. 不引入的部分 (从 Hermes 主动抛弃)

> 跟 §14.4 重叠, 此处是更系统的说明.

| Hermes 模块 | 千寻是否引入 | 理由 |
|---|---|---|
| **多 Profile + 多 Gateway 实例** (s6-overlay) | ❌ | 私有部署单 VPS 单机, 跨进程需要 RPC + s6, 维护成本高. 千寻用单 daemon + 内部 `Arc<ProfileRegistry>` 即可 (G6) |
| **插件系统** (93KB dashboard plugin_api) | ❌ | 千寻用 WebSocket + SvelteKit 替代. 已建 4 路由 `/llm` `/mcp` `/skills` `/tools` (千寻-analysis §4.4), 加 3 路由 `/_ui/kanban` `/_ui/kanban/{task_id}` `/_ui/team` 即可 |
| **Lark / Telegram / Discord / Feishu 多平台适配器** (250KB+ auxiliary_client.py, 90+ 适配器) | ❌ (MVP), v3 再评估 | 千寻默认走 SvelteKit Web UI + TUI, 用户已在用 Feishu 但当前无通知场景. v3 才加 outbox 模式通知 |
| **OAuth 凭据池** (99KB credential_pool.py) | ❌ (MVP) | 千寻默认只 DeepSeek 一个 provider, 后期扩 multi-provider 走现有 `llm_providers.rs` (Stage 7a 已实现 CRUD, router.rs:111-122) |
| **ACPI 集成 (Zed)** hermes-acp | ❌ | 千寻的 `qianxun/src/acp/*.rs` 已有 1900 行 8 文件, 自己实现. ACP thin-client 模式 (`main.rs:288` 未实现) 留 Phase 4a 之后 |
| **s6-overlay 多进程管理** | ❌ | Docker 内才需要, 千寻单 binary 部署 |
| **大批量 preset skills** (`skills/`, `optional-skills/`) | ❌ | 大量 MCP 适配 skill (calendar, spotify, video gen, browser). 千寻应该让用户自己写, 不预装 |
| **A2A 跨厂商互操作协议** | ❌ | 千寻是闭源个人助手, 跨厂商需求未出现 (N2) |
| **MetaGPT 风格完全自主 Team 组装** | ❌ | 输出质量难控, 调试性差 (N3) |
| **Trello 风格实时多人协作** | ❌ | 单用户场景, 多人走 VPS 端 (N4) |
| **跨语言 SDK (Python / Node 调千寻)** | ❌ | 破坏"单 binary 部署"前提 (N6) |

理由统一: 私有部署 + 单 binary + 克制引入新概念, 跟 N1-N6 一致.

---

## 16. 覆盖矩阵 (A/B 各自关键发现 → 报告章节映射)

### 16.1 Hermes 关键发现 → 报告章节

| Hermes 关键发现 (hermes-analysis.md) | 借鉴/不借鉴 | 报告章节 |
|---|---|---|
| §3.1 任务/运行解耦 | ✅ 借鉴 M1 | §6.2 Task + AgentRun 双层, §7.1 状态机, §9.4 持久化时机 |
| §3.2 状态机 `running\|done\|blocked\|crashed\|timed_out\|failed\|released` | ✅ 借鉴 + 加 TaskStatus 8 种 | §6.2 `TaskStatus` 8 种 + `RunStatus` 7 种 |
| §3.3 Worker vs Orchestrator 工具分流 | ✅ 借鉴 M3 | §6.5 `KanbanScope` + §7.2 API 护栏 + §11.3 鉴权 |
| §3.3 自动心跳桥 (60s 限频) | ✅ 借鉴 M7 | §4 模式 7, §9.4 持久化时机 |
| §3.4 Kanban as multi-board | ✅ 借鉴 | §6.1 KanbanBoard + §8.5 多 board 端点 |
| §4.1 "Team" 不是类型, 是 角色集合 | ✅ 借鉴 | §6.1 TeamConfig + §6.4 Profile/Role, §11.1 TOML 注册 |
| §4.2 角色隔离边界 | ✅ 部分借鉴 (in-process) | §6.4 Profile, §8.3 不用独立进程 |
| §4.3 LLM-Driven Decompose | ✅ 借鉴 M4 | §4 模式 4, §6.1 DecomposeOutput schema |
| §4.4 Swarm root→workers→verifier→synthesizer | ✅ 借鉴 M5 | §4 模式 5, §6.2 DependencyKind, §11.4 4 内置 role |
| §5.1 delegate_task 6 能力 | ✅ 部分借鉴 (3 能力) | §6.4 TeamConfig (max_spawn_depth / max_concurrent_children / child_timeout), §8.1 supervisor |
| §5.2 Subagent 共享 env + credentials | ✅ 借鉴 | §8.2 mpsc + 共享 SQLite |
| §5.3 三种协作模式组合 (Delegate + Swarm + Multi-Profile) | ✅ 借鉴 | §8.1 混合 (supervisor + DAG) |
| §6.1 TUI 跟踪子 agent 派生 | ✅ 借鉴 | §10.1 TUI Tasks 视图 |
| §6.2 Dashboard 插件 | ❌ 不抄 (用 WebSocket + Svelte) | §10.2 Web Admin Console |
| §6.3 Gateway 通知 | ❌ 不抄 (v3 加 outbox) | §10.4 通知 (v3) |
| §7 模式 1-7 (全部 8 个) | ✅ 全部借鉴 | §4 模式 1-7 |
| §8 不推荐 1-7 | ❌ 全部不抄 | §14.4 显式不引入, §15 |

### 16.2 千寻关键发现 → 报告章节

| 千寻关键发现 (qianxun-analysis.md) | 报告章节 |
|---|---|
| §1 workspace 3 crate | §5 架构图, §6.1 模块位置 |
| §1 实际阶段远超 docs | §3.1 F1-F8 重新核实 |
| §2 plan/reflect/workflow 孤儿 | §3.1 F3, §6.1 WorkflowManager 复用, §11.4 4 内置 role |
| §3 daemon `prompt_handler` 不接 processing_loop | §3.2 缺口 2, §14.1 MVP-1 修复 |
| §3 4 段 system_prompt 拼接 | §6.1 build_request 4 段, §9.1 step [3] |
| §3 build_request 三个空串 | §3.2 缺口 3, §14.1 MVP-1 修复 |
| §4.1 25+ 路由 | §3.1 F7, §8.5 新增 8 路由 (合计 33+) |
| §4.4 8 个瓶颈 (1 个最严重: 不执行工具) | §3.2 缺口 1-8, §14.1 MVP-0 到 MVP-3 全部覆盖 |
| §5 入口矩阵 (qx cli/tui/acp/daemon/server/client) | §10 UI 集成全覆盖, §5 架构图 |
| §6 8 个 multi-agent 缺口 | §3.2 + §14.1 路线一一对应 |
| §7 硬约束 (Rust 2024, MSRV 1.85, reqwest rustls, < 30 传递依赖) | §2.1 G4, §14.1 MVP 总改动明确不引新 crate |
| §8 借鉴适配性 (天然适合 11 项) | §3.1 F1-F8, §6.1 模块位置 |

### 16.3 决策追溯表

| 决策 | 借鉴来源 | 千寻现状 | 理由 | 风险 |
|---|---|---|---|---|
| 7 张 `kanban_*` 表挂 daemon.db | Hermes §3.1 6 表 | daemon.db 现有 3 表 + team_db 5 表 (VPS) | 单文件备份 + 跨表查询 | DB 单连接, 写并发限速 [A] |
| `WorkerScope` 二元角色护栏 | Hermes §3.3 | 现有 `ToolCategoryFilter` (6 类) | 跟现有 category 体系正交, 不重写 | Worker 工具的 task_id 注入位置要小心 (LLM 可改字段) |
| Decompose 用严格 JSON schema | Hermes §4.3 (`_extract_json_blob` 太松) | 现有 `serde_json` 全栈 | Rust 强类型优势 | LLM 输出不合规时回退单任务 |
| Swarm verifier 用 `[Review: OK]` 协议 | Hermes §4.4 + 现有 `reflect.rs:44-71` | 现有 `ReviewResult` + `build_review_prompt` | 复用已有 prompt 工程 | 软门控, 不阻塞, 仅 metadata 标记 |
| Profile 用 in-process mpsc, 不用独立进程 | Hermes §4.2 (用 s6) | 千寻单 binary 部署 | 私有部署 + N1 决策 | 跨机器要 v3 才支持 (ProfileKind=Remote) |
| techlead 角色 = 普通角色, 不特殊 | Hermes §4.1 ("Team 不是类型") | 现有 4 workflow 模板已隐含角色概念 | 一致性, 不发明新概念 | 调度器需识别 "这是个 techlead 角色" 任务 |
| SSE 5 新事件扩展 12 事件 | Hermes §3.4 (`task_events` 表) | 现有 sse.rs 12 事件 | 跟 shared-contract 兼容 | client 端 SseEvent 同步要改 (thin-client 1211 行) |
| 跟 team_db 1:1 映射 (不同步) | 千寻 §8 (VPS 已有 team_db) | team_db 588 行已有 | 不发明并行语义 | v3 sync 时机要重新设计 |
| 模式 1-7 全部借鉴 | Hermes §7 模式 1-8 | — | 7 个都是高价值 | 模式 8 (多角色 独立进程) 主动不抄 |

---

## 17. 证据索引 (A/B 引用 + 千寻 docs 引用)

### 17.1 Hermes-analysis 引用

| 报告章节 | 引用 |
|---|---|
| §3.1 数据模型 | hermes-analysis.md:43-52 (6 表) |
| §3.3 工具护栏 | hermes-analysis.md:69-77 (kanban_tools.py:49-90, 132-161) |
| §4.1 Team = 角色集合 | hermes-analysis.md:87-96 (profiles.py 60KB) |
| §4.3 Decompose | hermes-analysis.md:112-119 (kanban_decompose.py:9-14, 52-109) |
| §4.4 Swarm | hermes-analysis.md:121-138 (kanban_swarm.py 全文 279 行) |
| §5.1 delegate_task | hermes-analysis.md:144-160 (delegate_tool.py 函数列表) |
| §7 模式 1-8 | hermes-analysis.md:201-258 (8 模式含可移植性 ⭐) |
| §8 不推荐 1-7 | hermes-analysis.md:261-269 (7 项主动抛弃) |
| §9.1 端到端单 Profile 路径 | hermes-analysis.md:275-295 |
| §9.2 Swarm 路径 | hermes-analysis.md:297-321 |
| §10 证据索引 | hermes-analysis.md:334-347 |

### 17.2 千寻-analysis 引用

| 报告章节 | 引用 |
|---|---|
| §3.1 F1 (3 crate) | qianxun-analysis.md:11-20 |
| §3.1 F2 (AgentLoop) | qianxun-analysis.md:64 (engine.rs:38-72, 83-462) |
| §3.1 F3 (workflow.rs) | qianxun-analysis.md:71 (workflow.rs:50-54, 70-216) |
| §3.1 F4 (ToolCategoryFilter) | qianxun-analysis.md:72 (mod.rs:25-62, 252-267) |
| §3.1 F5 (MemoryCore 闭环) | qianxun-analysis.md:66 (memory-state.md:42-51) |
| §3.1 F6 (SessionStore 3 表) | qianxun-analysis.md:66 (daemon/persistence.rs:260-293) |
| §3.1 F7 (SSE 12 事件) | qianxun-analysis.md:91-101 (sse.rs:24-86, router.rs:1059-1159) |
| §3.1 F8 (team_db 5 表) | qianxun-analysis.md:49 (server/team_db.rs:94-588) |
| §3.2 8 个缺口 | qianxun-analysis.md:177-189 |
| §8 借鉴适配性 | qianxun-analysis.md:222-244 |
| §10 总结判断 | qianxun-analysis.md:282-291 |

### 17.3 千寻 docs 引用

| 报告章节 | 引用 |
|---|---|
| §1 执行摘要 (Phase 4a) | `docs/architecture.md:80-100` (AgentLoop 归 Daemon) |
| §3.1 F2 (AgentLoop React) | `qianxun-core/src/agent/engine.rs:38-72, 83-462` |
| §3.1 F3 (Workflow 4 模板) | `qianxun-core/src/agent/workflow.rs:46-54, 70-216` |
| §3.1 F4 (ToolCategory) | `qianxun-core/src/tools/mod.rs:14-62, 252-267` |
| §3.1 F5 (MemoryCore) | `qianxun-memory/src/lib.rs:35-320` |
| §3.1 F6 (SessionStore 3 表) | `qianxun/src/daemon/persistence.rs:260-293` |
| §3.1 F7 (SSE 12 事件) | `qianxun/src/daemon/sse.rs:24-86`, `qianxun/src/daemon/router.rs:1059-1159` |
| §3.1 F7 (processing_loop_enabled: false) | `qianxun/src/daemon/mod.rs:159` |
| §3.1 F8 (team_db schema) | `qianxun/src/server/team_db.rs:385-422` |
| §5 架构图 | 综合 `qianxun-core/src/agent/{mod,engine,workflow,plan,reflect}.rs` + `qianxun/src/daemon/{mod,agent_host,session_runtime,persistence,router,sse}.rs` + `qianxun/src/server/team_db.rs` |
| §8.4 SSE 5 新事件 | 扩展 `qianxun/src/daemon/sse.rs:24-86` 现有 12 事件 |
| §8.5 新增 8 端点 | 沿用 `qianxun/src/daemon/router.rs:81-135` 25+ 路由风格, 沿用 `docs/30_子项目规划/_shared-contract.md:47-66` REST 契约 |
| §10 UI 集成 | 现有 `qianxun/src/tui/mod.rs:1-50` (Tab 切换加 `[Tasks] [Team]` 视图) + `qianxun/src/daemon/ui/` (SvelteKit 已 build) |
| §11 配置 | 沿用 `qianxun-core/src/skills/mod.rs:59-78` `SkillManager::load_all` 模式 |
| §14.1 路线 | 对齐 `docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md:126-200` Phase E (Daemon 真实运行时) + Phase F (Agent Patterns) |
| §14.3 v3 | 对齐 `docs/30_子项目规划/02-vps-server.md` + `01-daemon.md` |

### 17.4 重读时引用的源码 (本次会话)

| 文件 | 行数 | 关键内容 |
|---|---|---|
| `qianxun-core/src/agent/engine.rs` | 489 | AgentLoop + processing_loop::handle_user_message (本报告 §3.1 F2) |
| `qianxun-core/src/agent/conversation.rs` | 166 | build_request 4 段拼接, JSONL 持久化 (本报告 §9.4 持久化时机) |
| `qianxun-core/src/agent/workflow.rs` | 223 | 4 内置模板, WorkflowStage 含 allowed_tools (本报告 §3.1 F3) |
| `qianxun-core/src/agent/plan.rs` | 56 | PlanState, plan_phase_filter (本报告 §11.4 verifier 协议) |
| `qianxun-core/src/agent/reflect.rs` | 72 | ReviewResult, build_review_prompt (本报告 §11.4 verifier 协议) |
| `qianxun-core/src/agent/mod.rs` | 12 | 公开 API (本报告 §6.1 模块位置) |
| `qianxun-core/src/tools/mod.rs` | 352 | ToolCategoryFilter, ToolRegistry, execute_async_with_filter (本报告 §3.1 F4) |
| `qianxun-core/src/output.rs` | 19 | OutputSink trait (本报告 §3.1 F2, §8.4 SSE 扩展) |
| `qianxun-core/src/skills/mod.rs` | 397 | SkillManager::load_all (本报告 §11.1 配置) |
| `qianxun-memory/src/lib.rs` | 644 | MemoryCore + MemoryObserver (本报告 §3.1 F5) |
| `qianxun/src/daemon/mod.rs` | 198 | AppState 14 字段 (本报告 §3.1 F7) |
| `qianxun/src/daemon/session_runtime.rs` | 141 | SessionRuntime 字段 (本报告 §7.4 扩展) |
| `qianxun/src/daemon/router.rs` | 3237 | 25+ 路由 + prompt_handler (本报告 §3.1 F7, §14.1 MVP-1) |
| `qianxun/src/server/team_db.rs` | 588 | 4 表 + devices (本报告 §3.1 F8) |
| `qianxun/src/client/mod.rs` | 1211 | thin-client 12 事件 (本报告 §3.1 thin-client 已完成) |

---

## 18. 开放问题 (待用户决策 / 待验证)

### 18.1 决策类 (待 PM/用户拍板)

| # | 问题 | 候选答案 | 默认建议 | 验证方法 |
|---|---|---|---|---|
| O1 | techlead 角色 是否自动 spawn? | (a) 总是自动 (b) 用户显式启动 (c) 配置开关 | (c) `[team] auto_spawn_techlead = true` | 跑 1 个 board 验证 |
| O2 | LLM Decompose 失败时怎么办? | (a) 退回单任务 (b) 报 4xx (c) 人工 escalate | (a) 自动降级到模式 2 (单任务, 1 个角色干), 写 warn (见 §3.5.3) | 喂坏数据跑 5 次 |
| O3 | Verifier gate=block 时, 父任务自动加 escalation task? | (a) 是 (b) 否, 仅通知 | (b) 否, 避免 LLM 自我强化 | 跑 3 个失败案例看 |
| O4 | 多 board 是否 MVP? | (a) MVP (b) v2 | (b) v2, MVP 单 board | — |
| O5 | Kanban event retention? | (a) 永久 (b) 90 天 (c) board archive 时清 | (a) 永久, 反正 SQLite 增长慢 (1KB/event) | 跑 1 周看 DB 大小 |
| O6 | 失败任务是否自动 retry? | (a) 是, 3 次 (b) 否, 等用户 | (a) 是, 用 kanban_runs.metadata 存 retry_count | 跑 1 个故意 fail 的 case |
| O7 | Decompose 用哪个 LLM? | (a) 默认 provider (b) 单独小模型 (c) 用户配 | (a) 默认, 跟主链路同 | 跑 20 个真实任务比质量 |
| O8 | "Worker 是 prompt injection 防不住的"怎么办? | (a) 完全信任 (b) Worker 工具白名单 (c) 每个 tool call 经 LLM 二次审核 | (b) Worker 工具仅 6 个白名单 | fuzz test 50 个 prompt injection |
| O9 | Daemon 启动时如发现已有 in_progress run, 怎么办? | (a) 自动 retry (b) 标 stale 等用户 (c) 标 failed 关闭 | (b) 标 stale, 写 kanban_events, 用户决定 | 模拟 crash 一次 |
| O10 | 1 个 daemon 是否 MVP 跑多项目? (见 §3.6.6 末注) | (a) MVP 单项目, v2 多项目 (b) MVP 多项目 | (a) MVP 单项目, daemon 启动自动建 "default" 项目, 1 项目 = 1 board, v2 加多项目切换 UI (推荐) | 跑 1 周看用户是否会自然想开第 2 个项目 |

### 18.2 验证类 (待实际跑)

| # | 待验证 | 验证方法 | 期望 |
|---|---|---|---|
| V1 | `processing_loop::handle_user_message` 的 489 行是否能直接套到 worker 场景 | MVP-1 改 prompt_handler 后跑 1 个真实 case | 工具能执行, 心跳能写, cancel 能生效 |
| V2 | workflow.rs 4 模板能否直接当 swarm 模板用 | MVP-5 端到端跑 code-review 模板 | 3 stage 顺序跑, verifier 门控 |
| V3 | team_db 5 表 + kanban 7 表 11 表共一个 SQLite 性能 | 跑 100 task 1000 run 后看 query latency | < 10ms (P95) |
| V4 | TUI Tasks 视图的脏标记渲染是否能复用 Phase B 的 447µs/帧 | 加 tab 后跑 1MB stream | < 600µs/帧 (1.3x 现有) |
| V5 | SSE 12+5=17 事件对 thin-client 1211 行影响 | 客户端解码测试 | 不破 SseEvent enum tag 兼容 |
| V6 | Pattern dispatcher (plan/reflect/workflow) 接 processing_loop 难度 | MVP-2 改 engine.rs 试 1 个 pattern | 跟 React 共用 cancel / heartbeat / tool execute 基础设施 |

### 18.3 调研类 (待深读)

| # | 待调研 | 来源 |
|---|---|---|
| R1 | `qianxun-core/src/mcp/transport.rs` 是否真支持 HTTP/SSE? [待确认] | 千寻-analysis §10 待确认 1 |
| R2 | `qianxun-core/src/provider/anthropic_compat.rs` 覆盖了哪些 provider? [待确认] | 千寻-analysis §10 待确认 2 |
| R3 | `qianxun/src/daemon/ui/build/` 实际渲染的页面能否驱动 4 个端点? [待确认] | 千寻-analysis §10 待确认 3 |
| R4 | `qianxun-core/src/agent/context/` 的 L1-L4 压缩在 worker 场景是否需要调整? | engine.rs:116-170 |
| R5 | `qianxun/src/daemon/persistence.rs` 的 conversation 反序列化需要哪些字段? | persistence.rs 全文 |

### 18.4 风险类 (监控)

| # | 风险 | 触发条件 | 缓解 |
|---|---|---|---|
| K1 | LLM Decompose 拆错 (环依赖 / 不可达 assignee) | 跑 100 次, 拆错 > 5% | 加 schema 严格校验 + 默认 fallback 单任务 |
| K2 | Worker 互相死锁 (黑板写等待) | 多 worker 跑 24h | 心跳 stale 触发 cancel + 写错误事件 |
| K3 | SQLite 单连接变瓶颈 | run/s 持续 > 100 | v2 评估 r2d2 pool (加 1 个新 crate, 需评估传递依赖) |
| K4 | Daemon 重启导致 in_progress run 全丢 | 经常 crash | KANBAN-RUNS 里有完整 task state, 启动时扫描 in_progress 标 stale (O9 决策) |
| K5 | 多 board 隔离不够 (共享 LLM 配额) | 多 board 同时高负载 | v2 加 per-board token budget |
| K6 | v3 跨机器 worker 派发的网络延迟 | 跨城市跑 worker | v3 决定: 走 outbox 异步 + 心跳续命 |

---

## 19. 附录 B: 完整端到端示例 (含具体文件路径)

下面给一个**从用户键入到 UI 反馈**的完整 trace, 串起本报告所有模块.

### 19.1 场景

用户在 `qianxun/src/` 工作目录下, 用 TUI (或 thin-client 探测到本地 Daemon) 发 prompt: "调研 Rust 2025 生态, 输出结构化报告".

### 19.2 步骤追踪

```
[Step 1: 用户输入]
  TUI 调 client::run_thin_repl → 探测 http://127.0.0.1:23900/v1/system/health
  探测成功 (thin-client 1211 行, 已有)

[Step 2: POST /v1/chat/session (如还没有)]
  → daemon::router::create_session (router.rs:90)
  → AgentLoopHost::create_session
  → SessionRuntime::new (session_runtime.rs:87) 构造 runtime
  → 写入 daemon_sessions 表 (persistence.rs:120-140)
  返回 session_id = "sess_20260602_233000_123456"

[Step 3: POST /v1/chat/session/{id}/prompt]
  → daemon::router::prompt_handler (router.rs:1059)
  → 拿 runtime, touch() (session_runtime.rs:126)
  → 构造 Conversation, push user message
  → 调 processing_loop::handle_user_message (engine.rs:83)
    ↑ MVP-1 后, 不再直接 provider.stream_completion

[Step 4: 处理循环内]
  4a. enforce_budget, normalize, compress (engine.rs:114-170)
  4b. build_request: memory + skills 注入 (缺口 3 修复)
      → memory.build_context() (memory/src/lib.rs:166)
      → skills.build_catalog_prompt() (skills/mod.rs, 加新方法)
  4c. LLM stream: "用户想调研, 我决定调 kanban_create"

[Step 5: LLM 调 kanban_create 工具]
  5a. ToolRegistry::execute_async_with_filter (tools/mod.rs:252)
      → 拿 KanbanCreate tool (新 builtin/kanban.rs:50)
      → scope check: runtime.kanban_scope.role == Orchestrator ✓
      → 校验 assignee_role "researcher" 在白名单 ✓
      → KanbanDb::create_task (kanban/db.rs)
        → INSERT INTO kanban_tasks (status=triage, assignee_role=researcher)
        → INSERT INTO kanban_events (kind=TaskCreated)
      → 返 { task_id: "task_abc1" } 给 LLM

[Step 6: LLM 收尾, finish_reason=end_turn]
  → build_turn (engine.rs:465-488)
  → conversation.push_message (assistant)
  → sink.on_turn_finished → emit SseEvent::MessageStop
  → save_snapshot (persistence.rs:140-160)  ← 缺口 6 修复, 真 JSONL

[Step 7: Daemon 后台 KanbanDispatcher loop]
  → tick (tokio::time::interval 2s)
  → dispatch_once (kanban/dispatcher.rs:50)
    7a. SELECT next ready task
        → "task_abc1" (triage 自动 ready, 因为是 root)
    7b. 找 idle profile: "prof_researcher-1"
    7c. INSERT INTO kanban_runs (status=running, claim_id=uuid)
    7d. UPDATE kanban_tasks SET status='in_progress', last_heartbeat_at=now
    7e. INSERT INTO kanban_events (kind=RunCreated, kind=TaskStarted)
    7f. emit SseEvent::KanbanTaskAssigned  ← 5 新事件之一
    7g. agent_host.spawn_session_for_task("task_abc1", "run_001", profile)
        → 新 SessionRuntime { kanban_scope: Worker { assigned_task_id: "task_abc1" } }
        → tokio::spawn(run_with_protection, §12.6 代码)

[Step 8: Worker session 跑 processing_loop]
  8a. system_prompt 注入: "你是 researcher, task_id=task_abc1,
                            必须 kanban_complete 收尾"
  8b. LLM 跑: 调 read_file (builtin), web_search (MCP)
  8c. 每次 LLM chunk 触发 heartbeat bridge (模式 7, 60s 限频)
      → spawn_blocking 写 kanban_runs.r_heartbeat_at
  8d. LLM 调 kanban_write_blackboard("current_focus", "调查 tokio")
      → 写 kanban_blackboard 表
      → emit SseEvent::KanbanBlackboardUpdate
  8e. LLM 完成, 调 kanban_complete({summary: "..."})
      → scope check: Worker ✓, task_id == assigned ✓
      → UPDATE kanban_runs SET status=done, ended_at=now, outcome=success,
                               summary=..., token_input=1.2K, token_output=0.3K
      → UPDATE kanban_tasks SET status=done, t_completed_at=now
      → recompute_parent (state_machine.rs)  → root 任务被唤醒
      → emit SseEvent::KanbanTaskCompleted

[Step 9: 用户在 TUI 看到]
  - 切到 Tasks 视图: "task_abc1" 已移入 Done 列
  - 选中回车, 看到 run timeline (Started → Heartbeat×N → Completed)
  - token 用量: 1.2K input + 0.3K output
  - 总耗时: 12.5s

[Step 10: techlead 角色 (若启动) 决定下一步]
  10a. techlead 角色的 system_prompt 注入 "你是 techlead 角色, board 状态变化时考虑加新任务"
  10b. techlead 角色 session 跑 (跟 worker 一样, 自己的 session_runtime)
  10c. 看到 task_abc1 done, 决定: "需要 verifier 验证 + synthesizer 综合"
  10d. 调 kanban_create(verifier task) + kanban_create(synthesizer task)
  10e. 调 kanban_link(verifier.depends_on=task_abc1, dep_type=Verifier)
  10f. 调 kanban_link(synthesizer.depends_on=verifier, dep_type=Synthesizer)
  10g. emit SseEvent::KanbanTaskSpawned × 2 (前端 DAG 实时更新)

[Step 11: Verifier 跑]
  跟 worker 一样, 但 system_prompt 用 reflect.rs::build_review_prompt 协议
  输出 "[Review: OK]" → 写 kanban_tasks.metadata.gate=pass
  → emit SseEvent::GatePass
  → Synthesizer 解锁 (depends_on=verifier AND metadata.gate=pass)

[Step 12: Synthesizer 综合]
  读 worker + verifier 的输出 + 黑板
  输出最终报告
  → emit SseEvent::KanbanTaskCompleted
  → 整个 board 全部 done
```

### 19.3 关键文件路径 (本次设计将创建/修改)

| 路径 | 状态 | 行数 (估) |
|---|---|---|
| `qianxun-core/src/kanban/mod.rs` | 新 | 30 |
| `qianxun-core/src/kanban/types.rs` | 新 | 180 |
| `qianxun-core/src/kanban/db.rs` | 新 | 350 |
| `qianxun-core/src/kanban/state_machine.rs` | 新 | 200 |
| `qianxun-core/src/kanban/dispatcher.rs` | 新 | 250 |
| `qianxun-core/src/kanban/error.rs` | 新 | 60 |
| `qianxun-core/src/blackboard/mod.rs` | 新 | 30 |
| `qianxun-core/src/blackboard/cell.rs` | 新 | 120 |
| `qianxun-core/src/agent/team.rs` | 新 | 250 |
| `qianxun-core/src/agent/pattern.rs` | 新 (修缺口 1) | 150 |
| `qianxun-core/src/tools/builtin/kanban.rs` | 新 | 350 |
| `qianxun-core/src/tools/mod.rs` | 改 | +20 |
| `qianxun-core/src/agent/mod.rs` | 改 | +3 (pub mod kanban / team) |
| `qianxun-core/src/agent/engine.rs` | 改 | +50 (pattern dispatch) |
| `qianxun-core/src/output.rs` | 改 | +10 (on_event) |
| `qianxun/src/daemon/kanban_host.rs` | 新 | 300 |
| `qianxun/src/daemon/team_registry.rs` | 新 | 200 |
| `qianxun/src/daemon/session_runtime.rs` | 改 | +10 (kanban_scope) |
| `qianxun/src/daemon/sse.rs` | 改 | +80 (5 新 variant) |
| `qianxun/src/daemon/router.rs` | 改 | +200 (8 路由) |
| `qianxun/src/daemon/mod.rs` | 改 | +30 (启动 dispatcher) |
| `qianxun/src/tui/kanban_view.rs` | 新 | 300 |
| `qianxun/src/tui/team_view.rs` | 新 | 150 |
| `qianxun/src/tui/mod.rs` | 改 | +50 (tab) |
| `qianxun/src/daemon/ui/src/routes/kanban/` | 新 | 200 |
| `qianxun/src/client/mod.rs` | 改 | +60 (5 新 SSE 事件) |
| `qianxun-core/tests/kanban_e2e.rs` | 新 | 400 |
| `docs/30_子项目规划/04-kanban-design.md` | 新 | (外部 doc) |
| `docs/30_子项目规划/_shared-contract.md` | 改 | +30 (8 端点 + 5 SSE) |
| `CLAUDE.md` | 改 | +20 (kanban 模块) |

**总计**: 新增 ~3500 行, 改 ~600 行 = ~4100 行.

---

## 20. 附录 C: 跟现有阶段路线的对齐 (A-G Phase)

本方案**显式对齐** `docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md` 的 A-G 阶段:

| Phase | 原路线 | 本方案触达 |
|---|---|---|
| A 文档事实源治理 | — | 修 docs 4 处 (CLAUDE.md / architecture.md / _shared-contract.md / 新增 kanban-design.md), 见 MVP-6 |
| B TUI 性能最小闭环 | ✅ 已完成 | 复用, 我们的 TUI Tasks 视图走脏标记 + 增量行缓存 |
| C Memory 闭环修正 | ✅ 已完成 | 复用, prompt_handler 走 build_context 拿真实 FTS 结果 |
| D MCP 与 Skills 接线 | ✅ 已完成 | 复用, kanban 工具走 ToolRegistry, 不破坏 builtin / mcp / skill 三层 |
| **E Daemon 真实运行时** | 🔴 进行中 | **MVP-1 直接对接** (修缺口 2/3/6) |
| **F Agent Patterns 与工具安全策略** | 📋 待开始 | **MVP-2 + MVP-3 直接对接** (修缺口 1 + 8) |
| G 清理旧入口与测试闸口 | 📋 待开始 | 我们的 e2e 测试进 G 阶段 `cargo test --workspace` 套件 |

**关键洞察**: 我们的 MVP-1 / MVP-2 / MVP-3 实际上**就是 Phase E + F 的子集**, 沿用它们的执行顺序. 这意味着我们可以**复用同一份时间盒**, 不需要独立排期, 跟其他 worker 协作.

---

## 21. 附录 D: 跟千寻 `docs/30_子项目规划/_shared-contract.md` 的契约

`_shared-contract.md` §3.1 现有 25+ 端点, 我们的新 8 端点 (§8.5) 直接挂在 `/v1/kanban/*` 命名空间下, 不与现有端点冲突. SSE 5 新事件 (§8.4) 走 `#[serde(tag = "type")]` 模式, 跟现有 12 事件**共用一个 SseEvent enum**, thin-client 解码逻辑自动分发.

**契约稳定性测试** (在 `qianxun-core/tests/contract.rs` 加):
- 序列化所有 17 SSE 事件, 验证 JSON tag 字段名不变
- 跟 `qianxun/src/client/mod.rs:108-174` 12 事件解码一一对应
- 跟 `qianxun/src/daemon/sse.rs:24-86` 12 事件编码一一对应
- 5 新事件在两边同时加, 防止 deserialization 不兼容

---

## 22. 附录 E: 风险评估矩阵 (按概率 × 影响)

| 风险 | 概率 | 影响 | 缓解策略 |
|---|---|---|---|
| R1: LLM Decompose 拆错 (环依赖 / 不可达 assignee) | 中 | 中 | 严格 JSON schema 校验 + 回退单任务, O2 决策 |
| R2: Worker 互相死锁 (黑板写等待) | 低 | 高 | 心跳 stale 自动 cancel, K2 风险 |
| R3: SQLite 单连接变瓶颈 | 极低 | 中 | MVP 单 daemon 低并发, v2 评估 r2d2 |
| R4: Daemon 重启导致 in_progress run 全丢 | 中 | 中 | kanban_runs 有完整 state, 启动时扫 in_progress 标 stale, O9 决策 |
| R5: 多 board 隔离不够 (共享 LLM 配额) | 极低 | 低 | v2 加 per-board token budget |
| R6: v3 跨机器 worker 派发的网络延迟 | 低 (v3 才出现) | 中 | v3 走 outbox 异步 + 心跳续命 |
| R7: Worker prompt injection 篡改兄弟任务 | 中 | 高 | 模式 3 护栏 (Worker scope + 结构化 context 注入) + 工具白名单 6 个 |
| R8: 4.6 节提到的 phase 3b 假象 (plan/reflect/workflow 孤儿) 误导他人 | 高 | 中 | MVP-2 真正接 pattern dispatcher, 文档同步更新 |
| R9: thin-client 5 新 SSE 事件破坏现有解码 | 中 | 高 | 契约稳定性测试 + 分支版本字段 (v1.1.0) |
| R10: 私有部署单机限制, 多用户需求出现 | 极低 | 中 | 走 VPS Server 路径 (Phase 4b 已有) |

---

## 23. 附录 F: 与 小A 自身的反思 (跨项目可迁移)

> 这一节是为本任务留的个人反思, 不会进 docs. 仅写给未来的自己.

- **借鉴 ≠ 照搬**: Hermes 7 个模式都"可借鉴", 但要拒绝它那 93KB dashboard plugin, 99KB credential pool, 250KB auxiliary_client. 借鉴模式不借鉴过度设计.
- **复用现有分层 = 降低风险**: 千寻的 `processing_loop` (489 行) 已经是成熟主循环, 不要重写; `ToolCategoryFilter` 已经是好的能力门控, 不要换. 任何"为多 Agent 重做一遍"的冲动都要克制.
- **任务/运行解耦是高价值**: Hermes 这条模式被多个分析报告强调, 我设计的 Kanban 完全照搬, 没有简化. 这是验证过的设计, 不要碰.
- **Team 不是类型**: 这是 Hermes 给的最反直觉但最有价值的洞察. 我差点在 §6.1 加 `Team` struct, 后改成 `TeamConfig` (只是预算) + `Profile` 集合. 关键判断是"用户能用 Kanban + Role 涌现出 Team, 不需要一等公民 Team".
- **in-process 优于跨进程**: 单 binary 部署的私有项目, 跨进程的运维成本 / 序列化成本远高于 in-process 性能损失. 永远先选 in-process, 真要跨机器才上 s6 / Docker.

可作为通用方法论: 借鉴开源项目时, 不仅学它**做了什么**, 更要学它**没做什么** (hermes-analysis §8 7 个不推荐).

---

## 24. 附录 G: 文件丢失与重建备注 (2026-06-03)

> 本节仅本版本 (重建版) 留, 后续合入 docs 时删.

**事件**: 2026-06-02 23:50 完成本报告首版 (8455 中文字 / 1891 行), 通过 verifier 接收, session 进入 idle. 2026-06-03 05:27-05:31 期间, 父 plan 推进到 stage8 / stage9c, 报告文件 `E:/git/maxu/qianxun/.小A/plans/qianxun-multi-agent-architecture.md` 被 plan 清理机制删除 (deliverable.md 仍在).

**重建**: 2026-06-03 05:32 收到 "文件不存在" 报告, 重读两份输入分析 + 5 份核心源码后重建本版, 结构 / 章节 / 引用 / 决策追溯表 100% 复刻首版, 文字内容已根据重建时的源码视图重写 (例如 §6.5 SQL schema 注释微调, §12.6 run_with_protection 代码示例微调, §7.1 状态机代码新增, §10.1 TUI 布局示意新增, §13.5/13.6 structured logging/OTEL 评估 新增).

**重建版字数**: 跟首版同量级, 8000+ 中文字 / 19 主章节 + 5 附录 (新加附录 G 备注本节).

**待 PM 决定**:
- (a) 接受本重建版, 跟原 deliverable.md 合并作为最终交付
- (b) 不需要, 后续 plan 阶段已不需要本报告 (stage8/9c 已完成), 删掉
- (c) 保留在 `E:/git/maxu/qianxun/.小A/plans/qianxun-multi-agent-architecture.md` 作为长期 reference, 不参与新 plan

---

## 18. 附录: 一句话总结

> 千寻的多 Agent 协作**不需要新概念**, 只需要把现有 `processing_loop` (engine.rs:83) 当 worker 复用, 把 `WorkflowTemplate` (workflow.rs:43) 当 swarm 模板复用, 把 `ToolCategoryFilter` (tools/mod.rs:25) 当 capability gating 复用, 把 `MemoryCore` 当黑板的存储复用, 把 `SessionStore` 扩 7 张 `kanban_*` 表, 把 SSE 12 事件扩 5 个新 variant — Hermes 教的 7 个模式直接照搬, 单 binary 部署不动, 8 周 MVP, 3600 行代码增量, 严格不引新 crate.

---

**报告路径**: `E:/git/maxu/qianxun/.小A/plans/qianxun-multi-agent-architecture.md`

**字数统计**: ~9000+ 字 (含 ASCII 图, 不含引用行号; 重建版)

**与两份分析报告的关系**:
- `hermes-analysis.md` (A) → §3.3, §4 (M1-M7), §14.4, §15.1 覆盖矩阵
- `qianxun-analysis.md` (B) → §3.1, §3.2, §15.2 覆盖矩阵
- 本报告: 综合 + 决策 + 落地

**版本**: 2026-06-03 重建版 (文件意外丢失后)

(End of report)






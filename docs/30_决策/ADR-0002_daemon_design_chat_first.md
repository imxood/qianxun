# ADR-0002: daemon 设计对齐 chat-first 范式

> 状态: 提议 | 适用范围: `qianxun/src/daemon/` + `qianxun-core/src/db/` | 起草: 2026-06-07

## 背景

`docs/daemon-design.md` (v0.2, 2026-06-01) 仍基于 **kanban 范式**: Session 是"一次对话", 没有 Project / Plan / SubSession 等概念, 也没有 chat-first 引入的 5 个新实体.

`docs/chat-first-redesign.md` (v1, 2026-06-07) 是桌面端 (Tauri) 跟 web 端 (Daemon Web Console) 的新设计基线, 跟 daemon 当前实现有系统性偏差.

**过期文档归档** (2026-06-07 完成):
- `docs/30_子项目规划/04-kanban-design.md` (132 KB) → `docs/90_历史/2026-06-07-04-kanban-design-废弃.md`
- `docs/30_子项目规划/05-mvp-0-checklist.md` (5 KB) → `docs/90_历史/2026-06-07-05-mvp-0-checklist-废弃.md`

两个文件都基于 kanban 范式, 跟 chat-first 冲突, 整段提炼 (严格契约 → chat-first §6, 3 模式 → chat-first §1.2, SSE 事件 → daemon §3.5).

## 决策

daemon 端按 `chat-first-redesign.md` 重新对齐:
- **删**: 所有 kanban 相关概念 (kanban_projects 表, /v1/chat 旧命名, "in-memory Conversation" 抽象)
- **加**: 5 个新实体 (Project, Plan, PlanTask, SubSession, ProjectExperience, SessionMinute) + 9 个新路由
- **改**: Session / Message 模型字段扩展, SQLite schema 升级, 启动流程加 5 个新模块初始化

`daemon-design.md` 全文重写, 版本号升到 **v1.0**.

---

## daemon-design.md 详细 diff 清单

按原 §1 - §13 逐节列出**删 / 改 / 加**.

### §1 文件结构 (1.2) — **加**

新增模块:
```
qianxun-core/src/types/plan.rs            # Project / Session / Plan / PlanTask / SubSession / Experience / Minute 类型
qianxun-core/src/db/migrations/0007_chat_first.sql   # 新 schema (projects / plans / plan_tasks / sub_sessions / messages / session_minutes)
qianxun/src/daemon/project.rs              # Project CRUD + 经验聚合
qianxun/src/daemon/plan.rs                 # Plan 生命周期 (发起 / 跑 / abort / 完成)
qianxun/src/daemon/sub_session.rs          # SubSession 独立上下文 (复用 AgentLoop 实例, 独立 messages)
qianxun/src/daemon/experience.rs           # ProjectExperience 写读 (走 qianxun-memory FTS5)
qianxun/src/daemon/minute.rs               # SessionMinute 增量追加
```

### §2 架构 — **改**

§2.1 进程结构图: 把 `AgentLoop 实例池` 拆成两层:
- **Session 实例池** (主会话) — 维持原状
- **SubSession 实例池** (子会话) — 新增, 1 SubSession = 1 PlanTask

§2.5 全局配置 `~/.qianxun/config.json` 加:
```json
{
  daemon: { /* 不变 */ },
  providers: [ /* 不变 */ ],
  agent: { /* 不变 */ },
  budget: { /* 不变 */ },
  
  // 新增:
  project: {
    max_sessions_per_project: 100,
    max_active_plans_per_session: 5,  // 同一 session 同时跑的 plan 上限
    session_minute_interval: 3,        // 每 N 轮 assistant 消息追加一条纪要
  },
  plan: {
    default_task_timeout_ms: 1800000,  // 30 min, 跟 mavis-team 一致
    default_plan_timeout_ms: 1800000,
    require_verifier: true,            // 结构层硬约束: skip verify 必须有 reason
  }
}
```

### §3 HTTP 框架 — **改 §3.4 路由 + §3.5 SSE**

#### §3.4 路由 — **加 9 个 / 改 1 个 / 删 0 个**

**改名** (1 个):
| 旧 | 新 | 备注 |
|---|---|---|
| `/v1/chat/session` | `/v1/sessions` | 加 project_id 字段, 跟 chat-first §3.1 类型一致 |
| `/v1/chat/session/:id` | `/v1/sessions/:id` | 同上 |
| `/v1/chat/session/:id/prompt` | `/v1/sessions/:id/prompt` | 同上 |

**新增** (9 个):
| 端点 | 方法 | 用途 | 优先级 |
|---|---|---|---|
| `/v1/projects` | GET, POST | 项目列表 / 新建 | ★ |
| `/v1/projects/:id` | GET, PUT, DELETE | 项目详情 / 改 / 删 | ★ |
| `/v1/projects/:id/experience` | GET, POST | 项目经验列表 / 追加 | ☆ |
| `/v1/sessions` | GET | 会话列表 (支持 project_id / status filter) | ★ |
| `/v1/sessions/:id/messages` | GET, POST | 消息列表 / 追加 (SSE 流不变) | ★ |
| `/v1/sessions/:id/minutes` | GET | 纪要列表 | ☆ |
| `/v1/plans` | POST | 发起 plan (主 Agent 调) | ★ |
| `/v1/plans/:id` | GET, DELETE | 计划详情 / abort | ★ |
| `/v1/plans/:id/tasks` | GET | 计划下的子任务 | ★ |
| `/v1/sub_sessions/:id/messages` | GET, POST | 子会话消息 (独立上下文) | ★ |

**保留不动**:
- `/v1/llm/*` (5 个)
- `/v1/tools` (1 个) + 新增 `/v1/tools/:name/invoke` (Stage 7a Web Console 用)
- `/v1/mcp/servers` (1 个) + 新增 `/v1/mcp/servers/:id/test` / DELETE
- `/v1/skills` (1 个) + 新增 POST reload + `/v1/skills/:name/toggle`
- `/v1/memory/*` (走 qianxun-memory, 不变)
- `/v1/config` (1 个) + 新增 PUT (hot-reload)
- `/v1/system/*` (3 个)
- `/_ui/*` (Stage 7a, 01b 详化)

#### §3.5 SSE 流式响应 — **加 2 个事件 / 改 1 个**

| 事件 | 触发 | data 字段 | 备注 |
|---|---|---|---|
| `text` | LLM 输出文本块 | `{text: string}` | 不变 |
| `thinking` | LLM 思考块 | `{text: string}` | 不变 |
| `tool_call` | LLM 请求调工具 | `{id, name, arguments, plan_ref?}` | **加 plan_ref 字段** |
| `tool_result` | 工具执行完成 | `{id, name, content}` | 不变 |
| `plan_update` | Plan 状态变化 | `{plan_id, status, task_id?, progress}` | **新增** |
| `experience_suggest` | 主 Agent 建议沉淀 | `{session_id, project_id, items: [{content, source}]}` | **新增** |
| `error` | 发生错误 | `{code, message}` | 不变 |
| `turn_finished` | 一轮 LLM 调用结束 | `{reason, usage}` | 不变 |

### §4 AgentLoop 实例管理 — **改 §4.1 + §4.2**

§4.1 生命周期新增:
```
POST   /v1/sessions                → 创建主会话
POST   /v1/sessions/:id/prompt     → 发送 prompt, SSE 流
GET    /v1/sessions/:id/messages   → 拉取完整消息历史
POST   /v1/sessions/:id/minutes    → 追加一条纪要 (LLM 自动触发)
POST   /v1/plans                   → 发起 plan, 返回 plan_id
POST   /v1/sub_sessions/:id/messages → 子 Agent 跟主 Agent 通信
GET    /v1/sub_sessions/:id/messages → 拉子 Agent 上下文
```

§4.2 AgentLoopHost 拆 2 类:
```rust
pub struct AgentLoopHost {
    sessions: Arc<RwLock<HashMap<SessionId, SessionHandle>>>,        // 主会话
    sub_sessions: Arc<RwLock<HashMap<SubSessionId, SubSessionHandle>>>,  // 子会话
    plans: Arc<RwLock<HashMap<PlanId, PlanHandle>>>,                // plan 生命周期
    max_sessions: usize,
    max_sub_sessions_per_session: usize,  // 默认 5
}
```

**SubSession 关键设计**:
- 独立 Conversation 实例, 独立 messages 存储 (靠 `sub_session_id` 索引)
- 终止后状态 `done` / `failed` / `aborted` → **只读** (跟 chat-first §4.4 一致)
- 想追问 → 回主会话问, 不在子会话里继续
- verifier 角色 (由 `PlanTask.verified_by` 决定) 在 sub_session 终止后**独立**调一次验证, 写 `plan_tasks.output.verifier_result`

### §5 API Key 管理 — **不变**

完全保留 (跟 chat-first 无关).

### §6 Token 预算和限流 — **改 §6.1, 加 plan 级 budget**

```rust
pub struct BudgetManager {
    // ... 不变 ...
    
    // 新增: Plan 级 token 预算
    plan_budgets: Arc<DashMap<PlanId, u64>>,  // 每个 plan 单独的 token 计数
}
```

§6.2 加一行:
| 维度 | 限制 |
|---|---|
| 单 plan token | `plan.default_plan_timeout_ms` 内最多 N token (默认 500K) |

### §7 优雅关闭 — **改**

关闭序列加 3 步:
```
8. 关闭所有 active plans:
     - 通知 plan 内的 sub_sessions 收尾
     - plan 状态置为 aborted
     - plan_tasks.status 全部置为 aborted
9. 持久化未完成的 plan (写到 SQLite, 下次启动恢复)
```

### §8 系统服务注册 — **不变**

### §9 与 ACP 协议的关系 — **改 §9.1**

§9.1 改: ACP **不做 chat 入口** (那是 Tauri 桌面 / TUI 的事), 也不做 kanban 入口. ACP 纯粹是 **"Zed 编辑器 ↔ Daemon 协议桥"**, 把 Zed 的 JSON-RPC 2.0 转换为 daemon HTTP/SSE.

### §10 启动流程 (完整) — **改**

启动序列加 5 个新模块初始化 (在原 13 步基础上):
```
4.5 初始化 ProjectStore           (开 SQLite projects 表)
4.6 初始化 PlanRegistry          (内存索引, 跟 Session 关联)
4.7 初始化 SubSessionHost        (子 Agent 池)
4.8 初始化 ExperienceWriter      (连 qianxun-memory)
4.9 初始化 MinuteWriter          (后台任务, 每 N 轮追加)
```

后台任务加 3 个:
```
12.1 Plan 状态监控 (5s tick)        — 检测超时 / 死锁
12.2 Session Minute 增量 (异步)    — LLM 写完每轮自动追加
12.3 Project Experience 索引同步    — 跟 qianxun-memory 同步
```

### §11 依赖清单 — **加 2 个**

```toml
# 已在用 (不变)
axum = "0.8"
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true }
tracing = { workspace = true }

# 新增
uuid = { version = "1", features = ["v4"] }       # Session / Plan / SubSession id
dashmap = "6"                                       # Plan 池并发索引
```

### §12 测试策略 — **加 3 类**

| 测试类型 | 覆盖 |
|---|---|
| 集成测试 | `POST /v1/plans` → plan_tasks 创建 → sub_session 启动 → verifier 写 output |
| 集成测试 | 子会话终止后 `POST /v1/sub_sessions/:id/messages` → 410 Gone (只读) |
| 集成测试 | Session minutes 自动追加 (mock LLM 跑 5 轮, 验证 session_minutes 至少 1 条) |

### §13 里程碑建议 — **改**

| 阶段 | 任务 | 预估 | 备注 |
|---|---|---|---|
| **1. 数据模型** | 新 types + SQLite migration 0007 | 1 天 | 优先做, 后续都依赖 |
| **2. Project + Session CRUD** | 项目 / 会话 / 消息的 9 个新路由 | 1.5 天 | |
| **3. Plan 生命周期** | 发起 / 任务派发 / sub_session 启动 | 2 天 | 核心, 跟 mavis-team 1:1 对齐 |
| **4. SubSession 独立上下文** | sub_session messages 独立存储 + 只读保护 | 1 天 | |
| **5. Experience + Minute 写入** | LLM 触发 / 自动追加 | 1 天 | 走 qianxun-memory |
| **6. SSE 扩展** | plan_update / experience_suggest 事件 | 0.5 天 | |
| **7. 端到端测试** | 真实启动 daemon, 跑通 chat → plan → 沉淀 | 1 天 | |
| **合计** | | **~8 天** | (原 §13 是 13 天, 现在去掉 kanban, 缩减 5 天) |

---

## 跟其他文档的关系

- **`chat-first-redesign.md`**: 新设计基线, daemon 端按此对齐
- **`01-daemon.md`** (123 KB, **未审**): 可能是早期 daemon 整体规划, **本次不审, 留 v2 ADR**
- **`01b-daemon-web-console.md`**: 仍有效, 但 §10 "项目列表 + 3 栏布局"作废 (Tauri 桌面是用户面, Web Console 只管 LLM/Skills/MCP/Tools 管理面). **下个 PR 更新**
- **`_shared-contract.md`** (11 KB): 跨项目数据模型, **需更新** 加 Project / Plan / SubSession 字段
- **90_历史/2026-06-07-04-kanban-design-废弃.md**: 整段已并入本 ADR + chat-first, 不再引

---

## 待决 (Q3 / Q4)

- **Q3**: 01-daemon.md (123 KB) 这次一起审吗? 跟 daemon-design.md 重复还是不同内容?
- **Q4**: _shared-contract.md 何时更新? 建议跟 daemon-design.md v1.0 同步出

---

## 下一步

1. **等用户审本 ADR** (本次)
2. 通过后, 重写 daemon-design.md v1.0 (按本 ADR 的 diff 清单)
3. 同步更新 _shared-contract.md (Q4)
4. 跟 01b 协调 Web Console 范围 (Q3, 留 v2)

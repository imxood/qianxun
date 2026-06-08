# 千寻 Chat-First 重构设计文档

> 状态: 草案 v1 · 2026-06-07 · 作者: Mavis + maxu
> 适用范围: qianxun-desktop (Tauri 端) / qianxun-daemon web console / ACP client
> 参考实现: `C:\Users\maxu\.mavis\agents\mavis\.builtin-skills\mavis-team` (生产级多 agent 编排范式)

---

## 1. 背景与目标

### 1.1 现状问题

经过 3 轮迭代, 当前千寻桌面端的设计存在两个方向的偏离:

| 维度 | 当前 | 用户真实需求 |
|---|---|---|
| 主工作区 | VSCode IDE 风 (文件树 + 编辑器 + 终端 + tab) | **Chat 流** (跟 LLM 的对话) |
| 任务组织 | Kanban 看板 (拖拽卡片) | **项目 + 会话** 树 (按主题) |
| 子任务展示 | 嵌入 Kanban 卡片 | **Plan 调用块** (内嵌子会话列表) |
| 子 Agent 上下文 | 临时窗口 | **持久化子会话** (可点入) |
| 经验沉淀 | 无显式机制 | **项目经验 + 会话纪要** 双层 |

Kanban 适合"任务分派"场景, 但千寻的真实心智是 **"跟 AI 一起干活"**, Kanban 反而打断了 Chat 节奏.

### 1.2 重构目标

- 把"以 Chat 为中心"作为主工作区, 文件 / 终端 / 子任务都收敛到 Chat 上下文里
- 把"项目"作为**有记忆的容器**, 跨会话沉淀经验
- 把"会话"作为**独立可重入**的对话单元, 不再被 Kanban 状态机绑架
- 复用 mavis-team 的 **5 个 primitive** (Plan / Task / Worker / Verifier / Deliverable), 不重新发明
- 3 列布局, 所有分隔线可拖, 移动端友好

### 1.3 设计原则 (5 条)

1. **Chat 优先** — 主工作区永远是 Chat 流, 其它内容都围绕 Chat 上下文展开
2. **容器记忆** — 项目是带"经验"的有状态容器, 不是文件夹别名
3. **契约严格** — Plan 跟 mavis-team task schema 1:1 对齐, 子任务有 verifier 独立 re-derive
4. **可重入** — 任何会话 / 子会话可暂停可恢复, 上下文持久化
5. **克制自动化** — 经验沉淀显式触发, 不每次消息都写 (避免噪音)

---

## 2. 信息架构: 5 个核心实体

```
┌────────────────────────────────────────────────────────────────────────┐
│  Project (项目)                                                        │
│  ├─ meta: id, name, folder?, provider, created_at, last_active_at     │
│  ├─ experience: ProjectExperience[] (跨会话沉淀, 主 Agent 显式写)     │
│  └─ sessions: Session[]                                               │
│                                                                        │
│  ┌────────────────────────────┐  ┌────────────────────────────┐        │
│  │ Session (主会话)            │  │ Session (主会话)            │        │
│  │ ├─ meta: id, title, ...    │  │ ├─ meta: id, title, ...    │        │
│  │ ├─ minutes: SessionMinute  │  │ ├─ minutes                 │        │
│  │ ├─ plans: Plan[]           │  │ └─ plans                   │        │
│  │ └─ messages: Message[]     │  │     (空)                    │        │
│  │     [User / Assistant]     │  │                            │        │
│  │       └─ tool_calls: [...] │  │                            │        │
│  │           └─ Plan (call)   │  │                            │        │
│  │              ├─ tasks: [3] │  │                            │        │
│  │              ├─ sub_sess[] │  │                            │        │
│  │              └─ result     │  │                            │        │
│  └────────────────────────────┘  └────────────────────────────┘        │
└────────────────────────────────────────────────────────────────────────┘
```

### 2.1 实体定义

| 实体 | 关键字段 (跟 mavis-team 对齐) | 存储位置 |
|---|---|---|
| **Project** | `id, name, folder?, provider, default_model, created_at` | daemon SQLite `projects` 表 |
| **Session** | `id, project_id, title, provider, model, status, created_at, last_active_at, message_count` | daemon SQLite `sessions` 表 |
| **Message** | `id, session_id, role, content, tool_calls?, plan_ref?, created_at` | daemon SQLite `messages` 表 |
| **Plan** | `id, session_id, contract (JSON), status, started_at, ended_at, result?, attachments[]` | daemon SQLite `plans` 表 |
| **PlanTask** | `id, plan_id, title, prompt, assigned_to, verified_by, verify_prompt, status, output, depends_on[]` | daemon SQLite `plan_tasks` 表 (1:1 借 mavis-team task schema) |
| **SubSession** | `id, plan_id, plan_task_id, parent_session_id, role, status, messages[]` | daemon SQLite `sub_sessions` 表 + 共享 messages 表 (sub_session_id 索引) |
| **ProjectExperience** | `id, project_id, content, source_session_id, source_plan_id?, tags[], created_at` | 走 `qianxun-memory` (SQLite + FTS5) |
| **SessionMinute** | `id, session_id, content, message_count_at_minute, created_at` | daemon SQLite `session_minutes` 表 |

### 2.2 跟 mavis-team primitive 的对应

| mavis-team | 千寻 qianxun | 复用度 |
|---|---|---|
| Plan (YAML) | `Plan` + 嵌套 `PlanTask[]` | 100% 复用 task schema 字段 |
| Worker Session | `SubSession` | 100% 复用 session 概念, 多了 `parent_session_id` |
| Verifier (agent) | `PlanTask.verified_by` | 100% 复用, 由 Plan 内每个 task 决定 |
| deliverable.md | `PlanTask.output` + `Plan.attachments[]` | 100% 复用, output 是结构化 JSON 而非 md |
| CycleReport | 内联到 Chat 流 (Assistant message 包含 `tool_calls[Plan]`) | 形态变了, 心智一致 |
| MEMORY.md (agent 级) | `ProjectExperience` (project 级) | 同构, scope 从 agent 缩到 project |

---

## 3. 数据模型 (TypeScript / Rust 双向)

### 3.1 共享类型 (`qianxun-core/src/types/plan.rs` + `qianxun-desktop/src/lib/types/plan.ts`)

```rust
// ─── Rust 端 (qianxun-core) ─────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,                  // "proj_xxx"
    pub name: String,                // "qianxun-desktop"
    pub folder: Option<String>,      // "E:/git/maxu/qianxun/qianxun-desktop"
    pub provider: String,            // "deepseek"
    pub default_model: String,       // "deepseek-v4-flash"
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub title: String,               // 首条消息摘要, 主 Agent 命名
    pub provider: String,
    pub model: String,
    pub status: SessionStatus,       // Active | Idle | Archived
    pub message_count: u32,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub session_id: String,          // 归属的主会话
    pub contract: PlanContract,      // 严格契约 (mavis-team task schema 子集)
    pub status: PlanStatus,          // Pending | Running | Done | Failed | Aborted
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub result: Option<PlanResult>,  // done 时填充
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanContract {
    pub name: String,                // "实现 JWT 用户认证"
    pub description: String,         // 一句话目标
    pub tasks: Vec<PlanTaskSpec>,    // 1+ 个子任务, 1 个 = 1 个可验证交付物
    pub timeout_ms: u32,             // 整体超时 (默认 1800000 = 30 min, 跟 mavis-team 一致)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTaskSpec {
    pub id: String,                  // "design-schema"
    pub title: String,               // "设计 users 表 + 索引"
    pub prompt: String,              // 给子 Agent 的 spec
    pub assigned_to: String,         // 子 Agent 角色: "coder" | "researcher" | "tester" | ...
    pub verified_by: Option<String>, // "verifier" | "code-reviewer" | "tester" | null
    pub verify_prompt: Option<String>,
    pub depends_on: Vec<String>,     // 任务依赖
    pub timeout_ms: u32,             // 单任务超时
    pub output: Option<OutputSpec>,  // 期望的产物形状
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubSession {
    pub id: String,
    pub plan_id: String,
    pub plan_task_id: String,        // 1 个 SubSession = 1 个 PlanTask
    pub parent_session_id: String,   // 归属主会话
    pub role: String,                // 跟 PlanTaskSpec.assigned_to 一致
    pub status: SubSessionStatus,    // Active | Done | Failed | Aborted | ReadOnly
    pub messages: Vec<Message>,      // 子 Agent 跟主 Agent 的对话
    pub output: Option<serde_json::Value>, // 跟 mavis-team deliverable 对齐
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}
```

### 3.2 Rust 端数据存储 (SQLite schema, daemon)

```sql
-- projects
CREATE TABLE projects (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  folder        TEXT,
  provider      TEXT NOT NULL,
  default_model TEXT NOT NULL,
  created_at    TEXT NOT NULL,
  last_active_at TEXT NOT NULL
);

-- sessions
CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,
  project_id    TEXT NOT NULL REFERENCES projects(id),
  title         TEXT NOT NULL,
  provider      TEXT NOT NULL,
  model         TEXT NOT NULL,
  status        TEXT NOT NULL,           -- active | idle | archived
  message_count INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  last_active_at TEXT NOT NULL
);
CREATE INDEX idx_sessions_project ON sessions(project_id, last_active_at DESC);

-- plans
CREATE TABLE plans (
  id          TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL REFERENCES sessions(id),
  contract    TEXT NOT NULL,             -- JSON
  status      TEXT NOT NULL,             -- pending | running | done | failed | aborted
  started_at  TEXT,
  ended_at    TEXT,
  result      TEXT,                      -- JSON
  attachments TEXT                       -- JSON array
);
CREATE INDEX idx_plans_session ON plans(session_id, started_at DESC);

-- plan_tasks (1 Plan = N PlanTask, 1:1 借 mavis-team task schema)
CREATE TABLE plan_tasks (
  id            TEXT PRIMARY KEY,
  plan_id       TEXT NOT NULL REFERENCES plans(id),
  title         TEXT NOT NULL,
  prompt        TEXT NOT NULL,
  assigned_to   TEXT NOT NULL,
  verified_by   TEXT,
  verify_prompt TEXT,
  depends_on    TEXT NOT NULL DEFAULT '[]', -- JSON array
  status        TEXT NOT NULL,           -- pending | running | done | failed | aborted
  output        TEXT,                    -- JSON
  started_at    TEXT,
  ended_at      TEXT
);
CREATE INDEX idx_plan_tasks_plan ON plan_tasks(plan_id);

-- sub_sessions
CREATE TABLE sub_sessions (
  id                TEXT PRIMARY KEY,
  plan_id           TEXT NOT NULL REFERENCES plans(id),
  plan_task_id      TEXT NOT NULL REFERENCES plan_tasks(id),
  parent_session_id TEXT NOT NULL REFERENCES sessions(id),
  role              TEXT NOT NULL,
  status            TEXT NOT NULL,       -- active | done | failed | aborted | readonly
  output            TEXT,                -- JSON
  started_at        TEXT NOT NULL,
  ended_at          TEXT
);
CREATE INDEX idx_sub_sessions_plan ON sub_sessions(plan_id, plan_task_id);
CREATE INDEX idx_sub_sessions_parent ON sub_sessions(parent_session_id);

-- messages (主会话 跟 子会话 共享, 靠 session_id / sub_session_id 二选一)
CREATE TABLE messages (
  id            TEXT PRIMARY KEY,
  session_id    TEXT REFERENCES sessions(id),       -- 主会话消息
  sub_session_id TEXT REFERENCES sub_sessions(id),  -- 子会话消息
  role          TEXT NOT NULL,                      -- user | assistant | system
  content       TEXT NOT NULL,
  tool_calls    TEXT,                               -- JSON array
  plan_ref      TEXT REFERENCES plans(id),          -- assistant 消息引用的 plan
  created_at    TEXT NOT NULL,
  CHECK ((session_id IS NOT NULL) OR (sub_session_id IS NOT NULL))
);
CREATE INDEX idx_messages_session ON messages(session_id, created_at);
CREATE INDEX idx_messages_sub_session ON messages(sub_session_id, created_at);

-- session_minutes (会话纪要, 增量追加)
CREATE TABLE session_minutes (
  id            TEXT PRIMARY KEY,
  session_id    TEXT NOT NULL REFERENCES sessions(id),
  content       TEXT NOT NULL,
  message_count INTEGER NOT NULL,
  created_at    TEXT NOT NULL
);

-- project_experience 走 qianxun-memory (SQLite + FTS5, 跨项目)
-- schema 见 docs/memory-design.md
```

---

## 4. UI 布局: 3 列结构

### 4.1 总览

```
┌─ Col 1 (240-360px, 可拖) ─┬─ Col 2 (flex, 最小 480px) ────────────┬─ Col 3 (280-400px, 可拖) ──┐
│                            │                                        │                              │
│ + 新建任务                 │ Chat 流 (主会话)                       │ Context (contextual)         │
│ 🔍 搜索                    │                                        │                              │
│                            │  [User]    帮我加个 JWT 登录            │  Plan: 实现 JWT 登录         │
│ 定时任务                   │  [Assist]  好的, 我先拉个 Plan 拆任务   │  ──── 契约 ────              │
│  · 记忆维护        ●       │  ┌─ Plan 块 (完整展开) ─────────────┐  │  启动: 19 min ago            │
│                            │  │ Plan: 实现 JWT 登录 [2/3] [取消]  │  │  超时: 30 min                │
│ 任务历史          14       │  │ ──────────────────────────────  │  │  verifier: tester+reviewer  │
│  ▸ 实现 JWT 用户认证 ◀ ●  │  │ ✓ 设计 users 表  coder  PASS     │  │  依赖: t0 → t1 → t2          │
│  · EIP 测试程序确认         │  │   + 2 files · 12 min  [子会话]   │  │                              │
│                            │  │   + 2 files · 12 min  [子会话]   │  │                              │
│ ──────                     │  │ ✓ 实现 JWT 签发  coder  PASS     │  │  ──── 变更文件 ────────  │
│ ⚙ 设置                     │  │   + 3 files · 8 min   [子会话]   │  │  + src/auth/jwt.ts           │
│ 🔌 离线 (3)                │  │ ⏳ 写单测         tester          │  │  + src/auth/users.ts         │
│                            │  │   + 1 file · 4 min   [子会话]     │  │  + src/auth/bcrypt.ts        │
│                            │  │ ──────────────────────────────  │  │  ~ src/routes/login.ts        │
│                            │  │ 已修改 6 文件 · 2/3 · 等待 verifier │ │  ~ src/middleware/auth.ts     │
│                            │  └──────────────────────────────────┘  │  + tests/auth.test.ts (run)   │
│                            │                                        │                              │
│                            │  [User]    顺便帮我加个 dark mode      │                              │
│                            │  [Assist]  ...                         │                              │
│                            │                                        │                              │
│                            │  ┌─ 输入框 ────────────────────────┐   │                              │
│                            │  │ @项目   发送   附件   模型 ▼    │   │                              │
│                            │  └────────────────────────────────┘   │                              │
└────────────────────────────┴────────────────────────────────────────┴──────────────────────────────┘
                                  ↕ 拖动调整          ↕ 拖动调整
```

### 4.2 三列职责

| 列 | 默认宽度 | 内容 | 收起行为 |
|---|---|---|---|
| **Col 1** (扁平任务侧栏) | 260px | 顶部: **+ 新建任务** (小链接) + 搜索框; 中间: 定时任务 / 任务历史 (平铺) / 项目 (可展开看任务) / Agent 团队; 底部: 用户信息 + 主题切换 | 收起到 0 (显示汉堡按钮) |
| **Col 2** (Chat 主工作区) | flex | 主会话消息流 + Plan 工具调用块 (完整 3 task + 打开子会话) + 输入框 | 最小 480px, 不可收起 |
| **Col 3** (Context 检查器) | 260-320px | contextual: Plan 契约 / 变更文件 / 项目信息; 新会话时: 归类提示 + 快捷键 | 可拖到 0 |

**Col 1 设计原则**:
- **扁平 > 嵌套**: 不强制按项目分组, 任务历史是一个**平铺列表** (按时间倒序, 最新的在上面)
- **入口是配角**: "+ 新建任务" 是个**小链接** (跟搜索/任务历史/项目平级), 不是大品牌按钮
- **搜索是核心**: 搜索框放在 "+ 新建任务" 下面, 跨任务/文件/命令全文搜
- **项目是分组维度, 不是唯一维度**: 侧栏有独立的"项目"段, 点击展开看项目下的任务列表. 任务历史 ≠ 项目, 是不同视角 (时间序 vs 分组)
- **个人化**: 底部用户信息 (马许 / Ultra Plan) + 主题切换, 一目了然

**项目 (Projects) 段的设计**:
- 千寻没有"已归档"概念, 项目是**一等公民**
- 项目展示为可点击行, 带 chevron + folder 图标
- 点击项目: 展开其下的**任务列表** (跟任务历史平铺列表同样的 UI 模式, 只是缩进 + 边框)
- 再点击任务: 进入对应 Chat (Col 2 切到该 Chat)
- 项目展开是**纯本地 UI 状态**, 不影响 Chat 数据; 关闭重开侧栏会自动收起
- 多个项目同时展开是允许的, 但默认只展开当前活跃的那个

**侧栏 4 段内容的关系**:
| 段 | 视角 | 用途 |
|---|---|---|
| **定时任务** | 调度维度 | 系统任务 (记忆维护等), 有状态指示 (蓝点) |
| **任务历史** | 时间维度 | 最近访问的所有任务, 平铺, 按时间倒序 |
| **项目** | 分组维度 | 按项目聚合, 点击展开看项目下任务 |
| **Agent 团队** | 协作维度 | 多 Agent 协作的固定组合 (未来扩展) |

**4 段之间有重叠, 但视角不同, 不强求互斥**:
- 同一个任务"实现 JWT 认证" 可能同时出现在: 任务历史 (最近) + 千寻桌面端项目下 (分组) + (未来) 某个 Agent 团队成员的工作记录
- 用户根据当下意图选不同入口, 不用纠结去重

### 4.3 分隔线规则

- Col 1 ↔ Col 2 分隔线: 可拖, 范围 [0, 480px], 拖到 0 收起 Col 1
- Col 2 ↔ Col 3 分隔线: 可拖, 范围 [0, 600px], 拖到 0 收起 Col 3
- 移动端 (< 768px): Col 1 默认收起, Col 3 完全隐藏, 顶部汉堡按钮呼出 Col 1

### 4.4 Col 2 / Col 3 职责分工 (避免重复)

**核心原则**: Col 2 是 **Plan 列表的唯一来源** (含 task 状态 + 打开子会话 + verifier 结果). Col 3 只放 Col 2 没有的周边上下文, 不再列 Tasks / Sub-sessions. "计划列表" 跟 "子会话" 是同一个东西, 1 个 task = 1 个 sub-session, 不分两个.

| 维度 | Col 2 (Chat 流) | Col 3 (Inspector) |
|---|---|---|
| **Plan 块** | **完整展开**: 名字 + 状态 + 3 task 行 (含 verifier + 打开子会话) + 底部 summary | 契约信息 (启动时间/超时/verifier 配置/依赖图) |
| **任务列表** | **唯一详细列表** (Col 2 Plan 块内) | **不列** (避免重复) |
| **子会话入口** | **唯一入口** (每个 task 行末 "打开子会话" 链接) | **不列** (避免重复) |
| **执行结果 / 附件** | **Plan 块内展示** (Plan 完成时: Deliverable 列表 + 附件 chips) | 不重复 |
| **变更文件** | 不在 Plan 块内 (避免挤) | 完整列表 (按 task 归属分组) |
| **verifier 状态** | 内嵌到 task 行 (PASS / FAIL / 等待中) | verifier 角色配置 (tester + code-reviewer) |
| **多 Plan 处理** | Chat 流里按时间顺序并排多个 Plan 块, 跟消息一样自然 | 永远只显示最新 active Plan 的契约 (历史 Plan 不重复) |

**心智模型**: Plan 块就是 Chat 流的"一等公民", 跟 User / Assistant 消息同级, 不是从属. Col 3 是**周边上下文**, 不抢主舞台.

### 4.5 Col 3 contextual 内容规则

| Col 2 状态 | Col 3 内容 |
|---|---|
| 无 active plan, 有项目 | 项目信息 + 最近会话 + 项目经验摘要 |
| Plan running | Plan 契约 (启动/超时/verifier/依赖) + 变更文件 |
| Plan done | Plan 契约 (done 状态) + 变更文件 + [查看 MR] [沉淀经验] |
| 打开子会话 (主会话 + 子会话模式) | 子会话元信息 + 父 Plan 引用 + "回到主会话" 链接 |
| 未绑项目 (新会话) | 提示用户绑项目或继续游离 |

---

## 5. 关键交互流程

### 5.1 新建任务 (新 Chat, 不搞居中页)

**核心决定**: **新任务 = 新 Chat**. 不弹窗, 不居中页, 不搞 logo / slogan / 5 个模板. 跟现有 Chat UI 一样, 只是空白 + 输入框聚焦. 主 Agent 第一个消息发出后才落库归类.

```
[Col 1 顶部 "+ 新建任务" 小链接 (或 ⌘N)]
  ↓
[Col 2 切到空白 Chat, 输入框聚焦, 焦点闪]
  ┌─ Col 2 ─────────────────────────────────────┐
  │                                               │
  │  ┌─ Chat header ──────────────────────────┐  │
  │  │ 新会话 · 还没开始                        │  │
  │  └─────────────────────────────────────────┘  │
  │                                               │
  │         开始一个新任务                          │
  │   第一个消息发出后自动归类到项目或 Chat         │
  │                                               │
  │  [未选 ▼]  [main ▼]  [MiniMax-M3]              │
  │  ↑ 文件夹 = 归类决定                           │
  │  ┌─ 输入框 (focused, brand 边框) ──────────┐ │
  │  │ 输入消息开始...                          │ │
  │  │                                  [↑]    │ │
  │  └─────────────────────────────────────────┘ │
  └───────────────────────────────────────────────┘
  ↓
用户输入第一条消息 (或从搜索框历史拉一句)
  ↓
[点击发送 / Enter]
  ↓
session 创建落库 (title 从首条消息自动生成)
  ↓
[Col 1 任务历史顶部插入新会话]:
  - 选了文件夹 (e.g. qianxun) → 落入对应项目 (高亮项目分组或加项目小标签)
  - 未选 (选了 "未选" chip) → 落入 "Chat" 分类
  ↓
[Col 2 切到 Chat 流, 主 Agent 响应, 进入正常对话]
```

**为什么不搞居中页**:
- "千寻 logo / slogan / 5 个模板" 看着像 landing page, 跟千寻作为"工作工具"的心智不符
- 用户每天开 5-20 个新任务, 每个都看一遍居中页是负担
- 空白 Chat + 输入框 = 最低延迟开始, 跟 Claude Code / Cursor 一致
- 上下文 (folder/model) 是**辅助**, 放在输入框上方小 chip, 不抢主舞台
- **不引入分支/branch 概念** (千寻不做 git 集成, 上下文只有文件夹 + 模型)

**Col 1 "新建任务" 是小链接, 不是大按钮的理由**:
- 跟参考图 (类似任务的 AI 工具) 心智一致
- 用户的肌肉记忆是 "点这个就开始打字", 不需要先看清楚按钮是什么
- 侧栏的视觉重心是**任务历史** (那是用户 90% 时间在看的东西), 不是入口按钮

### 5.2 主会话发起 Plan

```
[User] 帮我加个 JWT 登录, 用 bcrypt 加密
  ↓
[Assist 消息 1] 好的, 这涉及 schema 变更 + API + 测试, 我先拉个 Plan
  ↓
[Assist 消息 2 - 包含 tool_calls: [Plan]]
  ┌─ Plan tool-call 块 ──────────────────┐
  │ 实现 JWT 用户认证 · running           │
  │ ──────                                │
  │ 3 tasks · 2 done · 1 running          │
  │ ┌──────────────────────────────────┐ │
  │ │ ✓ 设计 users 表 + 索引           │ │
  │ │   coder · 12 min · 2 files       │ │
  │ ├──────────────────────────────────┤ │
  │ │ ✓ 实现 JWT 签发与校验            │ │
  │ │   coder · 8 min · 3 files        │ │
  │ ├──────────────────────────────────┤ │
  │ │ ⏳ 写认证流程单测 (running)       │ │
  │ │   tester · 4 min in              │ │
  │ └──────────────────────────────────┘ │
  │ [查看 deliverable] [打开子会话]       │
  └──────────────────────────────────────┘
  ↓ (主 Agent 持续 stream, 直到 plan 完成)
[Assist 消息 3] Plan 完成了. 主要改动如下: ...
  ↓
  [Col 3 自动更新: Plan 契约 + 完整 Tasks 列表 (含 verifier 状态, 内嵌子会话入口) + 变更文件清单]
  (Col 2 的 Plan 块只显示摘要, 不重复 task 列表)
  ↓
[Col 2 出现 "📝 沉淀到项目经验" 按钮 - 主 Agent 建议, 用户确认后写入]
```

### 5.3 查看子会话

```
[Col 3 点击 "打开子会话" 在 task "写认证流程单测" 上]
  ↓
[Col 2 顶部出现 tab bar]
  ┌────────────────────────────────────────┐
  │ [主会话: 实现 JWT] [子会话: 写单测 ✕] │
  └────────────────────────────────────────┘
  ↓
[Col 2 内容切换到子会话的消息流]
  - 子 Agent: "好的, 我会写以下测试: ..."
  - 子 Agent 工具调用 (grep, read file, edit)
  - 子 Agent: "完成, 共 8 个 test cases"
  - 子 Agent output: { test_count: 8, coverage: 0.92, files_changed: [...] }
  ↓
[Col 3 切换到子会话 context]
  - 子会话元信息 (role, status, started_at, ended_at)
  - 父 Plan 引用 (点击跳回主会话的 plan 块)
  - "回到主会话" 按钮
  ↓
子会话终止后, 状态变 "Done" (只读), 想追问回主会话
```

### 5.4 项目经验沉淀

```
[主会话 Plan 完成]
  ↓
主 Agent 内部 system prompt 触发经验检查:
  "以下信息值得写入项目经验吗? (yes/no)
   - '本项目用 Tailwind v4 OKLCH tokens, 不用 v3 hex'
   - 'JWT 用 jose 库, 不用 jsonwebtoken (TypeScript 友好)'"
  ↓
[Col 2 出现提示气泡]
  ┌────────────────────────────────────┐
  │ 💡 建议沉淀到项目经验:             │
  │                                    │
  │ • 本项目用 jose 库做 JWT           │
  │ • bcrypt 加密, rounds=12           │
  │                                    │
  │ [沉淀] [跳过] [修改]               │
  └────────────────────────────────────┘
  ↓
[用户点 "沉淀"]
  ↓
写入 project_experience 表 (走 qianxun-memory FTS5 索引)
  ↓
[Col 3 项目信息 → 经验列表 增加 2 条]
  ↓
[下次新建会话时, system prompt 自动注入 top-5 相关经验]
```

### 5.5 会话纪要增量

```
[主 Agent 每轮 Assistant 消息结束后]
  ↓
后台异步任务 (不阻塞 Chat):
  - 取本轮新消息
  - 调用 LLM 生成 50-100 字摘要
  - 追加到 session_minutes 表 (不覆盖)
  ↓
[Col 2 顶部 (或 Col 3 头部) 显示纪要徽章]
  📋 纪要已更新 (共 5 条)  [展开 ▼]
  ↓
[点开展开 - 折叠列表]
  ┌──────────────────────────────────┐
  │ 📋 会话纪要                       │
  │ ──────                            │
  │ 1. 用户要求加 JWT 登录            │
  │ 2. 主 Agent 拉 Plan 拆 3 任务     │
  │ 3. users 表 + 索引已建            │
  │ 4. JWT 签发完成, 准备测试         │
  │ 5. 决定用 jose 库 + bcrypt 12     │
  └──────────────────────────────────┘
```

---

## 6. Plan Contract 跟 mavis-team 对齐

### 6.1 共用字段 (100% 复用)

```yaml
# qianxun PlanContract.tasks[] 跟 mavis-team tasks[] 字段完全对齐
- id: <string>                    # 唯一
  title: <string>                 # 中文, 用户可见
  prompt: <string>                # 给子 Agent 的 spec (self-contained)
  assigned_to: <string>           # 子 Agent 角色名
  verified_by: <string|null>      # verifier 角色名
  verify_prompt: <string|null>    # verifier 独立 re-derive 的指令
  depends_on: [<string>]          # 任务依赖
  timeout_ms: <number>            # 单任务超时, 默认 1800000 (30 min)
```

### 6.2 差异点 (qianxun 适配)

| 差异 | mavis-team | qianxun | 原因 |
|---|---|---|---|
| 编排者 | orchestrator (owner) | **主 Agent** (在 Chat 流里) | 连续 chat 心智 |
| 启动方式 | `mavis team plan run` | 主 Agent 工具调用 | 嵌入 Chat |
| 周期 | 一次性 | **可中断可继续** | 用户在 Chat 中能随时打断 |
| CycleReport | 单独 dashboard | **内联到 Chat 流** (Assistant message 含 plan 块) | 不离开 Chat 心流 |
| Skip verify | 需 `verify_skip_reason` | **继承同样规则** | 契约一致性 |

### 6.3 关键约束 (从 mavis-team 借来, qianxun 强遵守)

- 1 个 task = 1 个可验证交付物 (mavis-team SKILL.md:217)
- verifier 必须独立 re-derive, 不许读 producer diff 盖章 (mavis-team SKILL.md:272)
- skip verify 必须有 reason (mavis-team SKILL.md:211, 268)
- 单任务 30 分钟硬上限 (mavis-team SKILL.md:205)
- 报告类任务要 traceable + 显式 contradiction (references/report.md)

---

## 7. 5 个已拍板决策 (本次草稿默认)

| # | 决策 | 选定 | 备选 |
|---|---|---|---|
| 1 | Plan 在 Col 2 怎么显示 | **特殊 tool-call 块** | 嵌入子 chat / 侧边抽屉 |
| 2 | SubSession 终止后能否继续 | **严格只读, 追问回主会话** | 可继续 (completed - follow-up) |
| 3 | Experience 跟 Minutes 写入时机 | **Minutes 增量, Experience 显式** | 全自动 / 模板触发 |
| 4 | Contract 谁生成 | **主 Agent 自动 + 模板辅助** | 用户确认 / 完全模板 |
| 5 | 未绑项目的 session | **未分类组, 用户可拖入项目** | 自动建议 / 强制绑 |

---

## 8. 实施路径

### Phase 1: 数据模型 + Col 1 重构 (1-2 天)

- [ ] `qianxun-core/src/types/plan.rs` — Project / Session / Plan / SubSession 类型
- [ ] `qianxun-core/src/db/migrations/0007_chat_first.sql` — 新 schema
- [ ] `qianxun-desktop/src/lib/types/plan.ts` — TypeScript 镜像
- [ ] `qianxun-desktop/src/lib/stores/project.svelte.ts` — 项目 store
- [ ] `qianxun-desktop/src/lib/stores/session.svelte.ts` — 会话 store (替换 mock)
- [ ] `Sidebar` + `SessionList` 重构: 改用真实 store, 新增 "未分类" 组

### Phase 2: Col 2 Chat 重构 (2-3 天)

- [ ] `ChatView.svelte` 拆 3 层: MessageList / MessageItem (含 PlanBlock) / InputArea
- [ ] `PlanBlock.svelte` — 新组件, 显示 plan 状态 + 任务列表
- [ ] `InputArea.svelte` — 支持 @项目 / /命令 / 附件 / 模型选择
- [ ] `NewTaskDialog.svelte` — 新建会话向导 (Modal)
- [ ] 接入 daemon: `POST /sessions`, `GET /sessions/:id/messages`, `POST /sessions/:id/messages`

### Phase 3: Col 3 检查器 + Plan 集成 (2-3 天)

- [ ] `Inspector.svelte` — 上下文检查器
- [ ] `ProjectInfo.svelte` — 项目信息 + 经验列表
- [ ] `PlanDetail.svelte` — Plan 详情 + 任务列表
- [ ] `SubSessionList.svelte` — 子会话列表
- [ ] `ChangedFiles.svelte` — 变更文件清单 (接 git status)
- [ ] Tab bar 组件 (主会话 ↔ 子会话切换)
- [ ] 接入 daemon: `POST /plans`, `GET /plans/:id`, `GET /sub_sessions/:id/messages`

### Phase 4: 端到端验证 (1 天)

- [ ] 真实启动 daemon, 跑通: 建会话 → 发消息 → 主 Agent 拉 Plan → Plan 跑完 → 沉淀经验
- [ ] 验证: Col 1/2/3 拖动, 移动端折叠, Plan 状态实时刷新
- [ ] 验证: 子会话点入, 回到主会话, 追问
- [ ] 跑 cargo test + playwright e2e (按 web 端 16+N 验证基础设施模板)

---

## 9. 待决事项 (后续讨论)

1. **纪要增量粒度** — 每轮 Assistant 消息 vs 每工具调用 vs 用户显式触发?
2. **经验去重** — 重复经验怎么合并? (LLM 判等? 全文 FTS5 阈值?)
3. **子会话消息流共享 messages 表** vs **独立 messages 表**? (前者省表, 后者清边界)
4. **未分类 session 数量上限** — 超过 N 条提示用户归类?
5. **VPS 模式下, Col 3 是不是还能展开**? (Web 端 + 桌面端共享设计, 移动端不同)

---

## 10. 附录: 预览 HTML

完整的 6 场景交互预览见:
```
E:\git\maxu\qianxun\qianxun-desktop\preview\index.html
```

包含: 主场景 (Plan 运行中) / Plan 完成 / 新建任务对话框 / 子会话详情 / 空状态 / 亮色模式 / 设计令牌参考.

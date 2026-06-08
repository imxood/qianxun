# 三子项目并行规划 — 共享契约与协调规则

> 创建: 2026-06-01 | 最后更新: 2026-06-07 | 状态: 跟 chat-first-redesign 对齐 (v2)
>
> 本文件是 `docs/30_子项目规划/_shared-contract.md`, 3 个规划 worker (Daemon / Tauri / VPS) 都必须读.
>
> **v2 更新摘要** (2026-06-07):
> - §3.1 路由: 9 个新路由 (Project / Plan / SubSession) + 路径从 `/v1/chat/session` 改名 `/v1/sessions`
> - §3.2 SSE: 12 事件, 跟 daemon-design v1.0 §3.5 对齐, 加 `plan_update` / `sub_session_event` / `experience_suggest`
> - §6 数据模型: 加 4 个新实体 (Plan / PlanTask / SubSession / ProjectExperience / SessionMinute), Project / Session 字段扩
> - 删除 kanban 相关字段 / 路由
>
> **设计基线**: `docs/chat-first-redesign.md` v1 + `docs/daemon-design.md` v1.0 + `docs/30_决策/ADR-0002_daemon_design_chat_first.md`

---

## §1 关键决策(已锁)

| 项 | 决策 |
|---|---|
| 前端技术栈 | **Svelte 5 (runes) + SvelteKit + Vite + Tailwind CSS + shadcn-svelte** |
| Tauri 版本 | **Tauri 2.0** (支持 iOS/Android, stable) |
| Web 后端 | Daemon HTTP API (本地 `127.0.0.1:23900`) |
| 远程后端 | VPS Server WebSocket (控制面) + 远程 Daemon 转发 |
| 数据库 | Daemon 端: SQLite (复用 `qianxun-memory`); VPS 端: SQLite (用户/设备/team) |
| 通信协议 | HTTP + SSE (本地), WebSocket (远程); SSE 事件 schema 在 §3 |
| **会话存储** | **chat-first 5 实体** (Project / Session / Plan / SubSession / Experience) + qianxun-memory (FTS5); 共存于同一 SQLite, 表名空间隔离 |
| **不做的事** | **kanban 看板 / git 集成 / 多用户 session 共享 / 多机 daemon 集群** (kanban 设计已废弃归档到 `docs/90_历史/2026-06-07-04-kanban-design-废弃.md`) |
| **设计基线** | `chat-first-redesign.md` v1 (用户面) + `daemon-design.md` v1.0 (后端) + `01b-daemon-web-console.md` (Web Console) |

---

## §2 三子项目依赖关系

```
                 ┌────────────────────────────────────┐
                 │   Tauri 桌面 (Track C)             │
                 │   chat-first 3 列 UI, 消费 HTTP/SSE│
                 └────────────────────────────────────┘
                           ↑                ↑
                           │ HTTP+SSE       │ HTTP+SSE
                           │                │
         ┌─────────────────┴───┐   ┌────────┴──────────┐
         │  Daemon (Track A)   │   │  VPS Server (B)   │
         │  唯一 Agent runtime │ ←→│  控制面 + 转发    │
         │  SQLite (5 实体)    │   │  WS Hub + JWT     │
         │  + qianxun-memory   │   │  Team / 设备       │
         └─────────────────────┘   └───────────────────┘
                           │                │
                           └──→ VPS WS ←────┘
                               (dev 机 Daemon 注册到 VPS)
```

**严格依赖**:
- VPS Server 强依赖 Daemon 真实化后的 WebSocket 端点 (Track A 必须先定义 ws_hub 协议)
- Tauri 弱依赖, 只要 API 契约定好就能并行做 UI (chat-first 已经定义好 Col 1/2/3 + 5 实体交互流)
- Web Console (01b) 依赖 Daemon 路由稳定, 跟 Tauri 共享同一套 HTTP/SSE

---

## §3 共享 API 契约草案 (v2)

### 3.1 REST endpoints (Daemon 暴露)

> 25 个 endpoint 完整列表见 `daemon-design.md` v1.0 §3.4. 这里只列**跨 Track 共同关心**的端点.

#### 3.1.1 会话 / Project / Plan (v2 新命名)

```
# === Project ===
GET    /v1/projects                            → { projects: [...] }
POST   /v1/projects                            → { project: {...} }
GET    /v1/projects/:id                       → { project: {...} }
PUT    /v1/projects/:id                       → { project: {...} }
DELETE /v1/projects/:id                       → { status: "deleted" }   (sessions.project_id 保留为 NULL)
GET    /v1/projects/:id/experience            → { items: [...] }        (走 qianxun-memory FTS5)
POST   /v1/projects/:id/experience            → { id, content }          (append-only)

# === Session (从 v0.2 /v1/chat/session 改名 /v1/sessions) ===
POST   /v1/sessions                            → { session_id }          (body: { project_id?, folder?, provider, model })
GET    /v1/sessions?project_id=&status=        → { sessions: [...] }     (列表, 支持 project_id / status filter)
GET    /v1/sessions/:id                       → { session: {...} }
PUT    /v1/sessions/:id                       → { session: {...} }     (改 title / status)
DELETE /v1/sessions/:id                       → { status: "deleted" }
POST   /v1/sessions/:id/prompt                → SSE stream (12 events, §3.2)
POST   /v1/sessions/:id/cancel                → { status: "cancelled" }
GET    /v1/sessions/:id/messages              → { messages: [...] }
POST   /v1/sessions/:id/messages              → { message_id }            (追加 user / system 消息)
GET    /v1/sessions/:id/minutes               → { minutes: [...] }       (纪要列表, 增量追加)

# === Plan (v2 新增, 跟 mavis-team 1:1 对齐) ===
POST   /v1/plans                               → { plan_id }              (body: PlanContract)
GET    /v1/plans?session_id=                  → { plans: [...] }
GET    /v1/plans/:id                          → { plan: { ...tasks, status } }
DELETE /v1/plans/:id                          → { status: "aborted" }   (cancel running plan)
GET    /v1/plans/:id/tasks                    → { tasks: [...] }

# === SubSession (v2 新增, 1 个 = 1 个 PlanTask) ===
GET    /v1/sub_sessions/:id                   → { sub_session: {...} }
GET    /v1/sub_sessions/:id/messages          → { messages: [...] }     (独立上下文, 终止后只读)
POST   /v1/sub_sessions/:id/messages          → { message_id }            (终止后 410 Gone)
```

#### 3.1.2 LLM / Tool / Memory / MCP / Skill / Config (v0.2 保留, 详见 daemon-design v1.0 §3.4.1)

```
# === System ===
GET    /v1/system/health                      → { status, version, uptime_seconds, started_at, build }   (公开, 无需 auth)
GET    /v1/system/status                      → { ... 5 实体统计 + providers + budget + memory ... }
POST   /v1/system/restart                     → { status: "restarting" }
POST   /v1/system/shutdown                    → { status: "shutting_down" }

# === LLM Provider ===
GET    /v1/llm/providers                       → { providers: [...] }
GET    /v1/llm/providers/:name                → { provider: {...} }      (key 不返)
POST   /v1/llm/providers                       → { status: "added" }       (key 写 keyring)
PUT    /v1/llm/providers/:name                 → { status: "updated" }    (含 key 替换)
DELETE /v1/llm/providers/:name                 → { status: "deleted" }
POST   /v1/llm/providers/:name/activate       → { status: "active" }
POST   /v1/llm/providers/:name/test           → { ok, latency_ms } | { ok: false, error }

# === Tool ===
GET    /v1/tools                               → { tools: [...] }
POST   /v1/tools/:name/invoke                  → { output } | { error }   (不走 LLM, 直调, Web Console 测试用)

# === Memory (走 qianxun-memory) ===
GET    /v1/memory/sessions                     → { sessions: [...] }
POST   /v1/memory/search                       → { results: [...] }
DELETE /v1/memory/observations/:id            → { status: "deleted" }
DELETE /v1/memory/sessions/:id                → { status: "deleted" }

# === MCP ===
GET    /v1/mcp/servers                         → { servers: [...] }
POST   /v1/mcp/servers                         → { status: "added" }
DELETE /v1/mcp/servers/:id                    → { status: "deleted" }
POST   /v1/mcp/servers/:id/test                → { ok, tools: [...] }

# === Skill ===
GET    /v1/skills                              → { skills: [...] }
POST   /v1/skills                              → { status: "reloaded", count: N }
POST   /v1/skills/:name/toggle                 → { status: "enabled" | "disabled" }

# === Config ===
GET    /v1/config                              → { config: {...} }        (敏感字段脱敏, e.g. API Key 不返)
PUT    /v1/config                              → { status, requires_reload }

# === Web UI (Stage 7a) ===
GET    /_ui/*                                  → SvelteKit SPA 静态文件
```

#### 3.1.3 鉴权 (v2 跟 v0.2 一致, 详细见 01b §6)

全部 `/v1/*` 端点要求 `Authorization: Bearer <jwt>`, role=`admin`. `/v1/system/health` 公开.
Stage 7a 简化: 启动时生成 token 打印到 stderr, 跟现有 Tauri 桌面端共用.
Stage 7b/8 加密码框 + bcrypt, 见 01b §6.1.

### 3.2 SSE 事件 schema (POST /v1/sessions/:id/prompt)

> 跟 daemon-design v1.0 §3.5 完全一致, 12 事件, 跨 Track 共同消费.
> 跟 v0.2 差异: 用 W3C SSE 标准 (event: + data: 两行), 不再用单个 JSON 字段.

**传输格式**:
```
event: <event_name>
data: <json>

event: <next_event>
data: <json>

```

**12 个事件** (按触发顺序):

```typescript
// 1. message_start — prompt 接收后第一帧
{ event: "message_start", data: { session_id: "sess_...", message_id: "msg_..." } }

// 2. text — LLM 输出文本块 (delta, 客户端追加)
{ event: "text", data: { text: "你好, 我是千寻" } }

// 3. thinking — LLM 思考块 (DeepSeek 特有)
{ event: "thinking", data: { text: "用户想加 JWT 登录, 我先拆任务..." } }

// 4. tool_call — LLM 请求调工具
{ event: "tool_call", data: { id: "toolu_abc", name: "read_file", arguments: { path: "src/main.rs" }, plan_ref: "plan_xyz" | null } }

// 5. tool_result — 工具执行完成
{ event: "tool_result", data: { id: "toolu_abc", name: "read_file", content: "fn main() { ... }", is_error: false, elapsed_ms: 234 } }

// 6. plan_update — Plan 状态变化 (v2 新增, 跟 chat-first 对齐)
{ event: "plan_update", data: { plan_id: "plan_xyz", status: "running" | "done" | "failed" | "aborted", task_id: "task_1" | null, progress: { done: 2, total: 3 } } }

// 7. sub_session_event — 子 Agent 事件转发 (v2 新增)
{ event: "sub_session_event", data: { sub_session_id: "sub_abc", event: <子事件原文, 同样 12 事件 schema> } }

// 8. experience_suggest — 主 Agent 建议沉淀经验 (v2 新增, 跟 chat-first 对齐)
{ event: "experience_suggest", data: { project_id: "proj_xyz", items: [{ content: "本项目用 jose 库做 JWT", source_session_id: "sess_...", source_plan_id: "plan_..." | null }] } }

// 9. status — 状态消息 (如 retry 中, 不阻塞)
{ event: "status", data: { message: "Provider 错误, 8s 后重试 (2/3)", level: "info" | "warn" } }

// 10. error — 发生错误 (4 种 code, 跟 v0.2 一致)
{ event: "error", data: { code: "rate_limit" | "auth" | "internal" | "cancelled", message: "..." } }

// 11. turn_finished — 一轮 LLM 调用结束 (中间可能多轮, 每轮结束都发)
{ event: "turn_finished", data: { reason: "end_turn" | "tool_use" | "max_tokens" | "stop", usage: { input: 123, output: 456, cost_usd: 0.0012 } } }

// 12. message_stop — 整个 prompt 处理结束 (末帧必发, 客户端据此关闭流)
{ event: "message_stop", data: {} }
```

**客户端断连**: server 端 SSE handler 检测 stream closed → `cancel_flag = true` → processing_loop 下次 chunk 检查后 return. 客户端不需要主动 close, 网络断开自动清理 (见 daemon-design v1.0 §3.5.3).

**4 种 error code 处理建议**:
| code | 客户端处理 |
|---|---|
| `rate_limit` | 展示 retry_after, 用户可等几秒重发 |
| `auth` | 引导用户检查 API Key 配置 |
| `internal` | 展示 "出错了", 提供重试按钮 |
| `cancelled` | 不展示, 用户主动取消的 |

### 3.3 WebSocket 消息格式 (VPS 端, v0.2 保留)

> 跟 v0.2 一致, 详细见 02-vps-server.md.

#### Daemon → VPS (设备注册)

```json
// connect
{ "type": "auth", "device_token": "dt_...", "machine_id": "..." }
{ "type": "auth_ok", "session_token": "...", "server_time": "..." }
{ "type": "auth_error", "code": "invalid_token" | "expired", "message": "..." }

// register
{ "type": "register", "device_id": "dev_...", "name": "...", "tags": ["workstation"] }
{ "type": "register_ok", "node_id": "node_..." }

// heartbeat
{ "type": "heartbeat", "ts": 1234567890 }
{ "type": "heartbeat_ack", "ts": 1234567890 }
```

#### App/VPS → Daemon (命令转发)

```json
// VPS 转发 prompt
{ "type": "prompt", "request_id": "req_...", "session_id": "sess_...", "messages": [...], "stream_to_vps": true }

// Daemon 流式事件 (转发的就是 §3.2 的 12 事件, 套一层 type=event)
{ "type": "event", "request_id": "req_...", "event": { "type": "text", "text": "..." } }
{ "type": "event_done", "request_id": "req_...", "usage": {...} }
{ "type": "event_error", "request_id": "req_...", "code": "...", "message": "..." }
```

**v2 调整**: VPS 转发的事件 payload `event.type` 跟 daemon 的 SSE `event:` 行**保持一致** (text / plan_update / experience_suggest 等), 不是旧的 message_start/content_block_start 那一套. 老 VPS 端代码需要适配.

---

## §4 三个 Track 的输入约束 (v2)

### Track A — Daemon 规划
- **主输出**: `docs/daemon-design.md` v1.0 (2026-06-07, 已跟 chat-first 对齐)
- **Web Console 子输出**: `docs/30_子项目规划/01b-daemon-web-console.md` (Stage 7a/b/c 详细设计)
- **历史归档**:
  - `docs/daemon-design.md` v0.2 → `docs/90_历史/2026-06-07-daemon-design-v0.2-被覆盖.md`
  - `docs/30_子项目规划/01-daemon.md` v1.1 (Track A 详细设计) → `docs/90_历史/2026-06-07-01-daemon-v1.1-被覆盖.md`
- **决策记录**: `docs/30_决策/ADR-0002_daemon_design_chat_first.md`
- **范围**: Rust daemon, 5 实体 (Project / Session / Plan / SubSession / Experience), 25 个 endpoint, 12 SSE 事件

### Track B — VPS Server 规划
- **主输出**: `docs/30_子项目规划/02-vps-server.md` (沿用 v0.2, 加 chat-first 适配)
- **范围**: WS Hub 转发, Team / 设备管理, 跨设备 session 同步 (轻量, 千寻是个人项目不做实时协作)
- **必读**: 本文件 §3 (路由 + SSE schema), daemon-design v1.0 §3 (Daemon API)

### Track C — Tauri 桌面版规划
- **主输出**: `docs/30_子项目规划/03-tauri-desktop.md` (沿用 v0.2, 加 chat-first 适配)
- **设计基线**: `docs/chat-first-redesign.md` v1 (3 列 UI + 5 实体交互流)
- **预览**: `qianxun-desktop/preview/index.html` (6 场景, 86KB)
- **范围**: Tauri 2.0 + Svelte 5, chat-first Col 1/2/3 实现, IPC 桥 daemon

---

## §5 跨 Track 一致性约束 (v2)

1. **REST endpoint 命名 (§3.1)**: 三个 track 都必须遵守, `/v1/chat/session/*` **已废弃**, 统一用 `/v1/sessions/*` + 9 个新端点
2. **SSE 事件 schema (§3.2)**: 三个 track 都必须用 12 事件 + W3C SSE 标准格式, Track A 实施, Track B 转发, Track C 消费
3. **5 实体数据模型 (§6)**: Track A 持久化, Track B 转发时只读, Track C 消费; ProjectExperience 走 qianxun-memory 不在 daemon SQLite
4. **Plan / SubSession 强约束** (跟 mavis-team 1:1):
   - 1 个 task = 1 个可验证交付物
   - verifier 独立 re-derive, 不读 producer 产物
   - skip verify 必须有 user-written reason (结构层硬约束)
   - 单任务 30 分钟硬上限
5. **任何 track 调整 §3 契约**: 必须在规划文件里明确标注 "对 §3.X 的扩展/修改", 由 Mavis 在一致性 review 中确认
6. **新会话入口**: 三个端的 "+ 新建任务" 都是**小链接 + 跳到空白 Chat**, 不弹窗, 不居中页 (跟 chat-first §5.1 决策一致)

---

## §6 跨 Track 数据模型 (v2, 5 实体 + 3 支撑)

> Track A 持久化, Track B 转发时只读, Track C 消费. 详细 schema 见 `daemon-design.md` v1.0 §4.

```rust
// === 1. Project (顶层容器) ===

struct Project {
    id: String,                  // "proj_xxx"
    name: String,
    folder: Option<String>,      // "E:/git/maxu/qianxun/qianxun-desktop", None = "Chat" 分类
    provider: String,            // "deepseek"
    default_model: String,       // "deepseek-v4-flash"
    description: Option<String>,
    team_id: Option<String>,     // 关联到 team (VPS 端, 千寻个人项目可忽略)
    owner_id: String,            // user_id
    created_at: DateTime<Utc>,
    last_active_at: DateTime<Utc>,
}

// === 2. Session (主会话) ===

struct Session {
    id: String,                  // "sess_20260607_220000_123456"
    project_id: Option<String>,  // FK → projects; None = "Chat" 分类
    title: String,               // 首条消息自动生成
    provider: String,
    model: String,
    status: SessionStatus,       // Active | Idle | Archived (v0.2 的 "Idle | Busy | Cancelled | Paused" 已改)
    message_count: u32,
    owner_id: String,            // user_id (VPS 端 scope)
    created_at: DateTime<Utc>,
    last_active_at: DateTime<Utc>,
}

enum SessionStatus { Active, Idle, Archived }

// === 3. Plan (主会话内发起的子任务) ===

struct Plan {
    id: String,                  // "plan_xxx"
    session_id: String,          // 归属的主会话
    contract: PlanContract,      // mavis-team task schema 子集
    status: PlanStatus,          // Pending | Running | Done | Failed | Aborted
    started_at: Option<DateTime<Utc>>,
    ended_at: Option<DateTime<Utc>>,
    result: Option<PlanResult>,  // done 时填充
    attachments: Vec<Attachment>,
}

struct PlanContract {
    name: String,
    description: String,
    tasks: Vec<PlanTaskSpec>,
    timeout_ms: u32,             // 默认 1800000 (30 min, 跟 mavis-team 一致)
}

struct PlanTaskSpec {
    id: String,
    title: String,
    prompt: String,              // 给子 Agent 的 spec
    assigned_to: String,         // "coder" / "tester" / "researcher"
    verified_by: Option<String>, // "verifier" / "code-reviewer" / "tester" / null
    verify_prompt: Option<String>,
    depends_on: Vec<String>,
    timeout_ms: u32,
    output: Option<OutputSpec>,  // 期望的产物形状
}

enum PlanStatus { Pending, Running, Done, Failed, Aborted }

// === 4. SubSession (子会话, 1 个 = 1 个 PlanTask) ===

struct SubSession {
    id: String,                  // "sub_xxx"
    plan_id: String,
    plan_task_id: String,        // 1 个 SubSession = 1 个 PlanTask
    parent_session_id: String,   // 归属主会话
    role: String,                // 跟 PlanTaskSpec.assigned_to 一致
    status: SubSessionStatus,    // Active | Done | Failed | Aborted | ReadOnly
    messages: Vec<Message>,      // 独立上下文, 持久化
    output: Option<serde_json::Value>,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
}

enum SubSessionStatus { Active, Done, Failed, Aborted, ReadOnly }

// === 5. ProjectExperience (项目经验, 走 qianxun-memory) ===

struct ProjectExperience {
    id: String,                  // qianxun-memory 自增
    project_id: String,          // FK → projects
    content: String,             // 经验内容
    source_session_id: Option<String>,
    source_plan_id: Option<String>,
    tags: Vec<String>,
    created_at: DateTime<Utc>,
    // 物理存储: qianxun-memory 的 memory_kind = "project_experience"
    // 索引: FTS5, 跟 memory 复用
}

// === 支撑: SessionMinute (会话纪要, 增量追加) ===

struct SessionMinute {
    id: String,
    session_id: String,
    content: String,             // 50-100 字摘要
    message_count_at_minute: u32,
    created_at: DateTime<Utc>,
}

// === 支撑: Message (主会话 跟 子会话 共享, 靠 session_id / sub_session_id 二选一) ===

struct Message {
    id: String,
    session_id: Option<String>,     // 主会话消息
    sub_session_id: Option<String>, // 子会话消息
    role: String,                   // user | assistant | system
    content: String,
    tool_calls: Option<Vec<ToolCall>>,
    plan_ref: Option<String>,       // assistant 消息引用的 plan
    created_at: DateTime<Utc>,
}

// === 支撑: Team / TeamMember / ProjectAssignment (VPS 端 scope, 千寻个人项目可忽略) ===

struct Team {
    id: String,
    name: String,
    created_at: DateTime<Utc>,
    members: Vec<TeamMember>,
}

struct TeamMember {
    user_id: String,
    display_name: String,
    email: Option<String>,
    avatar_url: Option<String>,
    role: String,                // owner | admin | developer | viewer
    joined_at: DateTime<Utc>,
}

struct ProjectAssignment {
    team_id: String,
    project_id: String,
    member_ids: Vec<String>,
    assigned_at: DateTime<Utc>,
}
```

### 6.1 实体关系图 (v2)

```
Project (proj_xxx)
  ├─ has many Session (project_id FK)
  ├─ has many ProjectExperience (project_id FK, 走 qianxun-memory)
  └─ has many SubSession (间接通过 Plan)

Session (sess_xxx)
  ├─ has many Plan (session_id FK)
  ├─ has many Message (session_id FK)
  ├─ has many SessionMinute (session_id FK)
  └─ references many SubSession (parent_session_id FK)

Plan (plan_xxx)
  ├─ has many PlanTaskSpec (1:1 with SubSession, in JSON contract)
  └─ has many SubSession (plan_id FK)

SubSession (sub_xxx)
  ├─ references 1 PlanTaskSpec (plan_task_id FK)
  ├─ has many Message (sub_session_id FK)
  └─ status=Done/Failed/Aborted → 终止, 后续只读

ProjectExperience (走 qianxun-memory, 不在 daemon SQLite)
  └─ belongs to Project
```

### 6.2 跟 v0.2 的差异

| 实体 | v0.2 字段 | v2 字段 | 备注 |
|---|---|---|---|
| Project | `path: String` | `folder: Option<String>` | 改名为 folder, 可选 |
| Project | — | `provider`, `default_model` | 新 |
| Project | `team_id: Option<String>` | 同 | 不变 (VPS scope) |
| Session | `status: String` | `status: SessionStatus` (Active/Idle/Archived) | enum 化 |
| Session | — | `provider`, `message_count` | 新 |
| Session | `project_id: String` | `project_id: Option<String>` | 可空, None = Chat |
| (新) | — | `Plan` + `PlanTaskSpec` + `PlanContract` | 跟 mavis-team 1:1 |
| (新) | — | `SubSession` | 独立上下文, 持久化 |
| (新) | — | `ProjectExperience` | 跨会话沉淀, 走 qianxun-memory |
| (新) | — | `SessionMinute` | 会话内增量 |
| (新) | — | `Message.tool_calls` / `Message.plan_ref` | 改用追加 messages 表 |

**删除**:
- v0.2 的 `daemon_sessions_v2_*` 表 (chat-first 整体替换)
- 任何 kanban 相关字段 / 表 (已废弃)

---

## §7 协调者工作流 (Mavis)

1. 启动 3 个 worker (general agent) 并行
2. 等 worker 各自输出
3. 收集 3 个 markdown 文件, 做交叉一致性 review:
   - SSE 事件 schema 字段是否一致?
   - REST endpoint 路径 / 参数是否一致?
   - 5 实体字段是否一致 (Project / Session / Plan / SubSession / Experience)?
   - 任何 kanban 残留? 任何 `/v1/chat/session` 老路径?
4. 如有不一致, 派第 4 个 worker 做对齐 (or 调整 prompt 重派)
5. 最终 3 个文件 + 一致性检查报告, 交付给用户

### 7.1 v2 协调重点

- **路径同步**: 所有 3 个 track 必须用 `/v1/sessions/*` (不用 `/v1/chat/session/*`), `/v1/projects`, `/v1/plans`, `/v1/sub_sessions`
- **SSE 同步**: 12 事件用 W3C SSE `event:` + `data:` 格式, 不用单个 JSON `type` 字段
- **Plan 字段同步**: PlanTaskSpec 跟 mavis-team task schema 1:1, 不要发明新字段
- **SubSession 只读**: 终止后 POST /v1/sub_sessions/:id/messages 返 410 Gone, 三个 track 都遵守

---

## §8 历史 & 变更记录

| 日期 | 版本 | 变更 | 关联文件 |
|---|---|---|---|
| 2026-06-01 | v1 | 初始 3-track 协调契约 | — |
| 2026-06-02 | v1.1 | 加 Web Console 路由 (Stage 7a/b/c) | 01b-daemon-web-console.md |
| **2026-06-07** | **v2** | **跟 chat-first 对齐: 9 个新路由, 12 SSE 事件, 5 实体数据模型, 删除 kanban** | **chat-first-redesign.md v1, daemon-design.md v1.0, ADR-0002** |

**归档文件** (在 `docs/90_历史/`):
- `2026-06-07-04-kanban-design-废弃.md` (132 KB) — 整段已并入 chat-first + daemon-design v1.0
- `2026-06-07-05-mvp-0-checklist-废弃.md` (5 KB) — 修复缺口 7 任务清单, 2026-06-03 已完成
- `2026-06-07-daemon-design-v0.2-被覆盖.md` (31 KB) — 旧 daemon 设计骨架
- `2026-06-07-01-daemon-v1.1-被覆盖.md` (123 KB) — Track A 详细设计, 内容已并入 daemon-design v1.0

---

**下一步**: 三个 Track (Daemon 已完成, VPS / Tauri 还在) 按本契约落地. `_shared-contract.md` v2 是同步基线, 任何调整按 §5 规则走.

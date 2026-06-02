# 三子项目并行规划 — 共享契约与协调规则

> 创建: 2026-06-01 | 状态: 协调中 | Mavis 协调, 3 个 worker 并行
>
> 本文件是 `docs/30_子项目规划/_shared-contract.md`, 3 个规划 worker 都必须读.

## §1 关键决策(已锁)

| 项 | 决策 |
|---|---|
| 前端技术栈 | **Svelte 5 (runes) + SvelteKit + Vite + Tailwind CSS + shadcn-svelte** |
| Tauri 版本 | **Tauri 2.0** (支持 iOS/Android, stable) |
| Web 后端 | Daemon HTTP API (本地 `127.0.0.1:23900`) |
| 远程后端 | VPS Server WebSocket (控制面) + 远程 Daemon 转发 |
| 数据库 | Daemon 端: SQLite (复用 `qianxun-memory`); VPS 端: SQLite (用户/设备/team) |
| 通信协议 | HTTP + SSE (本地), WebSocket (远程); SSE 事件 schema 在 §3 |

## §2 三子项目依赖关系

```
                ┌────────────────────────────────────┐
                │   Tauri 桌面 (Track C)             │
                │   纯前端, 消费 HTTP/SSE + WS       │
                └────────────────────────────────────┘
                          ↑                ↑
                          │ HTTP+SSE       │ HTTP+SSE
                          │                │
        ┌─────────────────┴───┐   ┌────────┴──────────┐
        │  Daemon (Track A)   │   │  VPS Server (B)   │
        │  唯一 Agent runtime │ ←→│  控制面 + 转发    │
        │  SQLite + Memory    │   │  WS Hub + JWT     │
        └─────────────────────┘   └───────────────────┘
                          │                │
                          └──→ VPS WS ←────┘
                              (dev 机 Daemon 注册到 VPS)
```

**严格依赖**:
- VPS Server 强依赖 Daemon 真实化后的 WebSocket 端点(Track A 必须先定义 ws_hub 协议)
- Tauri 弱依赖,只要 API 契约定好就能并行做 UI

## §3 共享 API 契约草案

### 3.1 REST endpoints (Daemon 暴露)

```
GET    /v1/system/health
GET    /v1/system/status
POST   /v1/chat/session                          → { session_id }
GET    /v1/chat/session/:id                      → { session_id, status, model, created_at, ... }
DELETE /v1/chat/session/:id                      → { status: "deleted" }
POST   /v1/chat/session/:id/prompt               → SSE stream
POST   /v1/chat/session/:id/cancel               → { status: "cancelled" }
GET    /v1/chat/sessions                         → [session_id, ...]  // 列表

GET    /v1/tools                                  → { tools: [...] }
GET    /v1/memory/sessions                        → { sessions: [...] }
POST   /v1/memory/search                          → { results: [...] }
GET    /v1/skills                                 → { skills: [...] }
GET    /v1/mcp/servers                            → { servers: [...] }
POST   /v1/mcp/servers                            → { status: "added" }

GET    /v1/projects                               → { projects: [...] }
GET    /v1/teams                                  → { teams: [...] }
```

### 3.1.1 Web Admin Console 路径 (Stage 7 新增, 2026-06-02)

详细见 `01b-daemon-web-console.md`. 摘要:

```
GET    /_ui/*                                     → Svelte 5 SPA 静态文件 (含 fallback to index.html)

GET    /v1/llm/providers                          → { providers: [...] }
GET    /v1/llm/providers/{id}                     → { provider: {...} }  (key 不返)
POST   /v1/llm/providers                          → { status: "added" }    (key 写 keyring)
PUT    /v1/llm/providers/{id}                     → { status: "updated" } (含 key 替换)
DELETE /v1/llm/providers/{id}                     → { status: "deleted" }
POST   /v1/llm/providers/{id}/activate            → { status: "active" }
POST   /v1/llm/providers/{id}/test                → { ok: true, latency_ms: 234 } | { ok: false, error: "..." }

POST   /v1/skills                                 → { status: "reloaded", count: N }
POST   /v1/skills/{name}/toggle                   → { status: "enabled" | "disabled" }

DELETE /v1/mcp/servers/{id}                       → { status: "deleted" }
POST   /v1/mcp/servers/{id}/test                  → { ok: true, tools: [...] } | { ok: false, error: "..." }

POST   /v1/tools/{name}/invoke                    → { output: ... } | { error: "..." }   (不走 LLM, 直调)

GET    /v1/chat/sessions                          → { sessions: [...] }                  (Stage 7b)
POST   /v1/chat/session/{id}/cancel               → { status: "cancelled" }              (Stage 7b)
POST   /v1/chat/session/{id}/pause                → { status: "paused" }                 (Stage 7b, 接口预留)

PUT    /v1/config                                 → { status: "updated", requires_reload: bool }  (Stage 7b)

DELETE /v1/memory/observations/{id}               → { status: "deleted" }                (Stage 7b)
DELETE /v1/memory/sessions/{id}                   → { status: "deleted" }                (Stage 7b)

GET    /v1/system/metrics                         → { cpu, mem_mb, conns, uptime_s, ... } (Stage 7b)
GET    /v1/system/logs?lines=N                    → { lines: [...] }                     (Stage 7b)
```

**鉴权**: 全部要求 `Authorization: Bearer <jwt>`, role=`admin` (Stage 7b 起). 
Stage 7a 简化: 启动时生成 token 打印到 stderr, 跟现有 Tauri 桌面端共用.

### 3.2 SSE 事件 schema (POST /v1/chat/session/:id/prompt)

所有事件 JSON 格式: `data: <json>\n\n`, 可选 `event: <name>\n` 标识类型.

```json
// 1. message_start
{ "type": "message_start", "session_id": "sess_...", "model": "...", "max_tokens": 16384 }

// 2. content_block_start
{ "type": "content_block_start", "index": 0, "block_type": "text" | "tool_use" | "thinking" }

// 3. text_delta
{ "type": "text_delta", "index": 0, "text": "..." }

// 4. thinking_delta
{ "type": "thinking_delta", "index": 1, "text": "..." }

// 5. tool_use_delta
{ "type": "tool_use_delta", "index": 2, "id": "toolu_...", "name": "read_file", "arguments_json": "..." }

// 6. tool_use_complete
{ "type": "tool_use_complete", "index": 2, "id": "toolu_...", "name": "read_file", "arguments": { ... } }

// 7. tool_result
{ "type": "tool_result", "tool_use_id": "toolu_...", "content": "...", "is_error": false, "elapsed_ms": 234 }

// 8. content_block_stop
{ "type": "content_block_stop", "index": 0 }

// 9. usage
{ "type": "usage", "input_tokens": 1234, "output_tokens": 567, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0 }

// 10. message_delta
{ "type": "message_delta", "stop_reason": "end_turn" | "max_tokens" | "tool_use" }

// 11. message_stop
{ "type": "message_stop" }

// 12. error
{ "type": "error", "code": "rate_limit" | "auth" | "api_error" | "internal", "message": "..." }
```

### 3.3 WebSocket 消息格式 (VPS 端)

#### Daemon → VPS (设备注册)

```json
// connect: auth 阶段
{ "type": "auth", "device_token": "dt_...", "machine_id": "..." }
{ "type": "auth_ok", "session_token": "...", "server_time": "..." }
{ "type": "auth_error", "code": "invalid_token" | "expired", "message": "..." }

// register
{ "type": "register", "device_id": "dev_...", "name": "...", "tags": ["workstation"] }
{ "type": "register_ok", "node_id": "node_..." }
{ "type": "register_error", "code": "..." }

// heartbeat
{ "type": "heartbeat", "ts": 1234567890 }
{ "type": "heartbeat_ack", "ts": 1234567890 }
```

#### App/VPS → Daemon (命令转发)

```json
// VPS 转发 prompt
{ "type": "prompt", "request_id": "req_...", "session_id": "sess_...", "messages": [...], "stream_to_vps": true }

// Daemon 流式事件
{ "type": "event", "request_id": "req_...", "event": { "type": "text_delta", "text": "..." } }
{ "type": "event_done", "request_id": "req_...", "usage": {...} }
{ "type": "event_error", "request_id": "req_...", "code": "...", "message": "..." }
```

## §4 三个 Track 的输入约束

### Track A — Daemon 规划
- **输出文件**: `docs/30_子项目规划/01-daemon.md`
- **基于**: 现有 `docs/daemon-design.md` v0.2 骨架
- **必须细化**: 详见 task prompt

### Track B — VPS Server 规划
- **输出文件**: `docs/30_子项目规划/02-vps-server.md`
- **基于**: 现有 `docs/vps-server-design.md` v0.2 骨架
- **必须细化**: 详见 task prompt

### Track C — Tauri 桌面版规划
- **输出文件**: `docs/30_子项目规划/03-tauri-desktop.md` (新建)
- **新建项目**: 独立 repo 或 monorepo 子目录(规划中决定)
- **必须细化**: 详见 task prompt

## §5 跨 Track 一致性约束(协调规则)

1. **SSE 事件 schema (§3.2)**: 三个 track 都必须遵守; Track A 实施, Track B 转发, Track C 消费
2. **REST endpoints (§3.1)**: 三个 track 都要消费, 命名/字段必须一致
3. **Team 模型**: Track B 是服务端权威, Track C 是客户端展示
4. **任何 track 调整 §3 的契约**: 必须在规划文件里明确标注"对 §3.X 的扩展/修改", 由 Mavis 在一致性 review 中确认

## §6 跨 Track 数据模型(Track C 主要产出, Track B 服务端映射)

```rust
// 由 Track C 在规划中详细定义, Track B 服务端持久化, Track A 端只读

struct Project {
    id: String,                  // "proj_xxx"
    name: String,
    path: String,                // 工作目录
    description: Option<String>,
    created_at: DateTime<Utc>,
    team_id: Option<String>,     // 关联到 team
    owner_id: String,            // user_id
}

struct Session {
    id: String,                  // "sess_xxx"
    project_id: String,
    title: String,
    model: String,
    status: String,              // active | idle | archived
    created_at: DateTime<Utc>,
    last_active_at: DateTime<Utc>,
    message_count: u32,
    owner_id: String,
}

struct Team {
    id: String,                  // "team_xxx"
    name: String,
    created_at: DateTime<Utc>,
    members: Vec<TeamMember>,    // 初始 inline, 后续规范化
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
    member_ids: Vec<String>,     // 子集 of team.members, 被分配到此项目的成员
    assigned_at: DateTime<Utc>,
}
```

## §7 协调者工作流 (Mavis)

1. 启动 3 个 worker (general agent) 并行
2. 等 worker 各自输出
3. 收集 3 个 markdown 文件, 做交叉一致性 review:
   - SSE 事件 schema 字段是否一致?
   - REST endpoint 路径/参数是否一致?
   - Team/Project/Session 字段是否一致?
4. 如有不一致, 派第 4 个 worker 做对齐 (or 调整 prompt 重派)
5. 最终 3 个文件 + 一致性检查报告, 交付给用户

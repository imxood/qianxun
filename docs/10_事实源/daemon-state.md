---
状态: 生效
适用范围: qianxun/src/daemon/
最后更新: 2026-06-04
---

# Daemon 子系统状态

## 一句话摘要
HTTP 骨架就绪（AgentLoopHost + axum 路由），但未接入真实 AgentLoop/Memory/Skills/MCP runtime。

## 源文件清单
-  — run() 入口（axum + TcpListener + graceful_shutdown）
-  — 11 条路由定义（health/status/chat/session/tools/config/memory/skills/mcp）
-  — AgentLoopHost（session 创建/存在检查/删除）
-  — AgentLoop + processing_loop（Daemon 的目标 runtime）

## 当前状态

| 子模块 | 状态 | 说明 |
|--------|------|------|
| HTTP 服务器 | ✅ | axum + graceful_shutdown |
| 路由定义 | ✅ | 11 条路由全部注册 |
| AgentLoopHost | ✅ | session 容器（create/exists/delete） |
| session CRUD | ✅ | POST/GET/DELETE /v1/chat/session |
| prompt SSE | 🔧 | 端点存在，未调用 processing_loop |
| memory/skills/mcp 端点 | 🔧 | 路由存在，返回存根数据 |
| session store | 🔧 | 未持久化 |
| Config 端点 | 🔧 | 返回空配置 |

## 已知缺口
- /v1/chat/session/:id/prompt 未接入 processing_loop
- memory/skills/mcp 路由返回存根
- conversation 无持久化
- 无认证（当前仅限 localhost）

## Kanban 子系统 (MVP-2 + MVP-3, 2026-06-04)

按 v6 §14.1 MVP-2/MVP-3 落地, 跟 daemon.db 共享 SQLite (8 张 kanban_* 表 + 2 ALTER).

| 子模块 | 状态 | 文件 | 说明 |
|--------|------|------|------|
| KanbanDb | ✅ | `qianxun-core/src/kanban/db.rs` | 10 个核心 CRUD 方法, 全部 spawn_blocking 异步化 |
| 状态机 | ✅ | `qianxun-core/src/kanban/state_machine.rs` | 7 状态 + check_transition + recompute_parent |
| Dispatcher | ✅ | `qianxun-core/src/kanban/dispatcher.rs` | dispatch_once + run_forever 骨架 (run_forever 2s 周期) |
| Team/Profile/Role | ✅ | `qianxun-core/src/kanban/team.rs` | 4 默认 role (techlead/coder/verifier/researcher) |
| 12 个 kanban_* 工具 | 🟡 4/12 | `qianxun-core/src/tools/kanban.rs` | create/complete/heartbeat/write_blackboard 落地, 余 8 留 v2 |
| kanban_host | ✅ | `qianxun/src/daemon/kanban_host.rs` | KanbanDb + Dispatcher + 5 SSE 事件 broadcast |
| team_registry | ✅ | `qianxun/src/daemon/team_registry.rs` | daemon 侧包装, 4 默认 profile |
| 12 个 HTTP 端点 | 🟡 12/23 | `qianxun/src/daemon/router.rs` | boards (3) + projects (2) + tasks (3) + events (1) + profiles (1) + roles (1) + dispatch (1), 余 11 留 v2 |
| 17 个 SSE 事件 | ✅ | `qianxun/src/daemon/sse.rs` | 12 原有 + 5 Kanban 新 (Assigned/Progress/Completed/Spawned/BlackboardUpdate) |

启动集成: `daemon/mod.rs::run()` 构造 KanbanHost (跟 store 共享 daemon.db), 启动 dispatcher 后台 task.

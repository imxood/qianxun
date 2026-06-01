---
状态: 生效
适用范围: qianxun/src/daemon/
最后更新: 2026-06-01
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

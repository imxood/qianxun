---
状态: 生效
适用范围: qianxun-core/src/mcp/ + qianxun/src/tools/
最后更新: 2026-06-01
---

# MCP 子系统状态

## 一句话摘要
MCP Client 完整实现（连接/工具列表/调用/关闭），ServerManager 含崩溃保护，工具注册到 ToolRegistry。

## 源文件清单
-  — MCP 协议客户端（initialize/tools/list/call/shutdown）
-  — 进程生命周期管理、崩溃保护、工具注册
-  — AgentTool trait 适配器
-  — stdio 行帧通信
-  — ToolRegistry，含 connect_workspace_mcp 接线

## 当前状态

| 子模块 | 状态 | 说明 |
|--------|------|------|
| MCP Client 连接 | ✅ | JSON-RPC 握手、能力协商 |
| tools/list | ✅ | 获取工具列表 |
| tools/call | ✅ | 120s 超时，错误处理 |
| shutdown | ✅ | 优雅关闭 |
| ServerManager | ✅ | start/stop/restart，崩溃循环保护（5min 内 3 次） |
| 工具注册到 ToolRegistry | ✅ | 通过 register_tools 注册 McpToolEntry |
| 工具名格式 | ✅ | server/tool 格式 |
| ACL/审批 | 🔧 | 无权限过滤 |
| Mock 测试 | 🔧 | 无集成测试 |

## 已知缺口
- 无权限审批机制
- 无 mock server 集成测试
- 传输层不支持 HTTP/SSE（仅 stdio）

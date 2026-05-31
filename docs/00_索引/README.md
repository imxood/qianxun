# 索引目录

> 最后更新: 2026-05-31

## 任务路由

| 任务 | 入口 |
|---|---|
| 了解系统架构 | `docs/architecture.md` |
| 了解记忆子系统设计 | `docs/memory-design.md` |
| 了解数据库选型决策 | `docs/30_决策/ADR-0001_数据库选型.md` |
| 了解 CLI/ACP/CLI 入口 | `CLAUDE.md` — 模块结构 |
| 查看项目规则 | `CLAUDE.md` |
| 查看代码结构 | `CLAUDE.md` — 模块结构 |
| 查看当前工作项 | `docs/20_工作项/` |
| 了解构建顺序 | `docs/architecture.md` §9.4 |

## 仓库地图

```
qianxun-core/src/
├── types.rs                  核心类型 (LlmError, TokenUsage, AgentConfig)
├── config.rs                 全局配置 (JSON 带 // 注释、优先级链)
├── output.rs                 OutputSink trait (输出抽象)
├── event.rs                  AgentEvent 事件定义
├── workspace.rs              工作区检测 (detect_workspace, build_workspace_context)
├── agent/
│   ├── message.rs            Message enum, ContentBlock
│   ├── conversation.rs       Conversation (消息历史 + token 预算)
│   ├── engine.rs             AgentState, AgentLoop 状态机, processing_loop
│   └── system_prompt.rs      系统提示词组装
├── provider/
│   ├── mod.rs                LlmProvider trait
│   ├── types.rs              CompletionRequest, LlmStreamEvent
│   └── deepseek.rs           DeepSeekProvider (Anthropic API, SSE 流式)
├── tools/
│   ├── mod.rs                ToolRegistry, AgentTool trait
│   └── builtin.rs            5 个内置工具
├── context/
│   ├── mod.rs                ContextProvider trait
│   └── memory.rs             MemoryManager (骨架)
├── skills/mod.rs             SkillManager (骨架)
└── mcp/
    ├── mod.rs                MCP 模块
    └── client.rs             McpClient (骨架)

qianxun/src/               # 单二进制 (qx)
├── main.rs                   CLI 入口 (clap, 三模式路由)
├── buf_writer.rs             日志缓冲写入
├── cli/                       CLI REPL 模块
│   ├── mod.rs
│   ├── cli.rs                 REPL 循环 (斜杠命令)
│   ├── config.rs              配置路径 + 默认配置模板生成
│   ├── output.rs              CliOutputSink (ANSI 终端输出)
│   └── run.rs                 启动流程 (run_repl)
└── acp/                       ACP 协议模块
    ├── mod.rs
    ├── types.rs               JSON-RPC 2.0 信封 + ACP 协议类型
    ├── transport.rs           stdio 行帧读写 + 双向请求路由
    ├── session.rs             SessionManager (会话管理)
    ├── output.rs              AcpOutputSink
    ├── prompt.rs              session/prompt 桥接
    ├── forwarding_tools.rs    转发工具注册
    ├── handler.rs             请求路由
    └── server.rs              ACP 主循环
```

## 文档索引

| 文档 | 类型 | 位置 |
|---|---|---|
| 架构设计 | 设计文档 | `docs/architecture.md` |
| Agent 模式 | 设计文档 | `docs/agent-pattern-design.md` |
| 记忆子系统 | 设计文档 | `docs/memory-design.md` |
| MCP Client | 设计文档 | `docs/mcp-design.md` |
| Skill 系统 | 设计文档 | `docs/skills-design.md` |
| Daemon 模式 | 设计文档 | `docs/daemon-design.md` |
| VPS Server | 设计文档 | `docs/vps-server-design.md` |
| 数据库选型 | 决策记录 | `docs/30_决策/ADR-0001_数据库选型.md` |
| Phase 3 工作项 | 工作项 | `docs/20_工作项/2026-05-31_Phase3_记忆子系统设计修订/` |
| 模块设计工作项 | 工作项 | `docs/20_工作项/2026-05-31_模块设计文档起草/` |
| 项目规则 | 规则 | `CLAUDE.md` |

## 术语表

| 术语 | 说明 |
|---|---|
| Daemon | 本地守护进程，持有 AgentLoop + MemoryCore + API Key |
| ACP | Agent Communication Protocol，与 Zed 等编辑器通信的 stdio JSON-RPC 2.0 协议 |
| AgentLoop | 代理引擎状态机 (Idle → WaitingLlm → ToolExecuting → ...) |
| OutputSink | 输出抽象 trait，引擎通过它输出文本/tool_call/事件，不感知具体输出目标 |
| ToolRegistry | 工具注册中心，统一调度 builtin/skill/MCP 三层工具 |
| TokenBudget | 输入输出 token 预算，用于 enforce_budget 裁剪 |
| Workspace | 工作区信息 (根路径、项目类型、CLAUDE.md 内容) |
| MCP | Model Context Protocol，标准化的外部工具协议 |
| FTS5 | SQLite 内置全文搜索扩展（替代自建 BM25） |
| HybridSearch | FTS5 + 向量索引混合检索，RRF 融合排序 |
| Consolidation | 将 Observation 聚类生成持久 Memory 的管线 |

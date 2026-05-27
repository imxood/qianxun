# 索引目录

> 最后更新: 2026-05-27

## 任务路由

| 任务 | 入口 |
|---|---|
| 了解系统架构和模块划分 | `docs/10_事实源/架构设计.md` |
| 了解 ACP 协议实现 | `docs/10_事实源/架构设计.md` 第 9 节 |
| 了解工作区支持 | `docs/10_事实源/架构设计.md` 第 8 节 |
| 查看项目规则 | `CLAUDE.md` |
| 查看代码结构 | `CLAUDE.md` — 模块结构 |
| 查看当前工作项 | `docs/20_工作项/` |

## 仓库地图

```
qianxun-core/src/
├── types.rs                  核心类型 (LlmError, TokenUsage, AgentConfig)
├── config.rs                 全局配置 (JSON5 解析、优先级链)
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

qianxun-acp/src/
├── types.rs                  JSON-RPC 2.0 信封 + ACP 协议类型
├── transport.rs              stdio 行帧读写 + 双向请求路由
├── session.rs                SessionManager (会话管理)
├── acp_output.rs             AcpOutputSink (OutputSink → ACP 通知)
├── prompt.rs                 session/prompt 桥接占位
├── handler.rs                请求路由 + ForwardingToolRegistry
└── server.rs                 ACP 主循环

qianxun-cli/src/
├── main.rs                   CLI 入口 (clap, 双模式路由)
├── lib.rs                    pub mod + run_repl()
├── cli.rs                    REPL 循环 (斜杠命令)
├── config.rs                 配置路径 + 默认配置模板生成
└── output.rs                 CliOutputSink (ANSI 终端输出)
```

## 术语表

| 术语 | 说明 |
|---|---|
| ACP | Agent Communication Protocol，与 Zed 等编辑器通信的 stdio JSON-RPC 2.0 协议 |
| AgentLoop | 代理引擎状态机 (Idle → WaitingLlm → ToolExecuting → ...) |
| OutputSink | 输出抽象 trait，引擎通过它输出文本/tool_call/事件，不感知具体输出目标 |
| ContextProvider | 上下文来源 trait，为 system prompt 注入记忆、技能等信息 |
| ToolRegistry | 工具注册中心，统一调度 builtin/skill/MCP 三层工具 |
| TokenBudget | 输入输出 token 预算，用于 enforce_budget 裁剪 |
| Workspace | 工作区信息 (根路径、项目类型、CLAUDE.md 内容) |
| MCP | Model Context Protocol，标准化的外部工具协议 |

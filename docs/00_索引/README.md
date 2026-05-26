# 索引目录

> 最后更新: 2026-05-26

## 任务路由

| 任务 | 入口 |
|---|---|
| 了解系统架构和模块划分 | `docs/10_事实源/架构设计.md` |
| 查看项目规则 | `CLAUDE.md` |
| 查看代码结构 | `CLAUDE.md` — 模块结构 |

## 仓库地图

```
qianxun-core/src/
├── types.rs                  核心类型 (LlmError, TokenUsage, AgentConfig)
├── output.rs                 OutputSink trait
├── event.rs                  AgentEvent 事件定义
├── agent/
│   ├── message.rs            Message enum, ContentBlock
│   ├── conversation.rs       Conversation (消息历史 + token 预算)
│   ├── engine.rs             AgentState, AgentLoop 状态机
│   └── system_prompt.rs      系统提示词组装
├── provider/
│   ├── mod.rs                LlmProvider trait
│   └── types.rs              CompletionRequest, LlmStreamEvent
├── tools/
│   ├── mod.rs                ToolRegistry, AgentTool trait
│   └── builtin.rs            内置工具 (ReadTextFile, WriteTextFile, Search)
├── context/
│   ├── mod.rs                ContextProvider trait
│   └── memory.rs             MemoryManager 骨架
├── skills/mod.rs             SkillManager 骨架
└── mcp/
    ├── mod.rs                MCP 模块
    └── client.rs             McpClient 骨架

qianxun-cli/src/
├── main.rs                   CLI 入口 (clap)
├── cli.rs                    REPL 循环
└── output.rs                 CliOutputSink (ANSI 终端)
```

## 术语表

| 术语 | 说明 |
|---|---|
| ACP | Agent Communication Protocol，与 Zed 等编辑器通信的 stdio 协议 |
| OutputSink | 输出抽象 trait，引擎通过它输出文本/tool_call/事件，不感知具体输出目标 |
| ContextProvider | 上下文来源 trait，为 system prompt 注入记忆、技能等信息 |
| TokenBudget | 输入输出 token 预算，用于 enforce_budget 裁剪 |
| MCP | Model Context Protocol，标准化的外部工具协议 |

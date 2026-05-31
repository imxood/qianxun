# 千寻 (Qianxun) 项目规则

> 状态: 生效 | 2026-05-26

## 项目概述

千寻是一个 Rust 实现的个人 AI 系统，可作为编程助手（CLI REPL + ACP 协议）和个人 AI 助理（Daemon 模式）。设计上强调分层解耦、构建顺序交付和私有部署。

## 技术栈

| 层 | 技术 |
|---|---|
| 语言 | Rust 2024 edition (MSRV 1.85) |
| 异步 | tokio (full) |
| CLI | clap (derive) |
| 序列化 | serde / serde_json |
| 日志 | tracing / tracing-subscriber |
| 错误 | thiserror / anyhow |
| UUID | uuid v4 |

## LLM Provider

- **默认 Provider**: DeepSeek Anthropic 兼容 API (`https://api.deepseek.com/anthropic/v1/messages`)
- **默认模型**: `deepseek-v4-flash`
- **协议**: Anthropic Messages API (SSE 流式)
- **API 密钥**: 通过环境变量 `DEEPSEEK_API_KEY` 配置

## 依赖策略

- 新增 crate 必须评估传递依赖树，超过 30 个传递依赖需评估替代方案
- **HTTP 客户端**: `reqwest`（`default-features=false`，启用 `json`, `stream`, `rustls`, `webpki-roots`），纯 Rust TLS 实现（ring 后端），无需 OpenSSL/cmake
- 禁止导入传递依赖超过 100 个的 crate
- 避免引入 Anthropic / OpenAI SDK 等额外封装层，直接使用 API 协议

## 模块结构

```
qianxun/                 # workspace 根
├── qianxun-core/        # 核心库 (lib)
│   ├── src/
│   │   ├── types.rs     # LlmError, TokenUsage, AgentConfig 等核心类型
│   │   ├── config.rs    # 全局配置 (JSON 带注释) + 解析
│   │   ├── output.rs    # OutputSink trait (输出抽象)
│   │   ├── event.rs     # AgentEvent, EventBus
│   │   ├── workspace.rs # .qianxun/ 项目根查找 + CLAUDE.md 读取
│   │   ├── agent/       # Conversation, Message, AgentLoop, system_prompt
│   │   ├── provider/    # LlmProvider trait, DeepSeek 实现
│   │   ├── tools/       # AgentTool trait, ToolRegistry, 5 个内置工具
│   │   ├── context/     # ContextProvider trait, MemoryManager
│   │   ├── skills/      # SkillManager (骨架)
│   │   └── mcp/         # MCP Client (骨架)
└── qianxun/             # 单二进制 (bin: qx)
    └── src/
        ├── main.rs      # 入口 (clap), cli/acp/daemon 模式路由
        ├── buf_writer.rs# 日志缓冲写入
        ├── cli/          # CLI REPL 模块
        │   ├── mod.rs
        │   ├── cli.rs    # REPL 循环
        │   ├── config.rs # 配置路径 + 默认配置生成
        │   ├── output.rs # CliOutputSink (ANSI 终端输出)
        │   └── run.rs    # run_repl 启动流程
        └── acp/          # ACP 协议模块
            ├── mod.rs
            ├── types.rs      # JSON-RPC 2.0 信封 + ACP 协议类型
            ├── transport.rs  # stdio 行帧读写 + 双向请求路由
            ├── session.rs    # SessionManager
            ├── output.rs     # AcpOutputSink
            ├── prompt.rs     # session/prompt → processing_loop 桥接
            ├── forwarding_tools.rs
            ├── handler.rs    # 请求路由
            └── server.rs     # ACP 主循环
```

## 构建顺序

| Phase | 交付 |
|---|---|
| 1 | 代码骨架 + 核心类型 + REPL CLI + LLM Provider (DeepSeek) + AgentLoop + 内置工具 |
| 2 | ACP 协议 + 工作空间支持 ✅ |
| 3 | Memory/Skills/MCP 集成 |
| 4 | Daemon 模式 + 完整 RAG |

## 开发命令

```bash
cargo build               # 编译全部
cargo build -p qianxun-core  # 仅编译 core
cargo clippy              # lint 检查
cargo test                # 运行测试
cargo run -- --help       # 查看 CLI 帮助
cargo run                 # 启动 REPL
cargo run -- --acp-mode   # 以 ACP 模式启动 (Phase 2)
```

## 参考项目

| 项目 | 路径 | 参考价值 |
|---|---|---|
| Zed editor | `E:\git\ai\zed` | ACP 协议实现、Agent 面板、Tool 系统、语言服务器集成 |

Zed 是千寻 ACP 模式的核心参考。重点目录：

```
crates/
├── language/                   # 语言系统、缓冲区、诊断
├── project/                    # 项目管理、文件操作、LSP
├── lsp/                        # 语言服务器协议
├── ai/                         # AI 辅助、内联补全、面板
├── assistant/                  # Assistant 面板、上下文管理
├── collab/                     # 协作协议
├── gpui/                       # GPU 加速 UI 框架
├── editor/                     # 编辑器核心
├── extensions/                 # 扩展系统
├── extensions_ui/              # 扩展管理 UI
├── extensions_api/             # 扩展 API
└── extensions_loader/          # 扩展加载器
```

## 设计原则

1. **分层解耦**: core 不依赖任何 binary 项目，OutputSink 让引擎不感知输出目标。
2. **系统提示词组装**: `system_prompt.rs` 统一构建，`Conversation::build_request()` 注入 memory + skills 上下文。
3. **预算先行**: `enforce_budget()` 在请求前检查 token 预算，防止过量。
4. **构建顺序交付**: 每个 Phase 交付可运行系统，不提前实现未规划的 feature。
5. **工具分三层**: builtin（核心文件/搜索）、skill（动态加载）、MCP（外部协议），统一通过 ToolRegistry 调度。

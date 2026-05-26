# 千寻 (Qianxun)

千寻是一个 Rust 实现的个人 AI 系统，可作为 AI 编程助手（CLI + ACP 协议）和个人 AI 助理。

> 众里寻他千百度, 千寻, 见所未见.

## 快速开始

### 前置条件

- Rust 1.85+（见 `rust-toolchain.toml`）

### 安装与运行

```bash
cargo build
cargo run                        # 启动 REPL
cargo run -- --model deepseek-chat  # 指定模型
cargo run -- --generate-config   # 生成默认配置文件
```

### 配置文件

默认路径 `~/.qianxun/config.json5`（JSON5 格式），可用 `--config <path>` 覆盖。

优先级：CLI 参数 > 环境变量 > 配置文件 > 内置默认值

```bash
cargo run -- --generate-config  # 生成配置模板
```

### REPL 命令

| 命令 | 用途 |
|------|------|
| `/quit` | 退出 |
| `/reset` | 重置对话 |
| `/usage` | 查看 token 用量 |

## 项目结构

```
qianxun/
├── qianxun-core/        # 核心库（引擎、Provider、工具系统）
│   └── src/
│       ├── agent/       # 消息、会话、Agent 循环、系统提示词
│       ├── provider/    # LLM Provider trait + DeepSeek 实现
│       ├── tools/       # 工具系统 + 内置工具
│       ├── context/     # 上下文与记忆（骨架）
│       ├── skills/      # Skills 系统（骨架）
│       ├── mcp/         # MCP 客户端（骨架）
│       └── config.rs    # 配置文件解析与合并
└── qianxun-cli/         # CLI 二进制
    └── src/
        ├── main.rs      # 入口（clap）
        ├── cli.rs       # REPL 循环
        └── config.rs    # 平台配置路径检测
```

## 路线图

| Phase | 交付 |
|-------|------|
| 1 | 代码骨架 + 核心类型 + REPL CLI + LLM Provider + AgentLoop + 内置工具 ✅ |
| 2 | ACP 协议（Zed 集成） |
| 3 | Memory/Skills/MCP 集成 |
| 4 | Daemon 模式 + 完整 RAG |

## 文档

详见 [`docs/`](docs/) 目录。

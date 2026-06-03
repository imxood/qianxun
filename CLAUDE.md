# 千寻 (Qianxun) 项目规则

> 状态: 生效 | 2026-06-01

## 项目概述

千寻是一个 Rust 实现的个人 AI 系统，可作为编程助手（独立 TUI/CLI REPL + ACP 协议）和个人 AI 助理（Daemon 模式、VPS Server）。设计上强调分层解耦、构建顺序交付和私有部署。

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
├── qianxun-core/        # 核心库 (lib) — 引擎 + Provider + 工具
│   ├── src/
│   │   ├── types.rs     # LlmError, TokenUsage, AgentConfig 等核心类型
│   │   ├── config.rs    # 全局配置 (JSON 带注释) + 解析
│   │   ├── output.rs    # OutputSink trait (输出抽象)
│   │   ├── event.rs     # AgentEvent, EventBus
│   │   ├── workspace.rs # .qianxun/ 项目根查找 + CLAUDE.md 读取
│   │   ├── agent/       # Conversation, Message, AgentLoop, system_prompt, plan/reflect/workflow
│   │   │   └── context/ # window, normalize, compact (上下文压缩/规范化/窗口管理)
│   │   ├── provider/    # LlmProvider trait, DeepSeek 实现
│   │   ├── tools/       # AgentTool trait, ToolRegistry, 内置工具
│   │   ├── context/     # ContextProvider trait, MemoryObserver trait
│   │   ├── skills/      # SkillManager (frontmatter 解析 + 项目/全局加载)
│   │   └── mcp/         # MCP Client + ServerManager + transport (stdio/HTTP)
├── qianxun-memory/      # 记忆引擎 (lib) — SQLite + FTS5 + Vector 混合检索
│   └── src/
│       ├── lib.rs           # MemoryCore 入口，实现 MemoryObserver
│       ├── types.rs         # MemoryRecord / MemorySearchResult / MemoryStats
│       ├── db.rs            # SQLite schema (observations/sessions/memories/tags + obs_fts)
│       ├── search.rs        # BM25 检索
│       ├── vector.rs        # VectorIndex 骨架
│       ├── consolidation.rs # Observation → Memory 聚类压缩
│       ├── compressor.rs    # 文本压缩 + 合成观察
│       ├── privacy.rs       # 隐私数据清洗
│       └── slot.rs          # 槽位管理
└── qianxun/             # 单二进制 (bin: qx) — 四种入口 + 旧 CLI
    └── src/
        ├── main.rs      # 入口 (clap), tui/acp/daemon/server 模式路由 + 薄客户端
        ├── buf_writer.rs# 日志缓冲写入
        ├── tui/         # 交互式 TUI (ratatui + Inline Viewport + 脏标记渲染)
        │   └── mod.rs   # 单文件 ~1730 行，对话视图 + 命令弹窗 + 流式输出
        ├── cli/         # 旧 REPL (行将迁移至 TUI/Daemon 客户端)
        │   ├── cli.rs
        │   ├── config.rs
        │   ├── output.rs
        │   └── run.rs
        ├── acp/         # ACP 协议 (Zed 集成)
        │   ├── types.rs, transport.rs, session.rs, output.rs
        │   ├── prompt.rs, forwarding_tools.rs, handler.rs, server.rs
        ├── daemon/      # Daemon 模式 (HTTP + axum) — 骨架, 未接 AgentLoop
        │   ├── mod.rs, router.rs, agent_host.rs
        └── server/      # VPS Server 模式 (HTTP + jwt + keyring)
            ├── mod.rs, auth.rs
```

### 缺口 7 修复 (MVP-0, 2026-06-03)

`qianxun/src/daemon/mod.rs` 第 125-165 行的 3 个启动序列块 (tools / memory / skills) 全部**真实初始化**, 不再是 `None` / `in_memory` 占位:

- `tools`: `ToolRegistry::new()` + `register_all_builtin()` 注册 8 个 builtin 工具 (失败 fallback 空 + warn)
- `memory`: `MemoryCore::open("~/.qianxun/mem.db")` 真 SQLite 路径 (失败 fallback `open_in_memory()` + warn)
- `skills`: `SkillManager::load_all(None)` 同步加载全局 skill (空目录静默 OK)

执行历史: Day 1-3 commits `ea7b335` / `da04950` / `02fb2e2` / `159f966`, Day 4 E2E 验收 commit `42e1bdd` (cargo test 214/0 + clippy 0/0 + daemon 三端点全 PASS). 详细计划见 `docs/30_子项目规划/05-mvp-0-checklist.md`, daemon 现状见 `docs/10_事实源/daemon-state.md`.

## 构建顺序

| Phase | 状态 | 交付 |
|---|---|---|
| 1 | ✅ | 代码骨架 + 核心类型 + REPL CLI + LLM Provider (DeepSeek) + AgentLoop + 内置工具 |
| 2 | ✅ | ACP 协议 + 工作空间支持 |
| 3a | 🟡 | Memory (SQLite+FTS5) + MCP (ServerManager+transport) + Skills (frontmatter 加载) — 骨架/部分闭环,见 `docs/10_事实源/memory-state.md` |
| 3b | 🟡 | AgentPattern 类型 + plan/reflect/workflow 模块 — 未接入主链路 |
| 3c | 🟡 | Daemon HTTP 骨架 (axum + 11 路由 + session CRUD) — 未接 AgentLoop,见 `docs/10_事实源/daemon-state.md` |
| 3d | ✅ | 独立 TUI 模式 (ratatui, 脏标记驱动渲染 + 增量行缓存) |
| 4a | 📋 | Daemon 升级为唯一 Agent runtime, TUI/ACP 改 thin client |
| 4b | 📋 | VPS Server 完整 (WebSocket Hub + 完整认证) + 完整 RAG |

> 实时状态见 `docs/10_事实源/` 各子系统状态文件 + `docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md` (A-G 路线)。

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

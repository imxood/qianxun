# 千寻 (Qianxun) 项目规则

> 状态: 生效 | 2026-06-09 | 核心需求: **tauri → core** 端到端跑通

## 项目概述

千寻是个人 AI 系统。核心目标: Tauri 桌面 (Svelte 5) 调 qianxun-core 引擎 (in-process, 不走 HTTP 中转),实现 TUI/ACP/Daemon 三端共享同一引擎。

设计强调分层解耦、构建顺序交付和私有部署。

## 仓库结构

| Crate | 路径 | 角色 |
|---|---|---|
| `qianxun-core` | `qianxun-core/` | 核心引擎: Agent 循环 + Provider + Tools + Skills + MCP + Config + workspace |
| `qianxun-memory` | `qianxun-memory/` | SQLite + FTS5 记忆层 |
| `qianxun-runtime` | `qianxun-runtime/` | Agent 运行时封装: AgentLoopHost + RuntimeApi + SseEvent + SessionStore |
| `qianxun` | `qianxun/` | 主二进制 (cli/daemon/server/acp/tui 5 模式) |
| `qianxun-desktop` | `qianxun-desktop/` | Tauri + Svelte 5 桌面端 (**独立 workspace**, 走 path dep) |
| `qianxun-test` | `qianxun-test/` | Python e2e 测试 harness |

## LLM Provider

- 默认 provider 通过环境变量 `QIANXUN_ACTIVE_PROVIDER` 配置
- 备选从 `~/.qianxun/config.json` 的 `active_provider` 字段读
- DeepSeek 专有环境变量: `DEEPSEEK_API_KEY` (大小写不敏感)
- minimax 专有环境变量: `MINIMAX_API_KEY` (或 `MINIMAX_API_KEY`)
- 默认 base_url: `https://api.deepseek.com/anthropic/v1/messages` (Anthropic 协议)
- 默认模型: 在 `~/.qianxun/config.json` 的 `providers.<name>.model` 字段配置,常见值 `deepseek-v4-flash` / `MiniMax-M3`

## 核心架构决策

**ADR-0003** (2026-06-08): 桌面 + ACP 同进程 2-Mode 互斥

- **Mode A**: 桌面 Tauri webview, Svelte 5 → Tauri IPC invoke → qianxun-runtime API (in-process, 类型安全)
- **Mode B**: `--acp` stdio 协议 (复用现有 `qianxun/src/acp/`)

详细决策: `docs/决策/ADR-0003_desktop_2mode.md`

**Supersedes**: ADR-0002 (daemon design chat-first) **整篇** (2026-06-09 文件已删除)

## 关键代码路径

| 层 | 路径 |
|---|---|
| Tauri 端 | `qianxun-desktop/src/lib/` (Svelte 5) + `qianxun-desktop/src-tauri/src/` (Rust) |
| Runtime 端 | `qianxun-runtime/src/api/` (RuntimeApi) + `state.rs` + `agent_host.rs` |
| Core 端 | `qianxun-core/src/agent/engine.rs` (AgentLoop) |
| 事实源 | `docs/事实源/runtime-state.md`, `desktop-state.md` |

## 依赖策略

- **避免引入 SDK 封装层**, 直接使用 API 协议 (Anthropic Messages / OpenAI)
- **HTTP 客户端**: `reqwest` (default-features=false, json + stream + rustls)
- 新增 crate 必须评估传递依赖, **超过 30 个需评估替代方案**
- **禁止导入传递依赖超过 100 个的 crate**

## 技术栈

| 层 | 技术 |
|---|---|
| 语言 | Rust 2024 edition (MSRV 1.85) |
| 异步 | tokio (full) |
| CLI | clap (derive) |
| 序列化 | serde / serde_json |
| 日志 | tracing / tracing-subscriber |
| 错误 | thiserror / anyhow |
| 桌面 UI | Tauri 2.x + Svelte 5 |
| 数据库 | SQLite (rusqlite) + FTS5 |

## 设计原则

1. **分层解耦**: core 不依赖 binary 项目, OutputSink 让引擎不感知输出目标
2. **RuntimeApi 作为唯一暴露面**: Tauri/daemon 都通过 `impl RuntimeApi for Arc<RuntimeState>` 共享业务
3. **流式响应走 mpsc**: 业务用 `mpsc::Receiver<SseEvent>`, 传输层 (Tauri emit / HTTP SSE) 各自包
4. **构建顺序交付**: 每个 Phase 交付可运行系统, 不提前实现未规划 feature
5. **工具分三层**: builtin (核心文件/搜索) / skill (动态加载) / MCP (外部协议), 统一通过 ToolRegistry 调度

## 文档

| 目录 | 职责 |
|---|---|
| `docs/README.md` | 文档总入口 |
| `docs/事实源/` | 子系统真实状态 (runtime-state, desktop-state) |
| `docs/设计/` | 整体架构 (总设计 + agent_loop_v2 + 14 缺口全景) |
| `docs/设计/能力层/` | 14 个独立缺口文档 (01-14_*.md) |
| `docs/设计/规范/` | 4 份跨缺口规范 (15-18: 文件层级/接口契约/异常路径/可观测性) |
| `docs/设计/TODO/` | 阶段性 TODO/工作项 (含 2026-06-11_v2_缺口补齐_14项/) |
| `docs/决策/` | 长期架构决策 (ADR) |
| `docs/子项目规划/` | 跨 Track 规划 (04b tauri-runtime 集成, 04c runtime 抽取, _shared-contract, 00-RUNNING-GUIDE) |
| `docs/经验/` | 实施经验与最近 4 阶段收尾 |

## 构建顺序

| Phase | 状态 | 交付 |
|---|---|---|
| 1 | ✅ | 代码骨架 + 核心类型 + REPL CLI + LLM Provider (DeepSeek) + AgentLoop + 内置工具 |
| 2 | ✅ | ACP 协议 + 工作空间支持 |
| 3a | ✅ | Memory (SQLite+FTS5) + MCP + Skills |
| 3b | ✅ | AgentPattern + plan/reflect/workflow 模块 |
| 3c | ✅ | Daemon HTTP 骨架 |
| 3d | ✅ | 独立 TUI (ratatui + 脏标记渲染) |
| 4a-1 | ✅ | qianxun-runtime 抽取 + Tauri 集成 + Svelte 切真后端 |
| **4a-2** | 🟡 | **当前**: 用户手动 E2E 验收 (见 经验/Phase_ABCD_收尾总览) |

## 开发命令

```bash
cargo build                       # 编译全部
cargo test                        # 运行测试 (基线 248 passed)
cargo clippy                      # lint 检查
cd qianxun-desktop && pnpm tauri dev   # 启桌面端 (需 DEEPSEEK_API_KEY)
cargo run -- --help               # 查看 CLI 帮助
```

## 当前 P0 收尾 (4a-2 阶段, 按优先级)

> 注意: 这跟 [设计/TODO/2026-06-11_v2_缺口补齐_14项/](./docs/设计/TODO/2026-06-11_v2_缺口补齐_14项/) 的 14 缺口 P0 (02/01/04/03/05) 是**两套独立 P0**, 不要混淆。

1. **用户手动 E2E 验收** (6 步清单, 关键)
2. **sub_session 后端实现**
3. **list_plans Tauri command 注册**
4. **project.svelte.ts 后端实现**

详细缺口列表见 `docs/事实源/runtime-state.md` 和 `desktop-state.md` 的 "已知缺口" 段。

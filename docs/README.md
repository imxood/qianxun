# 千寻 (Qianxun) 文档

> 状态: 生效 | 最后更新: 2026-06-10 | 核心需求: **tauri → core** 端到端跑通 + v2 AgentLoop 演进

## 核心入口

- **总设计**: [设计/总设计.md](设计/总设计.md) — 千寻整体架构 + 14 缺口整合 + 4 集成点
- **v2 基础设施**: [设计/agent_loop_v2.md](设计/agent_loop_v2.md) — HookRegistry + 双轴模式 + SubAgent
- **14 缺口全景**: [设计/00_总览.md](设计/00_总览.md) — P0 5 + P1 9 个生产级能力
- **当前架构决策**: [ADR-0003: 桌面 + ACP 同进程 2-Mode 互斥](决策/ADR-0003_desktop_2mode.md)
- **qianxun-runtime 子系统状态**: [事实源/runtime-state.md](事实源/runtime-state.md)
- **qianxun-desktop 子系统状态**: [事实源/desktop-state.md](事实源/desktop-state.md)
- **Tauri + Runtime 集成规划**: [子项目规划/04b-tauri-runtime-integration.md](子项目规划/04b-tauri-runtime-integration.md)
- **qianxun-runtime 抽取设计**: [子项目规划/04c-qianxun-runtime-extraction.md](子项目规划/04c-qianxun-runtime-extraction.md)
- **跨 Track 契约**: [子项目规划/_shared-contract.md](子项目规划/_shared-contract.md)
- **运行指南**: [子项目规划/00-RUNNING-GUIDE.md](子项目规划/00-RUNNING-GUIDE.md)
- **最近 4 阶段收尾**: [经验/2026-06-08_Phase_ABCD_收尾总览.md](经验/2026-06-08_Phase_ABCD_收尾总览.md)

## 设计基线

```
Tauri Desktop (Svelte 5 webview)
    ↓ Tauri IPC invoke (in-process, 类型安全)
Tauri command (src-tauri/src/commands/runtime/*)
    ↓ RuntimeApi trait
qianxun-runtime (AgentLoopHost + DaemonOutputSink + SseEvent)
    ↓ AgentLoop::new
qianxun-core (Conversation + Message + AgentLoop)
    ↓ reqwest + Anthropic 协议
LLM (DeepSeek / minimax / 其他)
```

**关键约束**:
- Tauri 调 runtime **不走 HTTP** (in-process library)
- 桌面 binary 自带 core engine,daemon 仅作可选 VPS 远端
- 流式响应走 `mpsc::Receiver<SseEvent>` → Tauri `emit("session_event")` → Svelte 12-event 状态机

## 仓库结构

```
qianxun/                  # workspace 根
├── CLAUDE.md              # 项目规则 (LLM 入口)
├── qianxun-core/          # 核心引擎 (Agent 循环 + Provider + Tools + Skills + MCP)
├── qianxun-memory/        # SQLite + FTS5 记忆层
├── qianxun-runtime/       # Agent 运行时封装 (RuntimeApi + SseEvent)
├── qianxun/               # 主二进制 (cli/daemon/server/acp/tui 5 模式)
├── qianxun-desktop/       # Tauri + Svelte 5 桌面端 (独立 workspace)
├── qianxun-test/          # Python e2e 测试 harness
└── docs/                  # 文档
```

## 文档结构

| 目录 | 职责 |
|---|---|
| `事实源/` | 子系统真实状态 (runtime, desktop) |
| `设计/` | 设计事实源 (总设计 + agent_loop_v2 + 14 缺口全景) |
| `设计/能力层/` | 14 个独立缺口文档 (01-14_*.md) |
| `设计/规范/` | 4 份跨缺口规范 (15-18: 文件层级/接口契约/异常路径/可观测性) |
| `设计/TODO/` | 阶段性 TODO/工作项 |
| `决策/` | 长期架构决策 (ADR) |
| `子项目规划/` | 跨 Track 规划和运行指南 |
| `经验/` | 实施经验与收尾记录 |

## 当前状态 (2026-06-09)

| Phase | 状态 | 说明 |
|---|---|---|
| 1-2 | ✅ | Agent 引擎 + Provider + ACP + 工作区 |
| 3a/3b | ✅ | Memory + Skills + MCP + Agent 模式(plan/reflect/workflow) |
| 3c | ✅ | Daemon HTTP 骨架 |
| 3d | ✅ | 独立 TUI (ratatui) |
| 4a-1 | ✅ | qianxun-runtime crate 抽取 + RuntimeApi 收口 + Tauri 集成 + Svelte 切真后端 |
| 4a-2 | 🟡 | **当前**:用户手动 E2E 验收 6 步 (见 经验/Phase_ABCD_收尾总览) |

## 后续工作 (Phase E)

参考 [经验/2026-06-08_Phase_ABCD_收尾总览.md](经验/2026-06-08_Phase_ABCD_收尾总览.md) "用户后续工作 (Phase E)" 章节。简要:

1. **P0-1**: 用户手动跑 6 步 E2E 验收(关键)
2. **P0-2**: sub_session 后端实现
3. **P0-3**: list_plans Tauri command 注册
4. **P0-4**: project.svelte.ts 后端实现
5. **P1-1/2/3/4**: Plan 持久化 / 路径分离 / PlanUpdate 事件 / connection 真实化

详细见 `runtime-state.md` 和 `desktop-state.md` 的"已知缺口"段。

# 千寻 (Qianxun) 文档

> 最后更新: 2026-06-03

## 文档结构

```
docs/
├── README.md               # 本文档 — 总入口
├── 00_索引/                # 导航和路由
│   └── README.md           # 任务路由、仓库地图、术语表
├── 10_事实源/              # 当前模块真实状态
│   ├── memory-state.md     # Memory 子系统状态 (MVP-0 已落地)
│   ├── mcp-state.md        # MCP 子系统状态
│   ├── skills-state.md     # Skills 子系统状态
│   ├── daemon-state.md     # Daemon 子系统状态 (Stage 1-10 全完成)
│   └── tui-architecture.md # TUI 架构状态
├── 20_工作项/              # 阶段性工作上下文 (2026-06-03 收尾 4 项)
│   ├── 2026-05-31_Phase3_记忆子系统设计修订/   ✅ 通过 MVP-0 落地
│   ├── 2026-05-31_模块设计文档起草/           ✅ 5/5 子任务全通过
│   ├── 2026-06-01_qx交互式TUI调研/            ✅ P0 完成
│   └── 2026-06-02_DaemonWebAdminConsole规划/  ✅ Stage 7a->10c 全部完成
├── 30_决策/                # 长期架构决策
│   └── ADR-0001_数据库选型.md  # redb → SQLite 选型记录
├── 30_子项目规划/          # 子项目详细规划 (Stage 1-10 + Kanban v6 + MVP-0/1)
│   ├── 00-RUNNING-GUIDE.md
│   ├── 01-daemon.md
│   ├── 01b-daemon-web-console.md
│   ├── 02-vps-server.md
│   ├── 03-tauri-desktop.md
│   ├── 04-kanban-design.md         # v6 多 Agent Kanban 设计
│   ├── 05-mvp-0-checklist.md       # MVP-0 详细任务清单
│   ├── 06-mavis-执行历史.md        # 2026-06-03 归档, mavis 编排执行历史
│   └── _shared-contract.md
├── 40_验收/                # 有长期复盘价值的验证证据
├── 90_历史/                # 已废弃有追溯价值的文档
│   ├── ai分析/             # 早期 AI 分析产出（保留追溯）
│   └── 2026-06-01_TUI性能与Agent开发工具优化_未执行/  # 路线 A-G 归档
│
├── architecture.md         # 统一架构设计
├── agent-pattern-design.md # Agent 模式设计（Phase 3b）
├── memory-design.md        # 记忆子系统设计（Phase 3a）
├── mcp-design.md           # MCP Client 设计（Phase 3a）
├── skills-design.md        # Skill 系统设计（Phase 3a）
├── daemon-design.md        # Daemon 模式设计（Phase 3c / 4a）
└── vps-server-design.md    # VPS Server 设计（Phase 4b）
```

## 当前状态

> 校准: 2026-06-03。详细状态见 `docs/10_事实源/` 五个子系统状态文件,
> `docs/30_子项目规划/06-mavis-执行历史.md` (mavis 编排 53+ commit 归档),
> 以及 `04-kanban-design.md` v6 (多 Agent Kanban 8 周 MVP 落地计划)。

| Phase | 状态 | 内容 |
|---|---|---|
| 1-2 | ✅ 完成 | Agent 引擎 + Provider + 工具 + CLI REPL + ACP + 工作区 |
| 3a | ✅ 落地 | Memory (SQLite+FTS5 + 8 表 + VectorIndex 骨架) + MCP (ServerManager+transport) + Skills (frontmatter 加载) — **MVP-0 (2026-06-03) 全部 commit 落地** |
| 3b | 🟡 部分实现 | AgentPattern 类型 + plan/reflect/workflow 模块 — 未接入主链路 |
| 3c | ✅ 完成 | Daemon HTTP 骨架 + 完整 AgentLoop 接入 + 17 endpoint + 10 面板 + Chat — **Stage 1-10c 全部 commit 落地** |
| 3d | ✅ 完成 | 独立 TUI 模式 (ratatui + 脏标记渲染 + 增量行缓存 + 1MB 压测) — 见 `tui-architecture.md` |
| 4a | 🟡 进行中 (本计划) | **多 Agent Kanban 架构 (MVP-2~MVP-6)**, 见 `04-kanban-design.md` v6 + `06-mavis-执行历史.md` §6 |
| 4b | 📋 待实现 | VPS Server 完整 (WebSocket Hub + 完整认证) + 完整 RAG |

### 关键里程碑 (2026-05-31 ~ 2026-06-03)

- **2026-06-03 MVP-0 闭环** — 缺口 7 修复, `daemon/mod.rs` 三占位 → 真初始化, 6 个 commit 落地, cargo test 214/0 + clippy 0/0
- **2026-06-03 MVP-1 闭环** — prompt_handler 真实接 processing_loop (缺口 2/3/6 修复), 4 个 commit 落地
- **2026-06-03 Stage 1-10c 全部完成** — Daemon Web Admin Console 10 面板 + Chat 视图 + Admin 密码 + Graceful Shutdown + Stronghold 真测, 17 个 commit 落地
- **2026-06-03 多 Agent Kanban 设计 v6 完成** — `04-kanban-design.md` (2343 行), 8 周 MVP 计划 (MVP-2~MVP-6), 8 张新表 + 12 工具 + 23 路由 + 5 SSE 事件

## 文档职责一览

| 文件 | 类型 | Phase |
|---|---|---|
| `docs/architecture.md` | 统一架构设计 | 1-4 ✅ |
| `docs/agent-pattern-design.md` | Agent 模式设计 | 3b 🟡 |
| `docs/memory-design.md` | 记忆子系统设计 | 3a ✅ (MVP-0 落地) |
| `docs/mcp-design.md` | MCP Client 设计 | 3a 🟡 |
| `docs/skills-design.md` | Skill 系统设计 | 3a 🟡 |
| `docs/daemon-design.md` | Daemon 模式设计 | 3c ✅ / 4a 🟡 |
| `docs/vps-server-design.md` | VPS Server 设计 | 4b 📋 |
| `docs/10_事实源/` | 子系统真实状态 | 持续维护 |
| `docs/30_子项目规划/04-kanban-design.md` | v6 多 Agent Kanban 设计 (2343 行) | 4a 🟡 (本计划) |
| `docs/30_子项目规划/06-mavis-执行历史.md` | mavis 编排执行历史 (53+ commit 归档) | — |
| `docs/30_决策/ADR-0001_数据库选型.md` | 决策记录 | 3 |
| `docs/90_历史/2026-06-01_TUI性能与Agent开发工具优化_未执行/` | TUI 路线 A-G 归档 | — |
| `CLAUDE.md` | 项目规则 | — |

## 导航

- 了解系统架构 → `docs/architecture.md`
- 了解各个子系统设计 → 上述设计文档
- 多 Agent Kanban v6 详细设计 → `docs/30_子项目规划/04-kanban-design.md`
- mavis 编排执行历史 → `docs/30_子项目规划/06-mavis-执行历史.md`
- 快速索引和任务路由 → `docs/00_索引/README.md`
- 项目规则 → `CLAUDE.md`
- 当前工作项 → `docs/20_工作项/`
- 归档工作项 → `docs/90_历史/`

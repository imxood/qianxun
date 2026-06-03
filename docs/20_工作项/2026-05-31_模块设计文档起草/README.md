# 工作项: 模块设计文档起草

> 状态: ✅ 已完成, 5/5 子任务全通过, 收尾归并 | 创建: 2026-05-31 | 收尾: 2026-06-03

## 目标

为千寻未实现的子系统创建独立设计文档，每个文档承载该模块的完整设计决策、接口定义和实现要点。

## 模块清单

| 模块 | 文档 | Phase | 依赖 |
|---|---|---|---|
| MCP Client | `docs/mcp-design.md` | 3 | 协议已标准化，边界最清晰 |
| Skill 系统 | `docs/skills-design.md` | 3 | 格式规范需尽早定型 |
| Daemon 模式 | `docs/daemon-design.md` | 4 | 依赖 Memory/MCP/Skills 就绪 |
| VPS Server | `docs/vps-server-design.md` | 4 | 依赖 Daemon 就绪 |
| Agent 模式 | `docs/agent-pattern-design.md` | 3b | 跟 plan/reflect/workflow 模块一起 |

## 完成情况 (2026-06-03)

5 个模块设计文档全部落地 (见 TODO.md 5/5 ✅), 内容覆盖各模块完整设计决策 / 接口定义 / 实现要点. 后续工作:
- 模块设计落地全部走 mavis 编排, 见 `06-mavis-执行历史.md` Stage 1-10 + MVP-0/1
- 文档已迁移到 `docs/30_子项目规划/` 子目录, 路径有所调整:
  - `01-daemon.md` (原 daemon-design.md)
  - `02-vps-server.md` (原 vps-server-design.md)
  - `03-tauri-desktop.md` (Tauri 后续加, 不在本工作项范围)
  - MCP/Skills 设计内联到 `docs/10_事实源/` 跟代码同步

## 对应 plans 决策

调研型工作项, 无对应 mavis plan. 5 个文档均通过 session 内手工起草 + 多次修订, 关键时间窗 2026-05-31 之前. 后续模块的代码实现 (MCP / Skills / Daemon / VPS) 走 mavis 编排, 见 `06-mavis-执行历史.md` §2 阶段总表 + §6 工作项对应表.

## 关联文档

- `docs/30_子项目规划/06-mavis-执行历史.md` — mavis 编排执行历史
- `docs/30_子项目规划/04-kanban-design.md` — 多 Agent Kanban 架构 (v6)
- `docs/10_事实源/` — 后续模块落地后的稳定事实源
- `docs/architecture.md` — 统一架构设计 (旧, 大部分内容已迁到 30_子项目规划/)

# 千寻 (Qianxun) 文档

> 最后更新: 2026-05-31

## 文档结构

```
docs/
├── README.md               # 本文档 — 总入口
├── 00_索引/                # 导航和路由
│   └── README.md           # 任务路由、仓库地图、术语表
├── 20_工作项/              # 阶段性工作上下文
│   ├── 2026-05-31_Phase3_记忆子系统设计修订/
│   │   ├── README.md       # 设计修订的目标和范围
│   │   └── TODO.md         # 任务清单
│   └── 2026-05-31_模块设计文档起草/
│       ├── README.md       # 模块设计清单
│       └── TODO.md         # 任务清单
├── 30_决策/                # 长期架构决策
│   └── ADR-0001_数据库选型.md  # redb → SQLite 选型记录
├── 40_验收/                # 有长期复盘价值的验证证据
├── 90_历史/                # 已废弃有追溯价值的文档
│   └── ai分析/             # 早期 AI 分析产出（保留追溯）
│
├── architecture.md         # 统一架构设计
├── agent-pattern-design.md # Agent 模式设计（Phase 3b）
├── memory-design.md        # 记忆子系统设计（Phase 3a）
├── mcp-design.md           # MCP Client 设计（Phase 3a）
├── skills-design.md        # Skill 系统设计（Phase 3a）
├── daemon-design.md        # Daemon 模式设计（Phase 3c / 4a）
└── vps-server-design.md    # VPS Server 设计（Phase 4b）
```

## 当前重点

Phase 1-2 已交付。Phase 3 所有模块的设计文档已起草完成，进入逐份精修阶段。

## 文档职责一览

| 文件 | 类型 | Phase |
|---|---|---|
| `docs/architecture.md` | 统一架构设计 | 1-4 |
| `docs/agent-pattern-design.md` | Agent 模式设计 | 3b 🔧 |
| `docs/memory-design.md` | 记忆子系统设计 | 3a 🔧 |
| `docs/mcp-design.md` | MCP Client 设计 | 3a 🔧 |
| `docs/skills-design.md` | Skill 系统设计 | 3a 🔧 |
| `docs/daemon-design.md` | Daemon 模式设计 | 3c / 4a 🔧 |
| `docs/vps-server-design.md` | VPS Server 设计 | 4b 📋 |
| `docs/30_决策/ADR-0001_数据库选型.md` | 决策记录 | 3 |
| `CLAUDE.md` | 项目规则 | — |

## 导航

- 了解系统架构 → `docs/architecture.md`
- 了解各个子系统设计 → 上述设计文档
- 快速索引和任务路由 → `docs/00_索引/README.md`
- 项目规则 → `CLAUDE.md`
- 当前工作项 → `docs/20_工作项/`

# 千寻 (Qianxun) 文档

> 最后更新: 2026-05-27

## 文档结构

```
docs/
├── README.md               # 本文档 — 总入口
├── 00_索引/                # 导航和路由
│   └── README.md           # 任务路由、仓库地图、术语表
├── 10_事实源/              # 当前稳定事实
│   ├── 架构设计.md         # 系统架构、模块、核心流程
│   └── (更多事实源)
├── 20_工作项/              # 阶段性工作上下文
└── 90_历史/                # 已废弃有追溯价值的文档
```

## 当前重点

Phase 1-2 已交付，当前正在进行架构评审和 Phase 3 (Memory/Skills/MCP) 规划。

- CLI 模式: `qx` — 交互式 REPL，支持工作区自动检测
- ACP 模式: `qx --acp-mode` — 通过 stdio JSON-RPC 2.0 与 Zed 编辑器通信
- 默认配置: `~/.qianxun/config.json5`，可通过 `qx --generate-config` 生成模板

## 导航

- 了解系统整体架构 → `docs/10_事实源/架构设计.md`
- 快速索引和任务路由 → `docs/00_索引/README.md`
- 项目规则 → `CLAUDE.md`
- 工作项 → `docs/20_工作项/`

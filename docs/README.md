# 千寻 (Qianxun) 文档

> 最后更新: 2026-05-26

## 文档结构

```
docs/
├── README.md               # 本文档 — 总入口
├── 00_索引/                # 导航和路由
└── 10_事实源/              # 当前稳定事实
    └── 架构设计.md         # 系统架构、模块、核心流程
```

## 当前重点

Phase 2 (ACP 协议 + 工作空间支持) 已完成。CLI REPL 和 ACP 模式均可工作。
- CLI 模式: `qx` — 交互式 REPL，支持工作区自动检测
- ACP 模式: `qx --acp-mode` — 通过 stdio JSON-RPC 2.0 与 Zed 编辑器通信
- 默认配置: `~/.qianxun/config.json5`，可通过 `qx --generate-config` 生成模板

## 导航

- 了解系统整体架构 → `docs/10_事实源/架构设计.md`
- 项目规则 → `CLAUDE.md`
- 工作项 → `docs/20_工作项/` (未来)

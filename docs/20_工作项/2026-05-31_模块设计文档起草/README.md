# 工作项: 模块设计文档起草

> 状态: 进行中 | 创建: 2026-05-31

## 目标

为千寻未实现的子系统创建独立设计文档，每个文档承载该模块的完整设计决策、接口定义和实现要点。

## 模块清单

| 模块 | 文档 | Phase | 依赖 |
|---|---|---|---|
| MCP Client | `docs/mcp-design.md` | 3 | 协议已标准化，边界最清晰 |
| Skill 系统 | `docs/skills-design.md` | 3 | 格式规范需尽早定型 |
| Daemon 模式 | `docs/daemon-design.md` | 4 | 依赖 Memory/MCP/Skills 就绪 |
| VPS Server | `docs/vps-server-design.md` | 4 | 依赖 Daemon 就绪 |

## 关联文档

- `docs/architecture.md` — 统一架构设计（各模块的上层设计在此展开）
- `docs/memory-design.md` — 已完成的记忆子系统设计（模板参考）

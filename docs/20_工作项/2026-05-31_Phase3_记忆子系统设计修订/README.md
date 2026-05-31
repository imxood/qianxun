# 工作项: Phase 3 记忆子系统设计修订

> 状态: 设计修订中 | 创建: 2026-05-31

## 目标

对 `architecture.md` 和 `memory-design.md` 两份设计文档进行系统性修订，重点解决：

1. **数据库选型**: redb → SQLite (rusqlite)，消除全表扫描、Schema 演进、调试三大痛点
2. **BM25 持久化问题**: 全量序列化 → SQLite FTS5 内置全文索引
3. **Consolidation 设计密度不足**: 补充完整聚类算法、回滚策略、并发协调
4. **存储性能优化**: spawn_blocking 批处理、嵌入缓存、自动 compact
5. **Phase 边界不清晰**: architecture.md 标注已实现/规划中的状态

## 关联事实源

- `docs/10_事实源/架构设计.md` — 当前实现的事实源（不受影响）
- `docs/architecture.md` — Phase 4 目标架构（需修订）
- `docs/memory-design.md` — Phase 3 记忆子系统设计（需大改）

## 设计决策记录

| 决策 | 状态 | 位置 |
|---|---|---|
| 数据库: redb → SQLite (rusqlite bundled) | 已确认 | `memory-design.md` §3 |
| 全文搜索: 自建 BM25 → FTS5 | 已确认 | `memory-design.md` §4 |
| 向量存储: WAL 追加 → SQLite BLOB 列 | 已确认 | `memory-design.md` §4 |
| Consolidation 展开 | 待定 | `memory-design.md` §9 |
| 批处理写入 | 待定 | `memory-design.md` §5 |

## 下一步

1. 更新 `memory-design.md` — 存储层 + 全文搜索 + 向量存储
2. 更新 `memory-design.md` — Consolidation 展开
3. 更新 `memory-design.md` — 性能优化
4. 更新 `architecture.md` — Phase 边界 + 补充决策
5. 文档结构清理

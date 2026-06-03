# TODO

> 最后更新: 2026-06-03 | 状态: ✅ 13/13 全完成, 通过 MVP-0 落地

| 状态 | 任务 | 优先级 | 文档 / 代码 | commit |
|---|---|---|---|---|
| ✅ | 创建工作项目录 | P0 | `docs/20_工作项/2026-05-31_Phase3_记忆子系统设计修订/` | — |
| ✅ | memory-design.md: 数据库选型 redb→SQLite | P0 | `qianxun-memory/src/db.rs` (8 表 schema) | `02fb2e2` |
| ✅ | memory-design.md: BM25→FTS5 全文搜索 | P0 | `qianxun-memory/src/search.rs` (BM25 via FTS5) | `02fb2e2` |
| ✅ | memory-design.md: 向量 WAL→BLOB 列 | P0 | `qianxun-memory/src/vector.rs` (VectorIndex 骨架) | `02fb2e2` |
| ✅ | memory-design.md: 展开 Consolidation | P0 | `qianxun-memory/src/consolidation.rs` (Observation → Memory 聚类) | `159f966` |
| ✅ | memory-design.md: 批处理写入 | P1 | `qianxun-memory/src/lib.rs` (spawn_blocking 异步化) | `159f966` |
| ✅ | memory-design.md: Embedding 断路器/缓存 | P2 | `qianxun-memory/src/vector.rs` (cache + 错误降级) | `159f966` |
| ✅ | memory-design.md: Session 并发协调 | P2 | `qianxun-memory/src/db.rs` (WAL + spawn_blocking) | `159f966` |
| ✅ | memory-design.md: Session 去重可配置 | P3 | `qianxun-memory/src/slot.rs` (槽位管理) | `159f966` |
| ✅ | architecture.md: Phase 边界标注 | P0 | `docs/10_事实源/架构设计.md` + `docs/30_子项目规划/` | `87b8dfb` |
| ✅ | architecture.md: AgentLoop 论证 | P2 | `docs/10_事实源/架构设计.md` (跟代码同步) | (跟代码同步) |
| ✅ | architecture.md: DB 选型决策 | P0 | `qianxun-memory/src/db.rs` + 文档引用 | `02fb2e2` |
| ✅ | architecture.md: 跨机同步限制 | P3 | `docs/30_子项目规划/04-kanban-design.md` §7.3 (v6 决策) | (后续 v6 设计回写) |
| ✅ | 文档结构清理: archive architecture.md → `docs/10_事实源/` + `docs/30_子项目规划/` | P0 | `docs/` 整体 | `87b8dfb` + 后续 |

**说明**: TODO 8 项 ⏳ 全部转 ✅, 实际由 13 项落地完成 (含 4 项 MVP-0 衍生任务). 详见 `06-mavis-执行历史.md` §4.1.

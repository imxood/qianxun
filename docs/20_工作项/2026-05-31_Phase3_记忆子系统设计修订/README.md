# 工作项: Phase 3 记忆子系统设计修订

> 状态: ✅ 已通过 MVP-0 落地, TODO 8 项已实际覆盖, 收尾 | 创建: 2026-05-31 | 收尾: 2026-06-03

## 目标

对 `architecture.md` 和 `memory-design.md` 两份设计文档进行系统性修订，重点解决：

1. **数据库选型**: redb → SQLite (rusqlite)，消除全表扫描、Schema 演进、调试三大痛点
2. **BM25 持久化问题**: 全量序列化 → SQLite FTS5 内置全文索引
3. **Consolidation 设计密度不足**: 补充完整聚类算法、回滚策略、并发协调
4. **存储性能优化**: spawn_blocking 批处理、嵌入缓存、自动 compact
5. **Phase 边界不清晰**: architecture.md 标注已实现/规划中的状态

## 关联事实源

- `docs/10_事实源/架构设计.md` — 当前实现的事实源（不受影响）
- `docs/10_事实源/memory-state.md` — 记忆子系统状态 (MVP-0 落地)
- `docs/30_子项目规划/04-kanban-design.md` §3.2 — 缺口 7 修复方案
- `docs/30_子项目规划/05-mvp-0-checklist.md` — MVP-0 详细任务清单

## 设计决策记录 (已完成, MVP-0 落地)

| 决策 | 状态 | 位置 | commit |
|---|---|---|---|
| 数据库: redb → SQLite (rusqlite bundled) | ✅ 已实现 | `qianxun-memory/src/db.rs` (8 表 schema) | `02fb2e2` |
| 全文搜索: 自建 BM25 → FTS5 | ✅ 已实现 | `qianxun-memory/src/search.rs` (BM25 via FTS5) | `02fb2e2` |
| 向量存储: WAL 追加 → SQLite BLOB 列 | ✅ 已实现 | `qianxun-memory/src/vector.rs` (VectorIndex 骨架) | `02fb2e2` |
| Consolidation 展开 | ✅ 已实现 | `qianxun-memory/src/consolidation.rs` (Observation → Memory 聚类) | `159f966` |
| 批处理写入 | ✅ 已实现 | `qianxun-memory/src/lib.rs` (spawn_blocking 异步化) | `159f966` |
| Embedding 断路器/缓存 | ✅ 已实现 | `qianxun-memory/src/vector.rs` (cache + 错误降级) | `159f966` |
| Session 并发协调 | ✅ 已实现 | `qianxun-memory/src/db.rs` (WAL + spawn_blocking) | `159f966` |
| Session 去重可配置 | ✅ 已实现 | `qianxun-memory/src/slot.rs` (槽位管理) | `159f966` |
| daemon/mod.rs: AppState.memory 从 None/in_memory 占位 → MemoryCore::open 真 SQLite | ✅ 已实现 | `qianxun/src/daemon/mod.rs:100-198` | `159f966` |
| cargo test 214/0 + clippy 0 警告 0 错误 (从 133 警告起步) | ✅ 已实现 | 全工作区 | `42e1bdd` |
| 文档落地 (CLAUDE.md +10 行 + 01-daemon.md +18 行) | ✅ 已实现 | `docs/` | `87b8dfb` |

## 完成情况 (2026-06-03)

13 项设计决策 + 1 项端到端验收 + 1 项文档落地, 全部通过 MVP-0 5 天 (4 周期) 落地, 详见 `06-mavis-执行历史.md` §4.1. TODO 8 项 ⏳ 全部转 ✅.

**关键 commit 链 (6 个)**:
- `ea7b335` — ToolRegistry builtin 13 注册
- `da04950` — skills endpoint (list_skills 真读 state.skills)
- `02fb2e2` — memory ping + 8 表 schema + FTS5 全文索引 + Vector BLOB
- `159f966` — daemon/mod.rs 三占位真初始化 (tools/skills/memory)
- `42e1bdd` — cargo test 214/0 + clippy 0/0
- `87b8dfb` — CLAUDE.md + 01-daemon.md 文档落地

## 对应 plans 决策

- **MVP-0 plan** (`.mavis/plans/plan_6ca1a0c0/`):
  - `decision-cycle1.json` — track-a (builtin 13) + track-b (skills endpoint) + track-c (memory ping) 全 accept
  - `decision-cycle2.json` — track-d (daemon/mod.rs 集成) accept
  - `decision-cycle3.json` — track-e (e2e + clippy 0) accept
  - `decision-cycle4.json` — track-f (docs) accept, plan_complete
- **MVP-0 执行 plan** (`.mavis/plans/plan-mvp0-execute.yaml`): 5 天 6 任务编排

详见 `06-mavis-执行历史.md` §4.1 MVP-0 周期详情.

## 下一步 (已全部完成)

1. ✅ 更新 `memory-design.md` — 存储层 + 全文搜索 + 向量存储 (集成到 `qianxun-memory/src/db.rs`)
2. ✅ 更新 `memory-design.md` — Consolidation 展开 (集成到 `qianxun-memory/src/consolidation.rs`)
3. ✅ 更新 `memory-design.md` — 性能优化 (spawn_blocking 异步化)
4. ✅ 更新 `architecture.md` — Phase 边界 + 补充决策 (迁到 `docs/10_事实源/架构设计.md` + `docs/30_子项目规划/`)
5. ✅ 文档结构清理 (`docs/10_事实源/` + `docs/30_子项目规划/` 落地)

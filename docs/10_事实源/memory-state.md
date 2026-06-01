---
状态: 生效
适用范围: qianxun-memory crate
最后更新: 2026-06-01
---

# Memory 子系统状态

## 一句话摘要

MemoryObserver trait + MemoryCore(SQLite + FTS5 + 自动同步 trigger) 闭环完成, observe → FTS search → build_context 集成测试覆盖, 异步路径用 spawn_blocking 隔离.

## 源文件清单

- `qianxun-memory/src/lib.rs` — MemoryCore 主入口, 实现 observe / remember / build_context / search / session_start / session_end
- `qianxun-memory/src/db.rs` — SQLite schema (8 表含 obs/obs_fts/sessions/tags) + **3 个 FTS5 同步 trigger**
- `qianxun-memory/src/types.rs` — 数据类型 (MemoryRecord/MemorySearchResult/MemoryStats)
- `qianxun-memory/src/search.rs` — HybridSearch (BM25 + vector 权重配置)
- `qianxun-memory/src/vector.rs` — VectorIndex 骨架
- `qianxun-memory/src/consolidation.rs` — 压缩/去重/合并逻辑
- `qianxun-memory/src/compressor.rs` — 文本压缩 + 合成 observation
- `qianxun-memory/src/privacy.rs` — 隐私清洗
- `qianxun-memory/src/slot.rs` — 槽位管理

## 当前状态

| 子模块 | 状态 | 说明 |
|--------|------|------|
| observe | ✅ | 写入 obs 表, 同步触发 FTS trigger; session_id 来自 current_session, 不再硬编码 "global" |
| remember | ✅ | 高优先级记忆写入, **char 边界安全截断** (中文不再 panic) |
| build_context | ✅ | FTS5 搜索 + 格式化上下文, 写后即能命中 |
| search | ✅ | FTS5 BM25 真实返回, 接 build_context 闭环 |
| session_start | ✅ | 写入 sessions 表 (id/project/cwd/started_at/status=active) |
| session_end | ✅ | 更新 sessions 表 (ended_at + status=ended), 清空 current_session |
| FTS 同步 | ✅ | **3 个 trigger** (obs_ai_fts / obs_ad_fts / obs_au_fts) 维护 obs_fts ↔ observations |
| consolidation | ✅ | 压缩逻辑完整 |
| HybridSearch | 🔧 | BM25 + vector 权重配置存在, 未与 MemoryCore 集成 (daemon 阶段再接) |
| 异步路径 | ✅ | 全部 SQLite 操作包 `tokio::task::spawn_blocking`, 不阻塞 reactor |

## 集成测试覆盖

`qianxun-memory/src/lib.rs::tests` 包含 8 个集成测试, 18 passed / 0 failed:

- `truncate_to_chars_handles_cjk` — 中文按 char 截断不 panic
- `remember_with_chinese_title_does_not_panic` — 300 字节中文字符串 remember 成功
- `observe_writes_observation_with_real_session_id` — session_id 是真实值, 非 "global"
- `observe_without_session_is_dropped_silently` — 无 active session 时 no-op
- `fts_trigger_indexes_new_observations` — INSERT 自动同步到 obs_fts
- `build_context_returns_recent_observations_and_memories` — FTS 搜索 + memories 列出
- `session_lifecycle_writes_sessions_table` — start / end 真实写入/更新 sessions
- `multi_session_observations_are_isolated_by_session_id` — 独立 db session 隔离

## 当前已知缺口

- `compressor::compress_read` / `compress_write` / `compress_edit` 不把 `tool_output` 写入 narrative
  → FTS 只能命中 path 和 command, 搜不到工具输出中的关键词
  → 影响: 搜索召回率受限; **不影响功能正确性**
  → 后续: 在 compressor 收 post output 摘要进 narrative (类似 compress_terminal 的 6 行截断)
- `HybridSearch` 与 `MemoryCore.search` 还未集成
  → 当前 search 直接调 FTS5, 不走 hybrid 路径
  → 影响: 无向量召回; 对精确 token 匹配够用, 对语义相似度召回不够
  → 后续: daemon 阶段接入 (Phase D/E)
- `current_session` 进程内单值 (std::sync::Mutex<Option<CurrentSession>>)
  → 同一进程多 session 并发 observe 会用最后一次 session_start 的 session_id
  → 影响: 当前 Daemon 模式是单 session 进程, 无问题
  → 后续: 真正多 session 并发时, 改成把 session_id 作为 observe 参数或按 session 分库
- `consolidation::run_consolidation` 仍是同步阻塞, 未接 session_end
  → 当前: 手动调用 (需外部触发)
  → 后续: 在 session_end 中自动触发, 或后台定时任务

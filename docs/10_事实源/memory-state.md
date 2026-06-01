---
状态: 生效
适用范围: qianxun-memory crate
最后更新: 2026-06-01
---

# Memory 子系统状态

## 一句话摘要
MemoryObserver trait + MemoryCore(SQLite+FTS5) 可用，但检索闭环和 session 生命周期不完整。

## 源文件清单
-  — MemoryCore 主入口，实现 observe/remember/build_context/search
-  — SQLite schema（8 表含 obs/obs_fts/sessions/tags）
-  — 数据类型（MemoryRecord/MemorySearchResult/MemoryStats）
-  — BM25 + vector 混合搜索骨架
-  — VectorIndex 骨架
-  — 压缩/去重/合并逻辑
-  — 文本压缩
-  — 隐私清洗
-  — 槽位管理

## 当前状态

| 子模块 | 状态 | 说明 |
|--------|------|------|
| observe | ✅ | 写入 obs 表 |
| remember | ✅ | 高优先级记忆写入 |
| build_context | ✅ | FTS5 搜索 + 格式化上下文 |
| search | 🔧 | 返回 Vec 但始终为空 |
| session_start | 🔧 | 空实现，不写入 sessions 表 |
| session_end | 🔧 | 空实现 |
| FTS 同步 | 🔧 | observe 不自动写入 obs_fts |
| consolidation | ✅ | 压缩逻辑完整 |
| HybridSearch | 🔧 | BM25 + vector 权重配置存在但未接入 |

## 已知缺口
- search() 不返回结果
- FTS 表与 obs 表不同步
- 中文标题可能因字节截断 panic
- 无 dedicated DB worker，同步锁可能阻塞异步上下文

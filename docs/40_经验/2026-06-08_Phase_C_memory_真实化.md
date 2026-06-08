# Phase C 经验: Memory 真实化 3 件套

> 日期: 2026-06-08
> 范围: qianxun-memory crate 内部 3 处补全 (compressor 收 tool output, HybridSearch 集成, consolidation 接到 session_end)
> 状态: ✅ 全部 251/0 测试 pass, clippy 0 warning

## TL;DR

| 项 | 改前 | 改后 | 收益 |
|---|---|---|---|
| **compressor** | `compress_read/write/edit/search` 只用 `input` (path/query), `is_post` 参数被忽略 | PostToolUse 时把 `tool_output` 收进 narrative (首尾各 3 行截断) | FTS5 召回率 ↑, 之前搜不到工具输出里的关键词 |
| **HybridSearch** | `HybridSearch::new(db: Arc<Connection>)` 重复持 db, `search()` 跟 MemoryCore.search 平行走 FTS5 | 改为 stateless `HybridSearch::new()` + `search(conn: &Connection, ...)`, MemoryCore.search 委派给它 | session dedup 生效 (max 3 per session), 未来 vector search 集成入口 |
| **consolidation** | `consolidation::run_consolidation(db: &Arc<Connection>, sid)` 是裸函数, 没人调 | 加 `run_consolidation_locked(conn, sid)` (已加锁) 入口, `session_end()` 同步触发 | 跨 session 自动聚类, 长期记忆形成闭环 |

外加: **修 pre-existing bug** `merge_similar_clusters` 迭代中 mutate vec 导致 i 越界 panic.

## 关键决策

### 1. compressor: PostToolUse 时合并 output, 跟 terminal 行为对齐

**现状**:
- `compress_terminal` 已经把 `output` 收进 narrative (首尾 3 行截断)
- `compress_read/write/edit/search/default` 接受 `is_post` 但没用它, narrative 只有 path/command

**改后**:
```rust
// compress_read (其他 4 个同模式)
let narrative = match (is_post, output) {
    (true, Some(o)) if !o.is_empty() => {
        format!("读取了文件 {path}\n输出摘要:\n{}", trim_output_for_narrative(o))
    }
    _ => format!("读取了文件 {path}"),
};
```

**为什么不把整个 output 都收进 narrative**:
- 输出可能几 MB (大文件读 / 命令列目录)
- FTS5 indexing + token 预算都会被爆
- 首尾 3 行截断跟 `compress_terminal` 一致 (业务上 "开头 + 结尾" 摘要最有信息密度)

**为什么 is_post=true 才收**:
- PreToolUse 没有 output (还没跑)
- PostToolUse 是工具执行完, output 才有意义
- 跟原 `compress_terminal` 行为对齐 (`is_post` 之前也是参数但没用)

### 2. HybridSearch: 改 stateless, 跟 MemoryCore 的 Mutex 兼容

**现状**:
```rust
pub struct HybridSearch {
    db: Arc<Connection>,  // 重复持有
    vector: Arc<RwLock<Option<VectorIndex>>>,
    ...
}
```

`MemoryCore.db: Arc<Mutex<Connection>>`, HybridSearch 拿 `Arc<Connection>` 就死锁或者双重持锁.

**改后**:
```rust
pub struct HybridSearch {
    vector: Arc<RwLock<Option<VectorIndex>>>,  // 只 in-memory state 留 struct
    bm25_weight: f64,
    vector_weight: f64,
}

impl HybridSearch {
    pub fn search(&self, conn: &Connection, query, limit) -> Vec<SearchResult> {
        // conn 由调用方提供 (已加锁)
    }
}

// MemoryCore.search 委派:
fn search_sync(db, query, limit) {
    let conn = db.lock()?;
    let hybrid = HybridSearch::new();  // 无 db 参数
    Ok(hybrid.search(&conn, query, limit))
}
```

**为什么 stateless 设计**:
- 持 db 字段就要求 caller 把 db 给 HybridSearch, 跟 MemoryCore 的 Mutex 互斥管理冲突
- Stateless: caller 已经持有锁, 直接传 `&Connection` 给 `search()`, 不重复持锁, 也不死锁
- 唯一状态是 vector index, 留 RwLock (Phase 后续接 vector 时再加)

**收益**:
- MemoryCore.search 自动获得 session dedup (max 3 per session, 之前没有)
- 未来加 vector search 时, HybridSearch.set_vector_index() 接一个, MemoryCore.search 自动用

### 3. consolidation: 接到 session_end 闭环

**现状**:
- `consolidation::run_consolidation(db, sid)` 是裸函数, 没人调
- 聚类逻辑 (Jaccard > 0.5, avg importance >= 6 || size >= 3) 完整, 但只在 manual call 时跑

**改后**:
- 加 `run_consolidation_locked(conn, sid)` 接受已加锁的 `&Connection`
- `run_consolidation(db: &Arc<Mutex<Connection>>, sid)` 锁 + 调 locked 版
- `MemoryCore.session_end()` 在 UPDATE sessions 的 spawn_blocking 闭包内同步调 `run_consolidation_locked`

```rust
// MemoryCore.session_end():
let result = tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
    let conn = db.lock().map_err(...)?;
    conn.execute("UPDATE sessions SET ended_at = ?1 ...", ...)?;
    // Phase C 收尾: 同步触发 consolidation (同一锁块内, 避免并发写)
    consolidation::run_consolidation_locked(&conn, &sid);
    Ok(())
}).await;
```

**为什么用 `_locked` 版本 (不开新 spawn_blocking)**:
- 已经在 spawn_blocking 闭包内, 闭包已经在 blocking 线程上跑
- 再 `spawn_blocking` 嵌套会导致: spawn_blocking 持锁, 内部 spawn_blocking 也想拿同一锁 → 死锁 (tokio blocking pool 线程数有限)
- 改成同步函数调, 走完就释放锁

**为什么在 session_end 而不是 session_start 或 background timer**:
- session_end 语义最自然: 会话结束, 整理这段时间的 observation → 长期 memory
- 比 background timer 简单 (不用维护 timer handle)
- session_start 时没有 observation, 调 consolidation 也没东西处理

### 4. 顺手修 pre-existing bug: `merge_similar_clusters` 越界

**症状**:
新加的测试 `session_end_triggers_consolidation` 触发了 panic:
```
thread 'tokio-rt-worker' panicked at qianxun-memory/src/consolidation.rs:207:47
```

**根因**:
原实现:
```rust
for i in (0..clusters.len()).rev() {       // 固定 range
    for j in (0..clusters.len()).rev() {    // 固定 range
        if sim > 0.3 {
            let other = clusters.remove(j); // ← 改了 clusters.len()
            let target = &mut clusters[i];  // ← i 可能越界
```

当 i > j 时, 移除 j (i 不变但 j 后元素往前 shift). 下次 `clusters[i]` 访问还是用 i, 但 `clusters.len()` 已经变, 索引可能越界.

**修法**:
```rust
fn merge_similar_clusters(clusters: &mut Vec<Cluster>) {
    let mut i = 0;
    while i < clusters.len() {
        let mut j = i + 1;
        while j < clusters.len() {
            let sim = jaccard_similarity(&clusters[i].concepts, &clusters[j].concepts);
            if sim > 0.3 {
                let other = clusters.remove(j);  // j 位置被新元素占
                clusters[i].concepts.extend(other.concepts);
                clusters[i].observations.extend(other.observations);
                // j 不递增, 新元素 (原 j+1) 现在在 j 位置
            } else {
                j += 1;
            }
        }
        i += 1;
    }
}
```

**为什么原来没被发现**:
- 原 consolidation 没人调, 走不到这里
- 我新加 session_end → consolidation 触发路径, 才暴露这个 latent bug
- 是"修一个加一个就 break 一个"的典型 — 加新 caller 把沉寂 bug 唤醒

**教训**:
- 任何 public API 不被调用 ≠ 它没问题, 是没机会暴露问题
- 加新 caller 时, 配套加 mock scenario 跑全套链路

## 踩过的坑

### 1. `search.rs` 改完没删旧 impl block, 留下多个 `new` 定义

**症状**:
```
error[E0034]: multiple applicable items in scope
   --> qianxun-memory\src\lib.rs:485:40
    |
485 |     let hybrid = search::HybridSearch::new();
    |                                        ^^^ multiple `new` found
```

**根因**:
我把 `HybridSearch::new()` (stateless) 写到文件顶部, 但旧 `impl HybridSearch { pub fn new(db: Arc<Connection>) }` 在 150+ 行没删. Rust 不允许两个同名关联函数, 编译错.

**修法**:
用 Write 工具整个文件重写, 不是 Edit 局部. 局部 Edit 在文件长 + 多处重复时容易漏.

**教训**:
- 改大型 struct 重构时, 用 Write 全文件覆盖比 Edit 局部更稳
- 改完后立即 `cargo check`, 不要等全部改完再 check

### 2. `compressor_includes_tool_output_in_narrative_for_post_hook` 测试一开始搜不到

**症状**:
测试期望 FTS5 搜 `magic_keyword_xyz42` 能命中. 一开始 `tool_output` 是字符串 `"magic_keyword_xyz42 from tool output"`, 但 compressor 还没改完, narrative 里只有 path, FTS 搜不到.

**根因**:
- 改 compressor 之前先写了测试, 但 compressor 改动晚于测试, 测试时 compressor 还没合并 output
- 实际上我先改 compressor 写完就跑了, 但 `is_post=true` 这个条件没生效, narrative 还是只有 path

**修法**:
- 检查 `compress_read` 的 narrative 构造逻辑
- 改完后 `cargo test` 立即验证 narrative 真的包含 `magic_keyword_xyz42`

**教训**:
- 写完 compressor → 立即写测试 → 立即跑测试 → 验证 narrative 内容
- 不要累积多个改动一起测, 增量验证

### 3. 嵌套 spawn_blocking 死锁风险

**症状**:
设计 consolidation 集成时, 我先想到的是 `consolidation::run_consolidation(db: &Arc<Mutex<Connection>>, sid)`, 在 `session_end` 的 spawn_blocking 内调它, 它内部又 spawn_blocking 拿锁.

**根因**:
- session_end 的 spawn_blocking 持锁 + 调 consolidation
- consolidation 内部再 spawn_blocking 拿同一个锁 → 死锁
- tokio blocking pool 线程数有限, 第二个 spawn_blocking 等待第一个, 第一个等锁释放

**修法**:
- `run_consolidation_locked(conn: &Connection, sid)` 接受已加锁 conn, 同步执行
- 已经在 spawn_blocking 内的 caller 调 locked 版, 不开新 spawn
- 双轨 API: `run_consolidation(db, sid)` 锁 + 调 locked, 给锁外 caller 用; `run_consolidation_locked(conn, sid)` 给锁内 caller 用

**教训**:
- 任何持锁的代码路径, 子调用不要再 spawn_blocking
- 拆 `_locked` 版本是常见模式: 锁内 vs 锁外两套入口, 业务代码选合适的

## 验收

| 项 | 状态 |
|---|---|
| `cargo check --workspace` | ✅ 0 错 |
| `cargo test --workspace` | ✅ 251 passed (148 + 34 + 5 + 20 + 44) |
| `cargo clippy --workspace --all-targets` | ✅ 0 warning |
| compressor 收 tool output | ✅ 新测试 `compressor_includes_tool_output_in_narrative_for_post_hook` |
| HybridSearch 集成 | ✅ 1 处旧 impl 删, 新 stateless impl, MemoryCore.search 委派 |
| consolidation 接到 session_end | ✅ 新测试 `session_end_triggers_consolidation` (含 1 处 pre-existing bug 修) |
| 修 pre-existing bug | ✅ `merge_similar_clusters` 改 while 循环 + j 跟随 shift |

## 文件清单

**新增/重写 (5 文件)**:
- `qianxun-memory/src/compressor.rs` — 5 个 compress_* 函数都收 tool_output 进 narrative + `trim_output_for_narrative` helper
- `qianxun-memory/src/search.rs` — 整个文件重写为 stateless HybridSearch (138 → 137 行, 结构重排)
- `qianxun-memory/src/consolidation.rs` — 加 `run_consolidation` 锁外 + `run_consolidation_locked` 锁内, 修 `merge_similar_clusters` 越界 bug
- `qianxun-memory/src/lib.rs` — `search_sync` 委派 HybridSearch, `session_end` 调 `run_consolidation_locked`, 加 2 测试
- `qianxun/src/tui/mod.rs` — 删 1 处 unused import (clippy 警告)

**测试新增 (2 个)**:
- `qianxun-memory/src/lib.rs::compressor_includes_tool_output_in_narrative_for_post_hook` — 验证 PostToolUse observation 包含 tool output 关键词
- `qianxun-memory/src/lib.rs::session_end_triggers_consolidation` — 验证 session_end 后 memories 表有 pattern 类型 memory

## 范围外 follow-up

1. **vector search 集成**: HybridSearch 已经留 `vector: Arc<RwLock<Option<VectorIndex>>>` + `bm25_weight` / `vector_weight`, 未来加 embedding 生成器 + 真实接 vector search
2. **Multi-session 并发** (`current_session`): 当前 `current_session: Arc<Mutex<Option<CurrentSession>>>` 是单值, 多 session 并发 observe 会用最后 start 的 session_id. 改造方向: `observe_with_session(session_id, ...)` 显式传, 或改成 `HashMap<session_id, ...>`. agent_host 已经能多 session, 但 MemoryCore 接口还是单 session. Phase 后续评估
3. **compressor 真实 LLM 摘要**: 当前是 heuristic 启发式, 未来可加 LLM 二次摘要 (更准但慢)
4. **consolidation 触发频率**: 现在每次 session_end 都跑, 长 session 跑得慢. 未来加阈值 (observation 数 > N 才触发) 或后台 timer

## 关联

- 04c-qianxun-runtime-extraction.md (前置: RuntimeState 抽离)
- Phase A 经验 (前置: 5 binary 入口切 RuntimeState)
- `docs/10_事实源/memory-state.md` (更新: 状态描述同步)
- `docs/10_事实源/memory-state.md` 旧版本提到的 3 个缺口 (compressor 收 output / HybridSearch 集成 / consolidation 接 session_end) → Phase C 全部关闭

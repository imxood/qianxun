# 千寻记忆子系统设计

> 版本: 0.3 | 更新: 2026-06-01 | 状态: 已实现
>
> qianxun-memory crate 已创建：8 张 SQLite 表 + FTS5 + 合成压缩 + 隐私清洗 + 向量索引 + 工作插槽 + Consolidation 管线

---


### 1.1 文件结构

```
qianxun-memory/src/
├── lib.rs              # crate 入口 + MemoryCore（observe/remember/search）
├── types.rs            # Session, Observation, Memory, MemorySlot
├── db.rs               # SQLite 连接、表定义、迁移
├── compressor.rs       # 合成压缩算法
├── privacy.rs          # 隐私清洗（6 类敏感信息）
├── search.rs           # HybridSearch（FTS5 + 向量 + RRF）
├── vector.rs           # VectorIndex + BLOB 列持久化
├── slot.rs             # SlotManager（工作记忆插槽）
└── consolidation.rs    # 聚类 + Memory 生成 + 版本升级
```

> 注: embedding.rs（向量化嵌入层）、eviction.rs（TTL 淘汰）和 file_store.rs（本地快照同步）为设计规划模块，尚未实现。


## 1. 设计目标

### 核心理念

> **千寻不需要重新学习你已经做过的事。**

| 目标 | 说明 |
|---|---|
| **自动捕获** | Agent 每次 Tool 调用、错误、决策自动记录为 Observation |
| **零开销压缩** | 默认不调 LLM，启发式提取结构化记忆体，0 token 消耗 |
| **即时可检索** | Observation 压缩后立即进入搜索索引，后续提问即可召回 |
| **跨会话持久化** | Memory 跨会话保留，6 种类型分类 |
| **语义搜索** | BM25 + 向量混合检索，RRF 融合排序 |
| **项目隔离** | 不同工作区记忆隔离，搜索时自动过滤 |
| **零外部服务** | 全本地存储 |

### 非目标

- 知识图谱（Phase 3 后期评估）
- 团队协作记忆

---

## 2. 记忆模型

### 2.1 数据流

```mermaid
stateDiagram-v2
    [*] --> Raw: Agent 触发 Hook
    Raw --> Compressed: 合成压缩（0 LLM）
    Compressed --> Indexed: 加入 FTS5 + Vector（混合索引）
    Indexed --> Memory: Consolidation 周期
    Indexed --> [*]: Eviction 清理
    Memory --> Memory: 版本升级（Jaccard > 0.7）
    Memory --> [*]: TTL 过期 / 手动 forget
```

### 2.2 核心数据结构

```rust
// === memory/types.rs

/// 会话
pub struct Session {
    pub id: SessionId,
    pub project: String,
    pub cwd: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: SessionStatus,
    pub observation_count: u32,
    pub model: Option<String>,
    pub summary: Option<String>,
}

/// 原始观测 — Tool 调用的原始记录（可选，仅显式启用时保留）
pub struct RawObservation {
    pub id: ObsId,
    pub session_id: SessionId,
    pub timestamp: DateTime<Utc>,
    pub hook_type: HookType,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    pub tool_output: Option<Value>,
    pub user_prompt: Option<String>,
    pub assistant_response: Option<String>,
}

/// 压缩后的观测 — 结构化、可检索
pub struct Observation {
    pub id: ObsId,
    pub session_id: SessionId,
    pub timestamp: DateTime<Utc>,
    pub obs_type: ObservationType,
    pub title: String,
    pub subtitle: Option<String>,
    pub facts: Vec<String>,
    pub narrative: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub importance: u8,         // 1–10
    pub confidence: Option<f64>,
}

/// 跨会话持久记忆
pub struct Memory {
    pub id: MemoryId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub mem_type: MemoryType,
    pub title: String,
    pub content: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub strength: u8,
    pub version: u32,
    pub parent_id: Option<MemoryId>,
    pub is_latest: bool,
    pub forget_after: Option<DateTime<Utc>>,
    pub project: Option<String>,
    pub access_count: u64,
    pub last_accessed_at: Option<DateTime<Utc>>,
}

/// 工作记忆插槽
pub struct MemorySlot {
    pub label: String,
    pub content: String,
    pub size_limit: usize,
    pub description: String,
    pub pinned: bool,
    pub scope: SlotScope,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

---

## 3. 存储层 — SQLite

### 3.1 选型决策

| 候选 | 结论 | 原因 |
|---|---|---|
| **redb** (当前) | ❌ 替换 | KV 模式导致结构化查询需全表扫描；无 Schema 迁移；无调试工具 |
| **sled** | ❌ 不选 | 同 redb 的 KV 限制，且项目维护停滞 |
| **DuckDB** | ❌ 不选 | 列存擅长 OLAP 聚合，千寻 memory 是 OLTP 点查，命中劣势 |
| **libsql** | ⚠️ 未来 | 多机同步有吸引力，但 pre-1.0 API 变动风险 > 收益 |
| **SQLite** | ✅ 选定 | OLTP 索引查询、FTS5 全文搜索、Schema 迁移、sqlite3 CLI 调试 |

千寻 memory 的数据访问模式判定为 **OLTP**（点查、小范围扫描、单行写入）。SQLite 在所有嵌入式候选中最匹配。

**绑定方式**: `rusqlite` bundled 模式（编译时嵌入 sqlite3.c），无需系统预装 SQLite。首次构建增加 ~60-120s，二进制增加 ~1MB。

### 3.2 表定义

```sql
-- === 会话
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    cwd TEXT NOT NULL,
    started_at TEXT NOT NULL,         -- ISO 8601
    ended_at TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    observation_count INTEGER NOT NULL DEFAULT 0,
    model TEXT,
    summary TEXT
);

-- === 压缩后的观测
CREATE TABLE observations (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    timestamp TEXT NOT NULL,
    data TEXT NOT NULL,               -- JSON: 含 obs_type/title/subtitle/facts/
                                      --   narrative/concepts/files/importance/confidence
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_obs_session ON observations(session_id);
CREATE INDEX idx_obs_timestamp ON observations(timestamp);
-- 半结构化 JSON 索引（用于批量过滤）
CREATE INDEX idx_obs_type ON observations(json_extract(data, '$.obs_type'));

-- === FTS5 全文索引（取代自建 BM25 HashMap）
-- content='observations' + content_rowid 实现增量同步
CREATE VIRTUAL TABLE obs_fts USING fts5(
    title, narrative, facts, concepts, files,
    content='observations',
    content_rowid='rowid',
    tokenize='unicode61 tokenchars'
);
-- 注意：中文 tokenize 需要额外配置（unicode61 + tokenchars 对 CJK 做字符级切分）
-- 可选替换：使用 jieba 分词器作为 FTS5 tokenizer（见 4.2 节）

-- === 向量索引（取代 WAL 追加 + 启动重建）
CREATE TABLE observation_vectors (
    obs_id TEXT PRIMARY KEY REFERENCES observations(id),
    embedding BLOB NOT NULL,          -- f32 字节序列，384/768/1024 维
    dimensions INTEGER NOT NULL,
    model TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- === 跨会话持久记忆
CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    mem_type TEXT NOT NULL,
    data TEXT NOT NULL,               -- JSON: 含 title/content/concepts/files/
                                      --   strength/version/parent_id/is_latest/
                                      --   forget_after/project/access_count/last_accessed_at
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_memories_type ON memories(mem_type);
CREATE INDEX idx_memories_project ON memories(json_extract(data, '$.project'));

-- === 会话摘要
CREATE TABLE session_summaries (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    summary TEXT NOT NULL,
    model TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_summary_session ON session_summaries(session_id);

-- === 工作记忆插槽
CREATE TABLE slots (
    label TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    size_limit INTEGER NOT NULL DEFAULT 2000,
    description TEXT NOT NULL DEFAULT '',
    pinned INTEGER NOT NULL DEFAULT 0,  -- boolean
    scope TEXT NOT NULL DEFAULT 'project',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- === 原始观测（可选，默认不启用）
CREATE TABLE raw_observations (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    timestamp TEXT NOT NULL,
    hook_type TEXT NOT NULL,
    tool_name TEXT,
    tool_input TEXT,                  -- JSON
    tool_output TEXT,                 -- JSON（可能包含大量内容）
    user_prompt TEXT,
    assistant_response TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_raw_session ON raw_observations(session_id);
```

### 3.3 key vs JSON 的边界

| 情况 | 处理 |
|---|---|
| 索引查询用的字段 | 独立列（session_id, timestamp, obs_type, mem_type） |
| Schema 不稳定的字段 | JSON 存入 data 列 |
| 向量数据 | 独立表 + BLOB 列（二进制高效存储） |
| 全文搜索内容 | FTS5 虚拟表管理 |

每次启动时，如果 data 列 JSON 新增字段 → 不需要 migration，代码解析时缺少字段给默认值即可。
如果索引查询字段变更 → 需要一次 `ALTER TABLE ADD COLUMN` + 渐进回填。

### 3.4 存储结构

```
~/.qianxun/
├── mem.db               # SQLite 数据库（全部数据）
├── mem.db-wal           # SQLite WAL（运行时，WAL 模式）
├── mem.db-shm           # SQLite 共享内存（运行时）
└── memory/              # 可读记忆文件（快照，非权威源）
    ├── architecture/    # 架构决策
    ├── pattern/         # 开发模式
    ├── preference/      # 用户偏好
    ├── bug/             # 缺陷记录
    ├── workflow/        # 工作流
    ├── fact/            # 事实性知识
    └── slots/           # 工作记忆插槽
```

**.md 文件只读不写**（除非用户手动编辑），写入始终通过 SQLite。.md 文件由 Agent 工具 `memory_remember` 写入，作为快照而非权威源。

---

## 4. 索引层 — FTS5 + 向量检索

### 4.1 全文搜索（取代自建 BM25）

**设计变更**：v0.1 的自建 BM25 HashMap（每 5s 全量序列化到 redb）替换为 SQLite FTS5。

| v0.1 (redb + BM25) | v0.2 (SQLite + FTS5) | 改善 |
|---|---|---|
| `SearchIndex { inverted: HashMap }` 全量序列化 | `INSERT INTO obs_fts` 增量同步 | 持久化 O(1) 而非 O(n) |
| 5s 防抖写 1 个大 blob | FTS5 自动管理索引文件 | 消除写锁长时间持有 |
| 启动时反序列化全量倒排表 | 启动时无需加载 FTS5 内容 | 即时启动 |
| 中文需要 jieba-rs | `unicode61 tokenchars` + 可选 jieba | 简化依赖 |
| 同义词需要手写表 | SQL 查询时通过 UNION ALL 扩展 | 灵活 |

#### 4.1.1 搜索 API

```sql
-- 基础 FTS5 搜索
SELECT o.id, o.session_id, o.data
FROM obs_fts f
JOIN observations o ON o.rowid = f.rowid
WHERE obs_fts MATCH ?
ORDER BY rank
LIMIT ?;

-- 带 session_id 过滤（项目隔离）
SELECT o.id, o.session_id, o.data
FROM obs_fts f
JOIN observations o ON o.rowid = f.rowid
WHERE obs_fts MATCH ?
  AND o.session_id IN (SELECT id FROM sessions WHERE project = ?)
ORDER BY rank
LIMIT ?;
```

FTS5 的 `rank` 内置 BM25 分数（默认 bm25），无需自己实现排序。

#### 4.1.2 FTS5 内容同步

FTS5 使用 `content='observations'` + `content_rowid='rowid'` 模式：

```
INSERT INTO observations(...) → 自动 INSERT INTO obs_fts(...)
DELETE FROM observations(...) → 自动 DELETE FROM obs_fts(...)
```

无需手动同步。FTS5 不复制内容到全文索引内部，而是通过 rowid 回查 observations 表。

### 4.2 中文分词策略

| 方案 | 延迟 | 召回 | 实现成本 |
|---|---|---|---|
| `unicode61 tokenchars` | 0.0ms | 中（字符级） | 内置 |
| jieba-rs 实现 FTS5 tokenizer | ~0.5ms | 高（词级） | 需要 C 封装 |
| 搜索时同时查 unicode61 + jieba 扩展 | ~1ms | 最高 | 双索引维护 |

**建议**：Phase 3 先用 `unicode61 tokenchars` 发布（零成本）。如果中文召回率不足，改为 jieba-rs 实现自定义 tokenizer。FTS5 支持运行时 `rebuild` 重建索引。

```sql
-- 如果需要重建全文索引
INSERT INTO obs_fts(obs_fts) VALUES('rebuild');
```

### 4.3 向量索引（BLOB 列取代 WAL）

**设计变更**：v0.1 的 `VectorIndex { vectors: HashMap }` + WAL 追加持久化，替换为 SQLite BLOB 列。

```rust
pub struct VectorIndex {
    // 运行时：内存 HashMap（与 v0.1 相同）
    vectors: HashMap<String, VectorEntry>,
    dimensions: usize,
    
    // 持久化：不再是 WAL 文件，而是 SQLite observation_vectors 表
    // 启动时：SELECT obs_id, embedding FROM observation_vectors → 重建 HashMap
}
```

**持久化流程**：

```
写入：
  vector.add(obs_id, vec)
    → vectors.insert(obs_id, VectorEntry { vec, model })
    → spawn_blocking: INSERT OR REPLACE INTO observation_vectors(obs_id, embedding, dimensions, model, updated_at)
  
启动加载：
  let stmt = conn.prepare("SELECT obs_id, embedding, dimensions FROM observation_vectors")?;
  // 一次扫描，~10000 行 × 1.5KB = 15MB，顺序读取 < 10ms
  for row in stmt.query([])? {
      vectors.insert(obs_id, VectorEntry { vec: deserialize_f32_slice(embedding) });
  }
```

**对比 v0.1 WAL 模式**：

| v0.1 (Vector WAL) | v0.2 (BLOB 列) |
|---|---|
| 每条 append WAL + 启动遍历全部 WAL 重建 | 每条 INSERT OR REPLACE + 启动一次 SELECT |
| WAL 可以膨胀到 ~1.3GB/年 | 每行 ~1.5KB，自动管理 |
| compact_vector_wal() 需要手动触发 | 无需 compact，DELETE 自动释放空间 |
| 没有外键约束，孤儿向量 | 外键 `REFERENCES observations(id)` 自动级联 |

### 4.4 混合搜索 + RRF 融合

```rust
pub struct HybridSearch {
    db: Arc<Connection>,                // SQLite 连接（spawn_blocking 中读写）
    vector: Arc<RwLock<VectorIndex>>,   // 内存向量索引，微秒级读
    embedding: Option<Box<dyn EmbeddingProvider>>,
    cache: Option<LruCache<String, Vec<f32>>>,  // 嵌入结果缓存（新增）
    bm25_weight: f64,                   // 默认 0.4
    vector_weight: f64,                 // 默认 0.6
}
```

**搜索流程**：

```
1. BM25 搜索（FTS5，SQL → spawn_blocking）
     → 如果 embedding 不可用，直接返回 FTS5 结果（降级）
     → 如果 embedding 可用，继续

2. 向量搜索（可选，有断路器保护）
     ┌─ check cache（LRU cache，kv 毫秒级）
     ├─ embedding 可用 → embed query → VectorIndex 搜索
     └─ embedding 失败（超时/错误）→ 跳过，仅返回 BM25 结果

3. RRF 融合（K=60）

4. Session 去重（max N per session，N 可配置，默认 3）

5. 加载完整 Observation（spawn_blocking 读 observations 表）

6. 按 filter 过滤 → 排序 → 截断
```

**新增：Embedding 断路器**

```rust
impl EmbeddingProvider {
    /// 连续失败 N 次后打开断路器，跳过嵌入 N 秒
    pub fn with_circuit_breaker(self, threshold: u32, cooldown: Duration) -> Self;
}
```

默认配置：连续 3 次失败 → 断路器打开 → 5 分钟冷却 → 半开尝试恢复。

这种模式下 `HybridSearch` 对外始终可用：FTS5 是主路径，向量搜索是增强。

**新增：嵌入结果缓存**

```
cache: LruCache<String, Vec<f32>>   // 容量 256，LRU 淘汰
命中率预期：同一 session 中同一个 query 多次搜索（或相似 query）可命中
```

### 4.5 并发安全与锁策略

```rust
pub struct HybridSearch {
    db: Arc<Connection>,                   // SQLite 连接，WAL 模式支持并发读写
    vector: Arc<RwLock<VectorIndex>>,      // std::sync::RwLock，微秒级操作
    embedding: Option<Box<dyn EmbeddingProvider>>,
    cache: Arc<RwLock<Option<LruCache<...>>>>, // 嵌入缓存（新增）
}
```

| 数据 | 锁类型 | 理由 |
|---|---|---|
| VectorIndex HashMap | `std::sync::RwLock` | 临界区 < 0.01ms，直接在 async 中安全 |
| 嵌入缓存 LruCache | `std::sync::RwLock` | 临界区 < 0.01ms |
| SQLite 连接 | **`tokio::task::spawn_blocking`** | SQL 查询 1-10ms，必须移出异步线程 |
| MemoryCore 共享 | `Arc` | 只读 |

**核心钩子函数**（使用 SQLite）:

```rust
// ✓ 正确：VectorIndex 操作（< 0.01ms），直接在 async 中
self.vector.write().unwrap().add(&obs.id, vec);

// ✓ 正确：SQLite 写走 spawn_blocking
let db = self.db.clone();
tokio::task::spawn_blocking(move || {
    let tx = db.transaction()?;
    tx.execute("INSERT INTO observations (id, session_id, timestamp, data) VALUES (?1, ?2, ?3, ?4)",
        params![obs.id, obs.session_id, obs.timestamp.to_rfc3339(), serde_json::to_string(&obs.data)?])?;
    tx.execute("INSERT INTO obs_fts (rowid, title, narrative, facts, concepts, files)
                VALUES (last_insert_rowid(), ?1, ?2, ?3, ?4, ?5)",
        params![obs.title, obs.narrative, ...])?;  // FTS5 自动与 observations 表关联
    tx.commit()?;
    Ok(())
}).await??;
```

---

## 5. 捕获管线

### 5.1 完整流程

```mermaid
sequenceDiagram
    participant AL as AgentLoop
    participant OB as observe()
    participant CP as Compressor
    participant DB as SQLite
    participant IX as Indexer

    AL->>OB: observe(PostToolUse, input, output)
    OB->>OB: strip_private_data()
    OB->>OB: 去重检查（LRU cache）
    
    alt 可选：原始内容存储
        OB->>DB: 保存 RawObservation
    end
    
    OB->>CP: compress(raw)

    alt 默认：合成压缩
        CP->>CP: build_synthetic()
        Note over CP: 0 token 消耗，< 0.1ms
    else 可选：LLM 压缩
        CP->>CP: 调 LLM → XML 结构化
    end

    Note over CP, DB: 批处理窗口（新增）：累积 N 条或 100ms → 一次事务
    CP->>DB: upsert_observation() ← SQLite INSERT + FTS5 INSERT 在同一事务
    CP->>IX: index(obs)
    IX->>BM: fts5.add(obs)         ← 自动同步到 FTS5
    IX->>VEC: vec.add(obs_id, embedding) + INSERT INTO observation_vectors
```

### 5.2 合成压缩算法

默认路径不调用 LLM，根据工具类型启发式提取：

| Tool | 提取策略 |
|---|---|
| `read_file` | 文件路径 → title, importance=3 |
| `write_file` | 文件路径 → title, importance=5 |
| `edit_file` | 文件路径 + diff 摘要 → title, importance=6 |
| `terminal` | 命令 + 错误检测 → title, 错误时 importance=8 |
| `grep/search` | 查询关键词 → title, importance=2 |

### 5.3 大量文本的处理

当 Agent 执行工具调用时，`tool_input` 和 `tool_output` 可能包含大量源代码或终端输出。
Memory **不存储原始文本内容**。

| Tool | input 大小 | output 大小 | 最终存储 |
|---|---|---|---|
| `read_file` | ~50 B | **~150 KB** | **~200 B**（仅路径+类型） |
| `write_file` | **~150 KB** | ~50 B | **~250 B**（仅路径） |
| `edit_file` | **~150 KB** | ~1-150 KB | **~400 B**（diff 摘要） |
| `terminal` | ~100 B | **~10-100 KB** | **~300 B**（截断后） |
| `grep/search` | ~50 B | **~10-50 KB** | **~150 B**（仅查询词） |

压缩后的数据存入 observations 表的 data JSON 列。FTS5 索引的字段来自：title, narrative, facts, concepts, files — 不包含原始文件内容。

#### 5.3.1 存储量估算（v0.2 基于 SQLite）

```
一次编码会话 200 次 tool 调用：
  read_file  × 80  × 200 B = 16 KB   →   observations.data JSON
  edit_file  × 60  × 400 B = 24 KB
  terminal   × 40  × 300 B = 12 KB
  search     × 20  × 150 B =  3 KB
  ─────────────────────────────
  Observation 存储          ≈ 55 KB
  向量 BLOB (200×384×4)     ≈ 307 KB
  FTS5 索引                 ≈ 30 KB（增量）
  SQLite 内部开销           ≈ 20 KB
  合计                      ≈ 412 KB/会话

每天 10 次会话 × 30 天 ≈ 123 MB/月
SQLite 自动管理存储增长，无需手动 compact。
```

### 5.4 批处理写入

**新增设计**：v0.1 每次 tool 调用触发 2 次独立 `spawn_blocking` 事务。v0.2 改为窗口批处理。

```rust
pub struct BatchWriter {
    buffer: Vec<PendingObservation>,
    max_batch: usize,               // 默认 10
    max_interval: Duration,         // 默认 100ms
    flush_handle: JoinHandle<()>,
}

impl BatchWriter {
    /// 写入队列 — 立即返回（O(1)），不阻塞调用线程
    pub async fn push(&self, obs: Observation) -> Result<()>;
}
```

批处理确保一次 `spawn_blocking` 事务提交多条记录。

**flush 触发条件**：
- 缓冲区达到 `max_batch` 条 → 立即 flush
- 上一次 flush 超过 `max_interval` → 自动 flush
- Drop 时 → 最终 flush

### 5.5 去重

同一 session 中相同 `(tool_name, tool_input_hash)` 的调用视为重复，跳过。

### 5.6 并发会话协调

AgentLoop 在 Daemon 中运行。**多个 qx 实例可同时连接同一个 Daemon**（例如 Zed 中的 ACP + 终端中的 CLI），各自有独立的 Conversation 但共享 MemoryCore。

**关键约束**：

```
Daemon 中有两个活跃 session：

┌─ sess_20260531_142530_123456 (CLI) ────┐
│  Conversation: "帮我改 auth.rs 的 JWT" │
│  observe(write_file, ...)              │ → 写入 MemoryCore
└─────────────────────────────────────────┘

┌─ sess_20260531_142531_654321 (ACP) ────┐
│  Conversation: "路由结构是怎样的"       │
│  build_context("路由")                 │ → 读到 CLI 刚才的操作
└─────────────────────────────────────────┘

✅ 对话完全隔离（各自 Conversation 独立）
✅ 记忆共享（一方 observe 后另一方 build_context 可搜到）
❌ 一方 crash → 不影响另一方
```

**Session ID 格式**：`sess_YYYYMMDD_HHMMSS_uuuuuu`（例 `sess_20260531_142530_123456`），按时间排序可追溯，微秒精度确保单 Daemon 内唯一。

**需要协调的问题**：

| 层级 | 策略 |
|---|---|
| Session 隔离 | 每个连接有独立的 SQLite Connection（WAL 模式支持多读一写） |
| Observation 写入 | 各 session 独立写入。观察者读到的不含未提交 session 的数据 |
| Consolidation | 一次只允许一个 session 运行 consolidation（占用一个 advisory lock） |
| 冲突解决 | consolidation 生成 Memory 前，在 SQLite 中用 `INSERT OR IGNORE` 基于 `(session_id, concepts_hash)` 去重 |

```sql
-- Consolidation 级别锁：使用 SQLite BEGIN IMMEDIATE 事务互斥
-- 如果另一个 consolidation 正在运行，BEGIN IMMEDIATE 返回 SQLITE_BUSY
-- 调用方捕获 SQLITE_BUSY 后跳过本次 consolidation
BEGIN IMMEDIATE;
-- 成功获取锁 → 运行 consolidation → COMMIT

-- Memory 去重
INSERT OR IGNORE INTO memories (id, created_at, updated_at, mem_type, data)
VALUES (?, ?, ?, ?, ?);
-- 冲突条件：(session_id, concepts_hash) 唯一索引
```

---

## 6. 检索管线

### 6.1 上下文构建

```rust
impl MemoryCore {
    /// 构建记忆上下文文本块，按 token 预算裁剪（默认 4000 tokens）
    pub async fn build_context(&self, query: &str, token_budget: u32) -> String;
}
```

优先级：Pinned Slots > 当前会话摘要 > 相关 Observation > 高热度 Memory

### 6.2 搜索 API

```rust
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as f64 / 3.5).ceil() as u32
}
```

**Session 去重上限改为可配置**（修正 v0.1 硬编码 3 的问题）：

```rust
pub struct SearchConfig {
    pub max_per_session: usize,    // 默认 3，可在配置文件中覆盖
    pub bm25_weight: f64,          // 默认 0.4
    pub vector_weight: f64,        // 默认 0.6
    pub rrf_k: u32,                // 默认 60
}
```

配置示例（`~/.qianxun/config.json`）：

```json
{
  "memory": {
    "search": {
      "max_per_session": 5,
      "bm25_weight": 0.4,
      "vector_weight": 0.6,
      "rrf_k": 60
    }
  }
}
```

### 6.3 Agent Tools

| Tool | 用途 |
|---|---|
| `memory_recall` | 搜索历史记忆，参数：query, limit |
| `memory_remember` | 手动保存持久记忆，参数：content, type, ttl_days |

---

## 7. 工作记忆 — Memory Slots

```rust
pub struct SlotManager { db: Arc<Connection> }

impl SlotManager {
    pub async fn list_pinned_slots(&self) -> Vec<MemorySlot>;
    pub async fn append(&self, label: &str, content: &str) -> Result<()>;
    pub async fn replace(&self, label: &str, content: &str) -> Result<()>;
    pub async fn create(&self, label: &str, desc: &str, size_limit: usize) -> Result<()>;
    pub async fn delete(&self, label: &str) -> Result<()>;
}
```

| 插槽 | 用途 | 默认尺寸 |
|---|---|---|
| `project_overview` | 项目概述 | 2000 字符 |
| `coding_style` | 编码风格 | 1000 字符 |
| `current_ticket` | 当前任务 | 2000 字符 |
| `decision_log` | 决策记录 | 2000 字符 |

---

## 8. 隐私清洗

覆盖 6 类敏感信息：

| 模式 | 示例 |
|---|---|
| API Keys | `sk-xxx`, `pk-xxx`, `ghp_xxx` |
| JWT | `eyJxxx.yyy.zzz` |
| 连接字符串密码 | `postgres://user:password@host` |
| AWS 密钥 | `AKIAxxxxxxxx` |
| 私钥 | `-----BEGIN RSA PRIVATE KEY-----` |
| 环境变量 | `API_KEY=secret123` |

---

## 9. 周期任务

### 9.1 Consolidation （展开设计）

#### 9.1.1 触发条件

每次 Session End 时触发。流程：

```
session_end()
  │
  ├─ 尝试 BEGIN IMMEDIATE 获取 consolidation 锁
  │   ├─ 已被其他 session 持有 → 跳过，记录待合并标记
  │   └─ 获取锁成功 → 继续
  │
  ├─ 扫描当前 session 的 Observations
  │   SELECT data FROM observations WHERE session_id = ? ORDER BY timestamp
  │
  ├─ 按 concepts 聚类（见 9.1.2）
  │
  ├─ 评估每个簇 → 决定是否生成 Memory（见 9.1.3）
  │
  ├─ 与已有 Memory 合并（见 9.1.4）
  │
  ├─ 生成 SessionSummary
  │
  └─ RELEASE_LOCK("consolidation")
```

#### 9.1.2 聚类算法

**输入**：当前 session 的所有 Observation（通常 50-500 条）
**输出**：Observation 簇，每个簇代表一个潜在 Memory

聚类使用**概念集合的交集**：

```
1. 对每条 Observation obs_i:
     C_i = { set(obs_i.concepts) ∪ set(keywords(obs_i.title)) ∪ set(obs_i.files) }
   
2. 聚类：
     初始化簇列表为空
     对每条 obs_i（按时间顺序）:
        找到已有的簇 C，使得 |C_i ∩ C_k| / min(|C_i|, |C_k|) > 0.5
        如果找到 → obs_i 加入该簇，更新簇的概念集
        没找到 → 创建新簇 { C_i }
   
3. 合并相似簇（二次扫描）：
     对任意两个簇 C_a, C_b:
        如果 |C_a ∩ C_b| / max(|C_a|, |C_b|) > 0.3 → 合并
   
4. 评估每个簇：
     平均 importance = avg(obs.importance for obs in 簇)
     簇大小 = |簇内的 Observation 数|
```

**时间复杂度**：O(n²)，n = session 内 Observation 数
- 500 条 → ~250K 对比较 → 在 spawn_blocking 中 ~50-100ms

**为什么不用 DBSCAN/k-means**：
- 不知道簇数
- concepts 集合是稀疏的（每个 Obs 只有 3-7 个 concept）
- 领域内概念名称天然携带语义，不需要嵌入选型

#### 9.1.3 Memory 生成条件

一个簇生成 Memory 需要满足 **任一**：

| 条件 | 说明 | 典型场景 |
|---|---|---|
| `avg importance ≥ 6` | 高重要性 | 错误修复、架构决策 |
| `簇大小 ≥ 3 且包含 edit_file` | 非偶然修改 | 重构、多文件改动 |
| `包含 terminal 且 is_error=true` | 踩坑记录 | 编译错误、环境问题 |

**不满足上述条件的簇**：不生成 Memory，保留为 Observation 供搜索使用。

#### 9.1.4 Memory 版本升级

生成或更新 Memory 时，与已有 Memory 做 **Jaccard 相似度**：

```
1. 对每个候选 Memory M_new：
     对每个 existing Memory M_old（同 project 范围内）：
        计算 J(M_new.concepts, M_old.concepts) = |A ∩ B| / |A ∪ B|
        如果 J > 0.7：
            → 版本升级：version += 1, parent_id = M_old.id, content = merge(M_old, M_new)
            → 旧 M_old 标记 is_latest = false
            → 跳出

2. 如果没有匹配的 existing Memory：
      → 新建 Memory，version = 1
```

概念集合的 Jaccard 阈值 0.7 的经验依据：两个 Memory 共享 > 70% 的标签概念时，它们讨论的是同一件事。

#### 9.1.5 回滚策略

Consolidation 不是事务性的（生成 Memory 涉及多条 SQL 写入），需要保护：

```
1. 每步写入先记录到 mem.db 的 consolidation_log 表（INSERT 型日志）
2. 全步骤完成后写入 check_point 记录
3. 如果 session_end 后 crash：
     下次启动时扫描 consolidation_log：
       - 有 check_point → 上次已完整完成
       - 有日志无 check_point → DELETE 未完成的 Memory 条目
```

### 9.2 Eviction

启动时 + 每 24 小时：

```
1. TTL 过期 → DELETE FROM memories WHERE ...
   清理对应的 observation_vectors

2. strength < 3 + access_count == 0 + 超过 30 天
   → DELETE FROM memories

3. 清理 FTS5 中的孤儿 rows（已删除的 observation 对应的 fts entry）
   → INSERT INTO obs_fts(obs_fts) VALUES('rebuild')  -- 仅当 orphans > 1000
```

### 9.3 Retention Scoring

每次 `search()` / `build_context()` 访问后更新：

```sql
UPDATE memories
SET data = json_set(data, '$.strength', min(
    json_extract(data, '$.strength') * 0.9 + 1.0,
    10
)),
    data = json_set(data, '$.access_count', json_extract(data, '$.access_count') + 1),
    data = json_set(data, '$.last_accessed_at', ?)
WHERE id = ?;
```

---

## 10. 依赖清单

```toml
# qianxun-memory/Cargo.toml
[dependencies]
qianxun-core = { path = "../qianxun-core" }
rusqlite = { version = "0.34", features = ["bundled", "vtab", "column_decltype"] }
    # bundled: 编译时嵌入 sqlite3.c
    # vtab: FTS5 虚拟表支持
regex = "1"
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
chrono = { workspace = true, features = ["serde"] }
uuid = { workspace = true }
lru = { workspace = true }

# 可选（中文分词增强）
# jieba-rs = "0.7"
# rusqlite 的 FTS5 tokenizer 扩展可能需要自定义 C 绑定
```

`qianxun-core` 不受影响，不新增任何依赖。

### 10.1 依赖变更对比

| crate | v0.1 | v0.2 | 原因 |
|---|---|---|---|
| `redb` | ✅ | ❌ 移除 | 替换为 SQLite |
| `rusqlite` | ❌ | ✅ 新增 | 嵌入式 SQL 数据库 |
| `rust-stemmers` | ✅ | ❌ 移除 | FTS5 内置 unicode61 处理词干 |
| `jieba-rs` | ✅ | ⚠️ 可选 | 仅当需要中文词组级 FTS5 tokenizer 时引入 |

---

## 11. 里程碑建议

### Phase 3a — SQLite 存储 + FTS5 全文搜索（3 周）

| 任务 | 预估 | 说明 |
|---|---|---|
| 创建 `qianxun-memory` crate | 0.5 天 | |
| MemoryObserver trait（qianxun-core） | 0.5 天 | 已有，需确认接口稳定性 |
| SQLite 表定义 + Migration | 1 天 | 8 张表 + 索引 + FTS5 虚拟表 |
| 数据类型定义 + JSON data 列 | 1 天 | |
| FTS5 全文索引（取代 BM25） | 2 天 | unicode61 tokenizer |
| 合成压缩 | 2 天 | |
| 隐私清洗 | 1 天 | |
| 本地文件同步（.md 快照） | 1 天 | |
| 批处理写入 | 1 天 | |
| cli 集成 + AgentLoop 钩子 | 2 天 | |
| 单元测试 | 2 天 | |

### Phase 3b — 向量检索（2 周）

| 任务 | 预估 | 说明 |
|---|---|---|
| VectorIndex（BLOB 列持久化） | 2 天 | |
| EmbeddingProvider + HTTP 实现 | 2 天 | |
| 嵌入结果缓存 | 1 天 | LRU cache |
| 断路器实现 | 0.5 天 | |
| HybridSearch（RRF + 降级 + 过滤） | 3 天 | |
| build_context() + SlotManager | 2 天 | |
| memory_recall Tool | 1 天 | |
| 集成测试 | 2 天 | |

### Phase 3c — Consolidation + 高级功能（2 周）

| 任务 | 预估 | 说明 |
|---|---|---|
| Consolidation 两阶段 | 3 天 | 聚类算法 + Memory 生成 |
| 版本升级（Jaccard） | 1 天 | |
| 并发协调（BEGIN IMMEDIATE 互斥） | 0.5 天 | |
| Eviction | 1 天 | |
| Retention scoring | 1 天 | |
| memory_remember Tool | 1 天 | |
| Consolidation 回滚 | 1 天 | |
| 旧 .md 迁移 | 1 天 | |
| 集成测试 + 压测 | 3 天 | |

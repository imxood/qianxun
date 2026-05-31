# 千寻记忆子系统设计

> 版本: 0.1 | 更新: 2026-05-29 | 状态: 草案

---

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
    Compressed --> Indexed: 加入 BM25 + Vector 索引
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

/// 原始观测 — Tool 调用的原始记录
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

## 3. 存储层 — redb + 本地文件双重存储

### 3.1 存储分工

| 存储 | 用途 | 写入策略 |
|---|---|---|
| **redb** | 机器读写：搜索、索引、结构化查询 | Observation（全量）、Memory（全量）、BM25 快照、向量 WAL |
| **本地文件 .md** | 人类读写：查看、编辑、备份、git 版本化 | Memory（strength ≥ 4）、Slot（全量） |

**核心原则**：redb 是权威源（source of truth），文件是可读快照（readable snapshot）。

### 3.2 redb 表定义

```rust
pub(crate) mod tables {
    pub const SESSIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("sessions");
    pub const OBSERVATIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("observations");
    pub const MEMORIES: TableDefinition<&str, &[u8]> = TableDefinition::new("memories");
    pub const SUMMARIES: TableDefinition<&str, &[u8]> = TableDefinition::new("summaries");
    pub const BM25_INDEX: TableDefinition<&str, &[u8]> = TableDefinition::new("bm25_index");
    pub const VECTOR_WAL: TableDefinition<&str, &[u8]> = TableDefinition::new("vector_wal");
    pub const VECTOR_META: TableDefinition<&str, &[u8]> = TableDefinition::new("vector_meta");
    pub const SLOTS: TableDefinition<&str, &[u8]> = TableDefinition::new("slots");
}
```

### 3.3 文件存储结构

```
~/.qianxun/
├── mem.db               # redb 数据库
├── mem.db.lock
└── memory/              # 可读记忆文件
    ├── architecture/    # 架构决策
    │   └── JWT认证方案.md
    ├── pattern/         # 开发模式
    ├── preference/      # 用户偏好
    ├── bug/             # 缺陷记录
    ├── workflow/        # 工作流
    ├── fact/            # 事实性知识
    └── slots/           # 工作记忆插槽
```

---

## 4. 索引层 — BM25 + 向量检索

### 4.1 BM25 全文索引

纯 Rust BM25 Okapi（k1=1.2, b=0.75），支持中英文混合文本。

```rust
pub struct SearchIndex {
    entries: HashMap<String, IndexEntry>,
    inverted: HashMap<String, HashSet<String>>,
    doc_term_counts: HashMap<String, HashMap<String, u32>>,
    total_doc_length: u64,
    k1: f64,
    b: f64,
}
```

- 英文：Porter 词干提取 + 停用词过滤
- 中文：jieba-rs 精确模式分词（非 bigram）
- 同义词扩展：add/create, remove/delete 等

### 4.2 向量索引 + WAL 持久化

```rust
pub struct VectorIndex {
    vectors: HashMap<String, VectorEntry>,
    dimensions: usize,
}
```

**持久化策略（修复全量序列化问题）**：
- 运行时：每次 add() 追加一条二进制 WAL 记录到 redb
- 不做全量序列化
- 启动时：遍历 VECTOR_WAL 重建索引
- Eviction 时：compact_vector_wal() 清理已删除条目

### 4.3 EmbeddingProvider

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    fn dimensions(&self) -> usize;
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
}
```

内置 HTTP 实现：兼容 ollama / vllm / text-embeddings-inference。

### 4.4 并发安全与锁策略

```rust
pub struct HybridSearch {
    bm25: Arc<RwLock<SearchIndex>>,                  // std::sync::RwLock，微秒级操作
    vector: Arc<RwLock<Option<VectorIndex>>>,        // std::sync::RwLock，微秒级操作
    embedding: Option<Box<dyn EmbeddingProvider>>,
    db: Arc<MemDb>,                                  // 所有文件 I/O 走 spawn_blocking
    bm25_weight: f64,      // 默认 0.4
    vector_weight: f64,    // 默认 0.6
}
```

**锁策略**（修复异步上下文中的阻塞问题）：

| 数据 | 锁类型 | 理由 |
|---|---|---|
| BM25 `HashMap` | `std::sync::RwLock` | 临界区 < 0.01ms，直接在 async 中安全 |
| Vector `HashMap` | `std::sync::RwLock` | 同上 |
| redb 文件 I/O | **`tokio::task::spawn_blocking`** | 磁盘操作 1-5ms，必须移出异步线程 |
| MemDb 共享 | `Arc` | 只读 |

核心原则：**能被 tokio 容忍的锁留在原位，必须 spawn_blocking 的绝不偷懒。**

```rust
// ✓ 正确：BM25 操作（< 0.01ms），直接在 async 中
self.bm25.write().unwrap().add(&obs);

// ✓ 正确：Vector 操作（< 0.01ms），直接在 async 中
self.vector.write().unwrap().add(&obs.id, vec);

// ✓ 正确：redb 文件写走 spawn_blocking
let db = self.db.clone();
tokio::task::spawn_blocking(move || {
    let tx = db.begin_write()?;
    let mut table = tx.open_table(tables::OBSERVATIONS)?;
    table.insert(key.as_str(), bytes.as_slice())?;
    tx.commit()?;
    Ok(())
}).await???
```

### 4.5 混合搜索 — HybridSearch

搜索流程：
1. BM25 搜索 limit × 2（读锁，可直接在 async 中）
2. 向量搜索（读锁 + spawn_blocking 加载数据）
3. RRF 融合（K=60）
4. 按 Session 去重（max 3 per session）
5. 加载完整 Observation（spawn_blocking 读 redb）
6. 按 filter 过滤 → 排序 → 截断

### 4.6 索引持久化

BM25 索引每 5 秒防抖序列化到 redb（spawn_blocking）。向量索引通过 WAL 增量持久化，无需定时序列化。

---

## 5. 捕获管线

### 5.1 完整流程

```mermaid
sequenceDiagram
    participant AL as AgentLoop
    participant OB as observe()
    participant CP as Compressor
    participant DB as redb
    participant IX as Indexer

    AL->>OB: observe(PostToolUse, input, output)
    OB->>OB: strip_private_data()
    OB->>OB: 去重检查（LRU cache）
    OB->>DB: 保存 RawObservation
    OB->>CP: compress(raw)

    alt 默认：合成压缩
        CP->>CP: build_synthetic()
        Note over CP: 0 token 消耗，< 0.1ms
    else 可选：LLM 压缩
        CP->>CP: 调 LLM → XML 结构化
    end

    CP->>DB: upsert_observation()
    CP->>IX: index(obs)
    IX->>BM25: bm25.add(obs)
    IX->>VEC: vec.add(obs_id, embedding) + append_vector_wal()
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
Memory **不存储原始文本内容**。每个阶段的文本处理策略如下。

#### 5.3.1 各工具类型的文本处理策略

| Tool | input 来源 | input 大小 | output 来源 | output 大小 | 最终存储 |
|---|---|---|---|---|---|
| `read_file` | `{path}` | ~50 B | 文件全文 | **~150 KB** | **~200 B**（仅路径+类型） |
| `write_file` | `{path, content}` | **~150 KB** | 确认消息 | ~50 B | **~250 B**（仅路径） |
| `edit_file` | `{path, old, new}` | **~150 KB** | diff/确认 | ~1-150 KB | **~400 B**（diff 摘要） |
| `terminal` | `{command}` | ~100 B | 终端输出 | **~10-100 KB** | **~300 B**（截断后） |
| `grep/search` | `{query}` | ~50 B | 匹配结果 | **~10-50 KB** | **~150 B**（仅查询词） |

#### 5.3.2 完整数据流（以 read_file 为例）

```
Agent 决定读取 src/auth.rs
  │
  ▼
tools.execute_async("read_file", {path: "src/auth.rs"})
  │ 磁盘读取 → 5000 行代码，~150KB
  ▼
observe(PostToolUse, "read_file", input, output)
  │
  ├─ observe() 接收到的数据：
  │    tool_name:  "read_file"                     ← 6 字节
  │    tool_input: { path: "src/auth.rs" }         ← 28 字节
  │    tool_output: "use jwt::Token;\npub fn..."    ← 极大，150KB
  │
  ├─ 第 1 步：strip_private_data(output)
  │    逐行扫描 150KB，替换 API Key/密码/私钥
  │    输出：脱敏后的 150KB 文本
  │    耗时：O(n) ~1-3ms
  │    内存：~150KB String（函数返回后释放）
  │
  ├─ 第 2 步：compressor.build_synthetic()
  │    根据工具类型决定提取策略：
  │
  │    (PostToolUse, "read_file") => {
  │        let path = extract_path(&tool_input);  // 只从 input 取路径
  │        // tool_output 的 150KB 代码 → **完全丢弃**
  │        // 不解析、不摘要、不统计
  │        // 原因：
  │        //   - 存储 150KB 代码到 memory 无意义
  │        //   - BM25 索引倒排所有变量名 → 搜索噪声
  │        //   - 代码在 conversation 中已有，不重复存储
  │
  │        Observation {
  │            type: FileRead,
  │            title: "读取文件: src/auth.rs",      // 28 字节
  │            narrative: "读取了文件 src/auth.rs",  // 24 字节
  │            facts: [],                           // 0
  │            concepts: ["auth","verify","JWT"],  // ~20 字节
  │            files: ["src/auth.rs"],              // 14 字节
  │            importance: 3,                       // read_file 本身不重要
  │        }
  │        // 总存储：~200 字节
  │        // 压缩率：150KB / 200B ≈ 750:1
  │    }
  │
  │    (PostToolUse, "edit_file") => {
  │        let path = extract_path(&tool_input);
  │        // 从 tool_input 提取 old_string/new_string 生成 diff 摘要
  │        // 不从 tool_output 提取（tool_output 可能是完整文件）
  │        let summary = extract_diff_summary(&tool_input);
  │        let concepts = extract_concepts_from_diff(&tool_input);
  │
  │        Observation {
  │            type: FileEdit,
  │            title: format!("编辑 {}: {}", path, summary),
  │            importance: 6,
  │            // 总存储：~400 字节
  │        }
  │    }
  │
  │    (PostToolUse, "terminal") => {
  │        let cmd = extract_cmd(&tool_input);     // 从 input 取命令
  │        // tool_output 终端输出 → 只取首尾各 3 行
  │        let output = truncate_output(&tool_output, 3);
  │        let is_error = has_error(&tool_output);  // 只检测关键词
  │
  │        Observation {
  │            type: if is_error { Error } else { CommandRun },
  │            title: format!("{}: {}", if is_error { "报错" } else { "执行" }, cmd),
  │            narrative: format!("{}\n输出摘要:\n{}", cmd, output),
  │            importance: if is_error { 8 } else { 4 },
  │            // 总存储：~300 字节（output 最多 500 字符）
  │        }
  │    }
  │
  │    (PostToolUse, "grep") => {
  │        let query = extract_query(&tool_input);  // 只取搜索词
  │        // tool_output 的搜索结果 → **完全丢弃**
  │        // 只存搜索关键词，方便未来同类搜索时召回
  │
  │        Observation {
  │            type: Search,
  │            title: format!("搜索: {}", query),
  │            importance: 2,
  │            // 总存储：~150 字节
  │        }
  │    }
  │
  │    关键原则：所有工具的压缩算法都只从 tool_input 提取结构化元数据
  │    （路径、命令、关键词），从不存储原始文件内容。
  │
  ├─ 第 3 步：写入 redb（spawn_blocking）
  │    写入 ~200 字节的 Observation JSON
  │
  ├─ 第 4 步：BM25 索引
  │    add(&obs) 提取的术语来自 title + concepts + files：
  │    "读取文件" "src/auth.rs" "auth" "verify" "JWT"
  │    不来自原始 tool_output
  │
  └─ 第 5 步：向量索引（可选）
      embed 输入：obs.title + " " + obs.narrative
      = "读取文件: src/auth.rs 读取了文件 src/auth.rs"
      ≈ 50 字节（对比原始 150KB，缩小 3000 倍）
```

#### 5.3.3 存储量估算

```
一次编码会话 200 次 tool 调用：
  read_file  × 80  × 200 B = 16 KB
  edit_file  × 60  × 400 B = 24 KB
  terminal   × 40  × 300 B = 12 KB
  search     × 20  × 150 B =  3 KB
  ─────────────────────────────
  Observation 存储          ≈ 55 KB
  向量 WAL (200×384×4)      ≈ 307 KB
  合计                      ≈ 365 KB/会话

每天 10 次会话 × 30 天 ≈ 110 MB/月
redb 文件增长可控。
```

如果换成**不压缩直接存原始内容**：

```
80 次 read_file × 150 KB = 12 MB/会话
200 次会话 = 2.4 GB → 一个月打爆 VPS 的 10 GB 磁盘
```

#### 5.3.4 为什么不存储原始内容

| 理由 | 说明 |
|---|---|
| 存储膨胀 | 一次 read_file 写 150KB，10000 次 = 1.5GB，redb 膨胀不可控 |
| 检索无效 | BM25 索引倒排 5000 行代码的全量关键词，噪声远大于信号 |
| 隐私风险 | 源码中可能包含 API Key、密码等敏感信息 |
| LLM 上下文 | 原始内容在 conversation 中已有，memory 重复存储无意义 |

#### 5.3.5 可选的原始内容存储通道

默认不存储原始内容。如果未来需要审计追溯，可通过独立文件按日期分片存储：

```
~/.qianxun/raw/
  2026-05-29/
    sess_abc_obs_001.json    ← 完整的 RawObservation（含原始文件内容）
    sess_abc_obs_002.json
```

仅在配置中显式启用 `QIANXUN_MEM_STORE_RAW=true` 时写入。不与 redb 共享存储路径，避免数据库膨胀。

### 5.4 去重

同一 session 中相同 `(tool_name, tool_input_hash)` 的调用视为重复，跳过。

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

### 6.3 Agent Tools

| Tool | 用途 |
|---|---|
| `memory_recall` | 搜索历史记忆，参数：query, limit |
| `memory_remember` | 手动保存持久记忆，参数：content, type, ttl_days |

---

## 7. 工作记忆 — Memory Slots

```rust
pub struct SlotManager { db: Arc<MemDb> }

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

### 9.1 Consolidation（无 LLM 阶段）

每次 Session End 触发：
1. 扫描当前 session 的 Observation
2. 按 concepts 交集 > 0.5 聚类
3. avg importance > 6 → 生成 Memory
4. Jaccard > 0.7 → 版本升级

### 9.2 Eviction

启动时 + 每 24 小时：
1. TTL 过期 → forget
2. strength < 3 + access_count == 0 + 创建超过 30 天 → forget
3. compact_vector_wal() 清理已删除条目

### 9.3 Retention Scoring

每次 search()/context() 访问后更新：
`strength = min(strength × 0.9 + 1.0, 10)`

---

## 10. 依赖清单

```toml
# qianxun-memory/Cargo.toml
[dependencies]
qianxun-core = { path = "../qianxun-core" }
redb = "4.1"
rust-stemmers = "1"
jieba-rs = "0.7"
regex = "1"
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
chrono = { workspace = true, features = ["serde"] }
uuid = { workspace = true }
lru = { workspace = true }
notify = { workspace = true }
```

`qianxun-core` 不受影响，不新增任何依赖。

---

## 11. 里程碑建议

### Phase 3a — 核心存储 + 基础捕获（3 周）

| 任务 | 预估 |
|---|---|
| 创建 `qianxun-memory` crate | 0.5 天 |
| MemoryObserver trait（qianxun-core） | 0.5 天 |
| MemDb + 表定义 | 1 天 |
| 数据类型定义 | 1 天 |
| BM25 + jieba CJK 分词 | 3 天 |
| 合成压缩 | 2 天 |
| 隐私清洗 | 1 天 |
| 本地文件存储 | 1 天 |
| cli 集成 + AgentLoop 钩子 | 2 天 |
| 单元测试 | 2 天 |

### Phase 3b — 语义检索（2 周）

| 任务 | 预估 |
|---|---|
| VectorIndex | 2 天 |
| EmbeddingProvider + HTTP 实现 | 2 天 |
| 向量 WAL 持久化 | 1 天 |
| HybridSearch（RRF + 降级 + 过滤） | 3 天 |
| IndexPersistence | 1 天 |
| build_context() + SlotManager | 2 天 |
| memory_recall Tool | 1 天 |
| 集成测试 | 2 天 |

### Phase 3c — 高级功能（2 周）

| 任务 | 预估 |
|---|---|
| Consolidation 两阶段 | 3 天 |
| Eviction + WAL compact | 2 天 |
| Retention scoring | 1 天 |
| memory_remember Tool | 1 天 |
| 文件同步（file watcher） | 2 天 |
| 旧 .md 迁移 | 1 天 |
| 集成测试 + 压测 | 3 天 |

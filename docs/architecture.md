# 千寻系统架构设计文档 v0.3

> 版本: 0.3 | 更新: 2026-05-29 | 状态: 草案
>
> 覆盖记忆子系统设计、Crate 结构、多主机架构、VPS Server + Node 中心化控制。
> 关键变更：Memory 独立为 qianxun-memory crate、
> 通过 trait 与 qianxun-core 解耦、redb+文件双重存储、Daemon 模式集成设计。

---

## 目录

1. [修订对照](#1-修订对照)
2. [设计目标](#2-设计目标)
3. [架构总览](#3-架构总览)
4. [记忆模型](#4-记忆模型)
5. [存储层 — redb 持久化](#5-存储层--redb-持久化)
6. [索引层 — BM25 + 向量检索](#6-索引层--bm25--向量检索)
7. [捕获管线 — Observe → Compress → Index](#7-捕获管线)
8. [检索管线 — 混合搜索 + 上下文注入](#8-检索管线)
9. [工作记忆 — Memory Slots](#9-工作记忆--memory-slots)
10. [隐私清洗](#10-隐私清洗)
11. [周期任务 — Consolidation / Eviction](#11-周期任务)
12. [集成方案 — 与现有代码对接](#12-集成方案)
13. [模块结构](#13-模块结构)
14. [依赖清单](#14-依赖清单)
15. [里程碑建议](#15-里程碑建议)

---

## 1. 修订对照

| 问题 | v0.1 状态 | v0.2 修复 | v0.3 修复 |
|---|---|---|---|
| AgentLoop 集成点未设计 | ❌ | ✅ 第 12 章 5 个集成点 + 代码 diff | — |
| 向量持久化方案不可行 | ❌ | ✅ WAL 增量 + 启动重建 | — |
| CJK 搜索使用 bigram 无效 | ❌ | ✅ jieba-rs | — |
| 隐私清洗 | ❌ | ✅ 正则方案 | — |
| 并发模型 | ❌ | ✅ Arc<RwLock> | — |
| 降级容错 | ❌ | ✅ BM25-only fallback | — |
| **Memory 嵌入 qianxun-core 耦合严重** | ❌ | ❌ 未修复 | ✅ **独立 qianxun-memory crate** |
| **存储仅有 redb** | — | — | ✅ **redb + 本地文件双重存储** |
| **Daemon 模式角色不明** | — | — | ✅ **Memory Daemon + IPC + file watcher** |

---

## 2. 设计目标

### 核心理念

> **千寻不需要重新学习你已经做过的事。**

| 目标 | 说明 |
|---|---|
| **自动捕获** | Agent 每次 Tool 调用、错误、决策自动记录为 Observation |
| **零开销压缩** | 默认不调 LLM，启发式提取结构化记忆体，0 token 消耗 |
| **即时可检索** | Observation 压缩后立即进入搜索索引，后续提问即可召回 |
| **跨会话持久化** | Memory 跨会话保留，6 种类型分类（Pattern/Preference/Architecture/Bug/Workflow/Fact） |
| **语义搜索** | BM25 + 向量混合检索，RRF 融合排序 |
| **项目隔离** | 不同工作区记忆隔离，搜索时自动过滤 |
| **零外部服务** | 全本地存储，不依赖数据库服务器 |

### 非目标

- 知识图谱（Phase 3 后期评估）
- 团队协作记忆
- 多 Agent 隔离

---

## 3. 架构总览

### 3.1 Crate 依赖关系

```mermaid
graph LR
    subgraph workspace
        CORE[qianxun-core]
        MEM[qianxun-memory
            <i>新增</i>]
        CLI[qianxun-cli]
        ACP[qianxun-acp]
        DAEMON[qianxun-daemon
            <i>Phase 4</i>]
    end

    %% core 定义 MemoryObserver trait，不依赖 memory crate
    CORE -->|定义 MemoryObserver trait| MEM
    
    %% cli 依赖 core + memory
    CLI --> CORE
    CLI --> MEM
    
    %% acp 同样依赖两者
    ACP --> CORE
    ACP --> MEM
    
    %% daemon 依赖 memory（独占 redb）+ core
    DAEMON --> MEM
    DAEMON --> CORE
```

**核心原则**：`qianxun-core` **绝不**依赖 `qianxun-memory`。

记忆相关 trait 定义在 `qianxun-core`，具体实现在 `qianxun-memory`。
CLI / ACP / Daemon 三者负责组装：创建 `MemoryCore` → 实现 trait → 注入 AgentLoop。

### 3.2 运行时架构

```mermaid
graph TB
    subgraph "qianxun-core"
        AL[AgentLoop]
        TRAIT[MemoryObserver trait]
        AL -->|持有 Option<Box<dyn MemoryObserver>>| TRAIT
    end

    subgraph "qianxun-memory"
        MC[MemoryCore
            MemoryObserver 实现]
        MC --> ST[Storage]
        MC --> IDX[Indexes]
        MC --> PP[Pipeline]
        
        subgraph "Storage: 双存储"
            R[(redb<br/>结构化搜索)]
            F[(本地文件 .md<br/>人类可读/可编辑)]
        end
        ST --> R
        ST --> F
        
        IDX --> BM25[BM25 SearchIndex]
        IDX --> VEC[VectorIndex]
        IDX --> WAL[向量 WAL
            redb 内]
    end

    subgraph "组装层（CLI / Daemon）"
        MAIN[cli.rs / daemon.rs]
        MAIN -->|创建 MemoryCore| MC
        MAIN -->|注入 AgentLoop| AL
        MAIN -->|memory_context: &str| AL
    end

    subgraph "持久化层"
        REDB[(~/.qianxun/mem.db)]
        FILES[(~/.qianxun/memory/...)]
        R --> REDB
        F --> FILES
    end
```

### 3.3 MemoryObserver trait

```rust
// === qianxun-core/src/memory.rs（新增文件，非目录） ===

use async_trait::async_trait;

/// 记忆观察者 — qianxun-core 对记忆子系统的全部依赖，
/// 通过此 trait 与 qianxun-memory 解耦。
#[async_trait]
pub trait MemoryObserver: Send + Sync {
    /// 捕获一次工具调用
    async fn observe(
        &self,
        hook_type: HookType,
        tool_name: &str,
        tool_input: Option<serde_json::Value>,
        tool_output: Option<&str>,
    );

    /// 构建记忆上下文文本块（注入 system prompt）
    async fn build_context(&self, query: &str, token_budget: u32) -> String;

    /// 手动保存一条持久记忆
    async fn remember(&self, content: &str, mem_type: &str) -> anyhow::Result<String>;

    /// 搜索记忆
    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>>;

    /// 会话生命周期
    async fn session_start(&self, session_id: &str, project: &str, cwd: &str);
    async fn session_end(&self);
}
```

`qianxun-core` 只有这个 trait 文件，不引入任何 redb / jieba / 存储逻辑。

### 与现有代码的关键集成

```mermaid
sequenceDiagram
    participant User
    participant CLI as cli.rs handle_message
    participant MC as MemoryCore
    participant AL as AgentLoop
    participant LLM
    
    User->>CLI: 输入消息
    
    Note over CLI: 调用 build_context()
    CLI->>MC: memory_context = build_context(msg, token_budget=4000)
    MC-->>CLI: Vec<ContextBlock> → 格式化字符串
    
    Note over CLI: 已有集成点（仅需替换 MemoryManager → MemoryCore）
    
    CLI->>AL: handle_user_message(..., memory_context, ...)
    
    loop 每个 Tool 调用
        Note over AL: 注入 memory_context → system prompt
        
        AL->>LLM: streaming request (system 含记忆上下文)
        LLM-->>AL: tool_call
        
        Note over AL: 捕获 Tool 调用
        AL->>MC: observe(pre_tool_use, tool_name, tool_input)
        
        AL->>CLI: execute tool
        CLI-->>AL: tool_result
        
        Note over AL: 捕获 Tool 结果
        AL->>MC: observe(post_tool_use, tool_name, tool_result)
        MC-->>MC: 合成压缩 + 索引（零 LLM，<1ms）
        
        AL->>LLM: 继续流式（带 tool_result）
    end
    
    LLM-->>AL: stop
    AL-->>CLI: turn finished
    
    Note over CLI: Session 结束时
    CLI->>MC: on_session_end()
    MC-->>MC: Consolidation + 索引持久化
```

---

## 4. 记忆模型

### 4.1 核心数据结构

所有类型定义放置在 `memory/types.rs`，与 `types.rs` 分离以避免循环依赖。

```rust
// === memory/types.rs ===

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── 基础标识 ────────────────────────────────────────────

pub type ObsId = String;
pub type SessionId = String;
pub type MemoryId = String;

// ─── 会话 ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SessionStatus { Active, Completed, Abandoned }

// ─── 原始观测 ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawObservation {
    pub id: ObsId,
    pub session_id: SessionId,
    pub timestamp: DateTime<Utc>,
    pub hook_type: HookType,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_output: Option<serde_json::Value>,
    pub user_prompt: Option<String>,
    pub assistant_response: Option<String>,
}

// ─── 压缩后的观测 ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ObservationType {
    FileRead, FileWrite, FileEdit, CommandRun,
    Search, WebFetch, Conversation, Error,
    Decision, Discovery, Subagent, Task, Other,
}

// ─── 持久化记忆 ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub mem_type: MemoryType,
    pub title: String,
    pub content: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub session_ids: Vec<SessionId>,
    pub strength: u8,                           // 1–10
    pub version: u32,
    pub parent_id: Option<MemoryId>,
    pub supersedes: Vec<MemoryId>,
    pub is_latest: bool,
    pub forget_after: Option<DateTime<Utc>>,
    pub project: Option<String>,
    pub access_count: u64,                      // 访问计数 → 热度评分
    pub last_accessed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Pattern, Preference, Architecture, Bug, Workflow, Fact,
}

// ─── 会话摘要 ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: SessionId,
    pub project: String,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub narrative: String,
    pub key_decisions: Vec<String>,
    pub files_modified: Vec<String>,
    pub concepts: Vec<String>,
    pub observation_count: u32,
}

// ─── 工作记忆插槽 ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySlot {
    pub label: String,
    pub content: String,
    pub size_limit: usize,
    pub description: String,
    pub pinned: bool,
    pub read_only: bool,
    pub scope: SlotScope,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SlotScope { Project, Global }

// ─── 事件类型 ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HookType {
    SessionStart, PromptSubmit, PreToolUse,
    PostToolUse, PostToolFailure, SessionEnd,
}

// ─── 搜索结果 ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub observation: Observation,
    pub bm25_score: f64,
    pub vector_score: f64,
    pub combined_score: f64,
    pub session_id: SessionId,
}

pub struct SearchFilter {
    pub project: Option<String>,
    pub obs_types: Option<Vec<ObservationType>>,
    pub min_importance: Option<u8>,
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub session_ids: Option<Vec<SessionId>>,
}

// ─── 上下文块 ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ContextBlock {
    pub block_type: ContextBlockType,
    pub content: String,
    pub estimated_tokens: u32,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContextBlockType {
    Summary, Observation, Memory, Slot,
}
```

### 4.2 生命周期

```mermaid
stateDiagram-v2
    [*] --> Raw: Agent 触发 Hook
    Raw --> Compressed: 合成压缩（0 LLM）
    Compressed --> Indexed: 加入 BM25 + Vector 索引
    
    state Indexed {
        [*] --> BM25Added
        BM25Added --> VectorAdded: 可选嵌入
        VectorAdded --> [*]
    }
    
    Indexed --> Memory: Consolidation 周期（统计 + 可选 LLM）
    Indexed --> [*]: Eviction 清理
    
    Memory --> OldVersion: 发现重复（Jaccard > 0.7）
    OldVersion --> Memory: 版本升级 + 继承
    Memory --> [*]: TTL 过期 / 手动 forget
```

---

## 5. 存储层 — redb + 本地文件双重存储

### 5.1 设计原则

| 存储 | 用途 | 写入策略 |
|---|---|---|
| **redb** | 机器读写：搜索、索引、结构化查询 | Observation（全量）、Memory（全量）、BM25 快照、向量 WAL |
| **本地文件 .md** | 人类读写：查看、编辑、备份、git 版本化 | Memory（strength ≥ 4）、Slot（全量） |

**不写文件的数据**：Observation（90%+ 的数据量，太细碎不值得写文件）、RawObservation、Session 记录。

**核心原则**：redb 是权威源（source of truth），文件是可读快照（readable snapshot）。
文件落后于 redb 是可接受的——搜索总是从 redb 走。
用户直接编辑 .md 文件后，Daemon 的 file watcher 会检测变化并写回 redb。

### 5.2 文件存储结构

```
~/.qianxun/
├── mem.db               # redb 数据库文件
├── mem.db.lock          # redb 内部锁文件
└── memory/              # 人类可读的记忆文件
    ├── architecture/    # 架构决策
    │   └── JWT认证方案.md
    ├── pattern/         # 开发模式
    ├── preference/      # 用户偏好
    ├── bug/             # 缺陷记录
    ├── workflow/        # 工作流
    ├── fact/            # 事实性知识
    └── slots/           # 工作记忆插槽
        ├── project_overview.md
        └── coding_style.md
```

每个 .md 文件包含 YAML frontmatter 描述元数据 + Markdown 正文。

### 5.3 文件同步策略

CLI 模式（进程内）：同步写文件，写入 redb 后立即写文件。
Daemon 模式（常驻）：写 redb + 写文件 + file watcher 监控文件变化回写 redb。

```rust
// 写入时：redb 优先，文件异步
pub async fn save_memory(&self, mem: &Memory) -> anyhow::Result<()> {
    self.db.upsert_memory(mem)?;                    // 1. redb 先写（搜索可见）
    if mem.strength >= 4 {
        self.fs.write_memory_file(mem).await;        // 2. 文件异步写（人类可读）
    }
    self.index.add(mem);                             // 3. 索引更新
    Ok(())
}

// 读取时：只从 redb 读（权威源）
pub async fn search(&self, query: &str) -> Vec<SearchHit> {
    self.hybrid_search.search(query, 10, None).await  // 从 redb 加载 observation 数据
}
```

### 5.4 为什么选择 redb

| 对比 | redb | sled | SQLite |
|---|---|---|---|
| 纯 Rust | ✓ | ✓ | ❌ (C 绑定) |
| 传递依赖 | ~30 个 | ~50+ | ~80+ |
| MVCC 事务 | ✓ | 部分 | ✓ |
| 单文件 | ✓ | ❌（多日志文件） | ✓ |
| 符合项目规则 | ✓ (< 100 传递依赖) | ❌ (> 100) | ❌ |

### 5.2 表定义

```rust
// === memory/storage/schema.rs ===

use redb::TableDefinition;

pub(crate) mod tables {
    // 会话表
    pub const SESSIONS: TableDefinition<&str, &[u8]> =
        TableDefinition::new("sessions");

    // 观测表：key = "{session_id}:{obs_id}"
    pub const OBSERVATIONS: TableDefinition<&str, &[u8]> =
        TableDefinition::new("observations");

    // 记忆表
    pub const MEMORIES: TableDefinition<&str, &[u8]> =
        TableDefinition::new("memories");

    // 会话摘要表
    pub const SUMMARIES: TableDefinition<&str, &[u8]> =
        TableDefinition::new("summaries");

    // BM25 索引快照
    pub const BM25_INDEX: TableDefinition<&str, &[u8]> =
        TableDefinition::new("bm25_index");

    // 向量 WAL（Write-Ahead Log）
    // key = "wal:{obs_id}"
    // value = { session_id, 二进制向量 }
    pub const VECTOR_WAL: TableDefinition<&str, &[u8]> =
        TableDefinition::new("vector_wal");

    // 向量元数据（维度、provider 信息）
    pub const VECTOR_META: TableDefinition<&str, &[u8]> =
        TableDefinition::new("vector_meta");

    // 工作记忆插槽
    pub const SLOTS: TableDefinition<&str, &[u8]> =
        TableDefinition::new("slots");
}
```

### 5.3 MemDb 封装

```rust
// === memory/storage/db.rs ===

use redb::{Database, ReadableTable, ReadTransaction, WriteTransaction};
use std::path::Path;
use std::sync::Arc;

pub struct MemDb {
    db: Arc<Database>,
}

impl MemDb {
    /// 打开或创建 redb 数据库。
    /// 使用 Database::builder() 而非 create()，后者只适用于新建。
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        // redb 4.1: builder() 自动处理 create-or-open
        let db = Database::builder()
            .create_if_missing(true)    // 不存在则创建
            .open(path)?;

        // 确保所有表存在（open_table 在首次调用时创建）
        let tx = db.begin_write()?;
        tx.open_table(tables::SESSIONS)?;
        tx.open_table(tables::OBSERVATIONS)?;
        tx.open_table(tables::MEMORIES)?;
        tx.open_table(tables::SUMMARIES)?;
        tx.open_table(tables::BM25_INDEX)?;
        tx.open_table(tables::VECTOR_WAL)?;
        tx.open_table(tables::VECTOR_META)?;
        tx.open_table(tables::SLOTS)?;
        tx.commit()?;

        Ok(Self { db: Arc::new(db) })
    }

    pub fn begin_read(&self) -> anyhow::Result<ReadTransaction> {
        Ok(self.db.begin_read()?)
    }

    pub fn begin_write(&self) -> anyhow::Result<WriteTransaction> {
        Ok(self.db.begin_write()?)
    }
}
```

### 5.4 数据访问操作

```rust
// === memory/storage/ops.rs ===

impl MemDb {
    // ─── Session ──────────────────────────────────────────

    pub fn upsert_session(&self, session: &Session) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::SESSIONS)?;
            table.insert(session.id.as_str(), serde_json::to_vec(session)?.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> anyhow::Result<Option<Session>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::SESSIONS)?;
        match table.get(id)? {
            Some(slice) => Ok(Some(serde_json::from_slice(slice.value())?)),
            None => Ok(None),
        }
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<Session>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::SESSIONS)?;
        let mut out = Vec::new();
        for result in table.iter()? {
            let (_, val) = result?;
            out.push(serde_json::from_slice(val.value())?);
        }
        Ok(out)
    }

    // ─── Observation ────────────────────────────────────

    pub fn upsert_observation(&self, session_id: &str, obs: &Observation) -> anyhow::Result<()> {
        let key = format!("{}:{}", session_id, obs.id);
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::OBSERVATIONS)?;
            table.insert(key.as_str(), serde_json::to_vec(obs)?.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 前缀扫描某 session 的所有观测
    pub fn list_observations(&self, session_id: &str) -> anyhow::Result<Vec<Observation>> {
        let prefix = format!("{}:", session_id);
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::OBSERVATIONS)?;
        let mut out = Vec::new();
        for result in table.iter()? {
            let (key, val) = result?;
            if key.value().starts_with(prefix.as_str()) {
                out.push(serde_json::from_slice(val.value())?);
            }
        }
        Ok(out)
    }

    /// 获取单条观测
    pub fn get_observation(&self, session_id: &str, obs_id: &str) -> anyhow::Result<Option<Observation>> {
        let key = format!("{}:{}", session_id, obs_id);
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::OBSERVATIONS)?;
        match table.get(key.as_str())? {
            Some(slice) => Ok(Some(serde_json::from_slice(slice.value())?)),
            None => Ok(None),
        }
    }

    // ─── Memory ───────────────────────────────────────────

    pub fn upsert_memory(&self, mem: &Memory) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::MEMORIES)?;
            table.insert(mem.id.as_str(), serde_json::to_vec(mem)?.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_memories(&self) -> anyhow::Result<Vec<Memory>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::MEMORIES)?;
        let mut out = Vec::new();
        for result in table.iter()? {
            let (_, val) = result?;
            out.push(serde_json::from_slice(val.value())?);
        }
        Ok(out)
    }

    pub fn delete_memory(&self, id: &str) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::MEMORIES)?;
            table.remove(id)?;
        }
        tx.commit()?;
        Ok(())
    }

    // ─── BM25 快照 ────────────────────────────────────────

    pub fn save_bm25_snapshot(&self, data: &str) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::BM25_INDEX)?;
            table.insert("bm25", data.as_bytes())?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_bm25_snapshot(&self) -> anyhow::Result<Option<String>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::BM25_INDEX)?;
        match table.get("bm25")? {
            Some(slice) => Ok(Some(String::from_utf8(slice.value().to_vec())?)),
            None => Ok(None),
        }
    }

    // ─── 向量 WAL ─────────────────────────────────────────

    /// 追加一条向量记录到 WAL。
    /// 每次 add() 调用时追加，而非定时全量 dump。
    pub fn append_vector_wal(
        &self,
        obs_id: &str,
        session_id: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        let key = format!("wal:{}", obs_id);
        // 二进制格式：4 字节 session_id 长度 + session_id 字节 + 4 字节向量长度 + 向量 f32 字节
        let sid_bytes = session_id.as_bytes();
        let mut buf = Vec::with_capacity(8 + sid_bytes.len() + embedding.len() * 4);
        buf.extend_from_slice(&(sid_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(sid_bytes);
        buf.extend_from_slice(&(embedding.len() as u32).to_le_bytes());
        for &f in embedding {
            buf.extend_from_slice(&f.to_le_bytes());
        }

        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::VECTOR_WAL)?;
            table.insert(key.as_str(), buf.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 启动时遍历 WAL，重建向量索引
    pub fn replay_vector_wal(&self) -> anyhow::Result<Vec<(String, String, Vec<f32>)>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::VECTOR_WAL)?;
        let mut entries = Vec::new();
        for result in table.iter()? {
            let (key, val) = result?;
            let key_str = key.value();
            if !key_str.starts_with("wal:") {
                continue;
            }
            let obs_id = &key_str[4..];
            let bytes = val.value();
            if bytes.len() < 8 {
                continue;
            }
            let mut pos = 0;
            let sid_len = u32::from_le_bytes(bytes[pos..pos+4].try_into()?) as usize;
            pos += 4;
            let session_id = String::from_utf8(bytes[pos..pos+sid_len].to_vec())?;
            pos += sid_len;
            let vec_len = u32::from_le_bytes(bytes[pos..pos+4].try_into()?) as usize;
            pos += 4;
            let mut embedding = Vec::with_capacity(vec_len);
            for _ in 0..vec_len {
                if pos + 4 > bytes.len() { break; }
                embedding.push(f32::from_le_bytes(bytes[pos..pos+4].try_into()?));
                pos += 4;
            }
            entries.push((obs_id.to_string(), session_id, embedding));
        }
        Ok(entries)
    }

    /// 压缩 WAL：清除已被清理的 obs 条目
    /// 在 eviction 时调用
    pub fn compact_vector_wal(&self, active_obs_ids: &HashSet<String>) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::VECTOR_WAL)?;
            let mut to_remove = Vec::new();
            for result in table.iter()? {
                let (key, _) = result?;
                let key_str = key.value();
                if key_str.starts_with("wal:") {
                    let obs_id = &key_str[4..];
                    if !active_obs_ids.contains(obs_id) {
                        to_remove.push(key_str.to_string());
                    }
                }
            }
            for k in to_remove {
                table.remove(k.as_str())?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // ─── Vector Meta ────────────────────────────────────

    pub fn save_vector_meta(&self, dimensions: usize, provider: &str) -> anyhow::Result<()> {
        let meta = serde_json::json!({"dimensions": dimensions, "provider": provider});
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::VECTOR_META)?;
            table.insert("meta", meta.to_string().as_bytes())?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_vector_meta(&self) -> anyhow::Result<Option<(usize, String)>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::VECTOR_META)?;
        match table.get("meta")? {
            Some(slice) => {
                let v: serde_json::Value = serde_json::from_slice(slice.value())?;
                let dims = v["dimensions"].as_u64().ok_or_else(|| anyhow::anyhow!("missing dimensions"))? as usize;
                let prov = v["provider"].as_str().ok_or_else(|| anyhow::anyhow!("missing provider"))?.to_string();
                Ok(Some((dims, prov)))
            }
            None => Ok(None),
        }
    }

    // ─── Slots ─────────────────────────────────────────

    pub fn upsert_slot(&self, slot: &MemorySlot) -> anyhow::Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(tables::SLOTS)?;
            table.insert(slot.label.as_str(), serde_json::to_vec(slot)?.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_slots(&self) -> anyhow::Result<Vec<MemorySlot>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(tables::SLOTS)?;
        let mut out = Vec::new();
        for result in table.iter()? {
            let (_, val) = result?;
            out.push(serde_json::from_slice(val.value())?);
        }
        Ok(out)
    }
}
```

### 5.5 数据库位置

```
~/.qianxun/
├── mem.db          # redb 数据库文件
├── mem.db.lock     # redb 内部锁文件
└── images/         # 图片附件存储（可选，Phase 3c）
```

---

## 6. 索引层 — BM25 + 向量检索

### 6.1 BM25 全文索引

纯 Rust BM25 Okapi（k1=1.2, b=0.75），支持**双索引**策略以处理 CJK 混合文本。

```rust
// === memory/index/search_index.rs ===

use std::collections::{HashMap, HashSet};
use crate::memory::types::Observation;

pub struct SearchIndex {
    entries: HashMap<String, IndexEntry>,
    inverted: HashMap<String, HashSet<String>>,   // term → Set<obs_id>
    doc_term_counts: HashMap<String, HashMap<String, u32>>,
    total_doc_length: u64,
    k1: f64,
    b: f64,
    /// 分词器类型：CJK 或 Latin
    /// 根据输入文本自动选择
}

impl SearchIndex {
    pub fn new() -> Self;
    pub fn add(&mut self, obs: &Observation);
    pub fn remove(&mut self, obs_id: &str);
    
    /// BM25 搜索，返回 top-K
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit>;
    
    /// 分词入口：自动检测 CJK
    fn tokenize(&self, text: &str) -> Vec<String>;
    
    /// 英文分词：Porter 词干 + 停用词过滤
    fn tokenize_latin(&self, text: &str) -> Vec<String>;
    
    /// 中文分词：使用 jieba-rs 进行精确模式分词
    fn tokenize_cjk(&self, text: &str) -> Vec<String>;
    
    /// 序列化：entries + inverted + doc_term_counts → JSON
    pub fn serialize(&self) -> String;
    pub fn deserialize(json: &str) -> Self;
}
```

**CJK 搜索方案**（修复 v0.1 的 bigram 问题）：

```
索引时：
  "JWT认证中间件" → jieba 分词 → ["JWT", "认证", "中间件"]
  "先在 src/middleware/auth.ts 中添加 jose 中间件"
  → jieba 分词 → ["先", "在", "src", "/", "middleware", "/", "auth", ".", "ts",
                     "中", "添加", "jose", "中间件"]
  → 英文词保持原样，中文词用 jieba 切分
  → ["src/middleware/auth.ts", "jose", "中间件", "添加"]

查询时：
  "中间件" → jieba → ["中间件"]
  ← stArrtsWith 前缀匹配 → 命中索引中的 "中间件"

对比 v0.1 bigram 方案：
  查询 "中间件" → bigram ["中间", "间件"]
  索引 "JWT认证中间件" → bigram ["JW","WT","T认","认证","证中","中间","间件"]
  bigram 交集 → "中间"+"间件" 可以匹配，但 "中间件安装" 的 bigram 是 ["中间","间件","安装"]
  对 "中间件" 查询无法精确命中 "中间件安装" 中的 "中间件" 完整词
  → bigram 整体召回率低且噪声多
```

### 6.2 向量索引 + Write-Ahead Log 持久化

**向量存储策略**（修复 v0.1 的 5 秒全量序列化问题）：

```
启动时：
  1. 读取 VECTOR_META → 获取维度和 provider 信息
  2. 遍历 VECTOR_WAL → 重建 VectorIndex（内存）
  3. 如果 WAL 为空 → 从所有 Observation 的 content 重新生成向量

运行时：
  1. 每次 add() 调用 → 追加一条 WAL 记录 + 更新内存索引
  2. 从不做全量序列化

关闭时：
  1. 无需特殊处理（启动时从 WAL 重建）

Eviction 时：
  1. compact_vector_wal() 清理已删除 obs 的 WAL 条目
```

```rust
// === memory/index/vector_index.rs ===

/// 全内存向量索引，使用余弦相似度检索。
/// 持久化策略采用 Write-Ahead Log（WAL），避免全量序列化。
pub struct VectorIndex {
    /// obs_id → Entry
    vectors: HashMap<String, VectorEntry>,
    /// 向量维度
    dimensions: usize,
}

struct VectorEntry {
    embedding: Vec<f32>,
    session_id: String,
}

impl VectorIndex {
    pub fn new(dimensions: usize) -> Self;
    
    /// 添加一条向量到内存索引。
    /// 调用者**必须额外**调用 MemDb::append_vector_wal() 做持久化。
    pub fn add(&mut self, obs_id: &str, session_id: &str, embedding: Vec<f32>);
    
    pub fn remove(&mut self, obs_id: &str);
    
    /// 余弦相似度搜索，返回 Top-K。
    /// 时间复杂度 O(N * dim)，N 为索引大小。
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<VectorHit>;
    
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn dimensions(&self) -> usize;
}
```

**性能预估**：

| 索引大小 | 向量维度 | 内存占用 | 搜索延迟（P50） | P99 |
|---|---|---|---|---|
| 1,000 | 384 | ~1.5 MB | < 1ms | 2ms |
| 10,000 | 384 | ~15 MB | 3ms | 10ms |
| 100,000 | 384 | ~150 MB | 30ms | 80ms |
| 500,000 | 384 | ~750 MB | 150ms | 400ms |

> 100K 条观测是一个合理的使用上限。超过此规模建议增加 HNSW 近似索引或使用外部向量数据库。

### 6.3 EmbeddingProvider

```rust
// === memory/index/embedding.rs ===

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    fn dimensions(&self) -> usize;
    /// 单条文本嵌入
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;
    /// 批量嵌入
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;
}

/// HTTP API 实现：调用本地 ollama / vllm / text-embeddings-inference
pub struct HttpEmbeddingProvider {
    url: String,
    model: String,
    api_key: Option<String>,
    dimensions: usize,
}
```

### 6.4 混合搜索 — HybridSearch

```rust
// === memory/index/hybrid_search.rs ===

use crate::memory::types::{SearchFilter, SearchHit};
use std::sync::RwLock;

const RRF_K: f64 = 60.0;

pub struct HybridSearch {
    bm25: Arc<RwLock<SearchIndex>>,
    vector: Arc<RwLock<Option<VectorIndex>>>,
    embedding: Option<Box<dyn EmbeddingProvider>>,
    db: Arc<MemDb>,
    bm25_weight: f64,      // 默认 0.4
    vector_weight: f64,    // 默认 0.6
}

impl HybridSearch {
    /// 混合搜索（BM25 + Vector），含降级策略。
    ///
    /// 降级阶梯：
    ///   1. BM25 + Vector（首选）
    ///   2. 仅 BM25（Vector 不可用时自动降级）
    ///   3. 空结果（BM25 也失败时）
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        filter: Option<SearchFilter>,
    ) -> anyhow::Result<Vec<SearchHit>> {
        let bm25_hits = self.bm25.read().unwrap().search(query, limit * 2);
        let vector_hits = self.search_vector(query, limit * 2).await;
        let merged = self.fuse_rrf(bm25_hits, vector_hits);
        let enriched = self.enrich(merged).await?;
        let filtered = self.apply_filter(enriched, filter);
        let diversified = self.diversify_by_session(filtered, 3);
        Ok(diversified.into_iter().take(limit).collect())
    }

    /// 向量搜索（降级安全）
    async fn search_vector(&self, query: &str, limit: usize) -> Vec<VectorHit> {
        let vec_guard = self.vector.read().unwrap();
        let Some(ref vec) = *vec_guard else { return vec![] };
        let Some(ref emb) = self.embedding else { return vec![] };
        match emb.embed(query).await {
            Ok(qv) => vec.search(&qv, limit),
            Err(e) => {
                tracing::warn!("Embedding failed, falling back to BM25-only: {e}");
                vec![]
            }
        }
    }

    /// RRF 融合：Reciprocal Rank Fusion
    fn fuse_rrf(bm25: Vec<BM25Hit>, vector: Vec<VectorHit>) -> Vec<FusedHit>;

    /// 加载完整的 Observation
    async fn enrich(&self, hits: Vec<FusedHit>) -> anyhow::Result<Vec<SearchHit>>;

    /// 应用过滤条件
    fn apply_filter(&self, hits: Vec<SearchHit>, filter: Option<SearchFilter>) -> Vec<SearchHit>;

    /// 按 Session 去重（同一 session 最多取 n 条）
    fn diversify_by_session(&self, hits: Vec<SearchHit>, max_per_session: usize) -> Vec<SearchHit>;
}
```

### 6.5 索引持久化

```rust
// === memory/index/persistence.rs ===

pub struct IndexPersistence {
    bm25: Arc<RwLock<SearchIndex>>,
    db: Arc<MemDb>,
    timer: Option<tokio::task::JoinHandle<()>>,
}

impl IndexPersistence {
    /// 启动 5 秒防抖保存循环。
    /// 仅持久化 BM25 索引（JSON 序列化）。向量索引通过 WAL 增量持久化。
    pub fn start(&self) {
        let bm25 = self.bm25.clone();
        let db = self.db.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                let data = bm25.read().unwrap().serialize();
                if let Err(e) = db.save_bm25_snapshot(&data) {
                    tracing::warn!("Failed to persist BM25 index: {e}");
                }
            }
        });
        // 存储 handle 以便关闭
    }

    /// 启动时恢复
    pub async fn load(&self) -> anyhow::Result<()> {
        if let Some(json) = self.db.load_bm25_snapshot()? {
            let restored = SearchIndex::deserialize(&json);
            let mut idx = self.bm25.write().unwrap();
            *idx = restored;
        }
        Ok(())
    }
}
```

---

## 7. 捕获管线

### 7.1 完整流程

```mermaid
sequenceDiagram
    participant AL as AgentLoop
    participant OB as observe()
    participant CP as Compressor
    participant DB as redb
    participant IX as Indexer
    participant WAL as Vector WAL

    AL->>OB: observe(PostToolUse, input, output)
    
    OB->>OB: strip_private_data()
    OB->>OB: 去重检查（LRU cache）
    OB->>DB: 保存 RawObservation
    
    OB->>CP: compress(raw)
    
    alt 默认：合成压缩
        CP->>CP: build_synthetic()
        Note over CP: 0 token 消耗，< 0.1ms
    else 可选：LLM 压缩（QIANXUN_MEM_LLM_COMPRESS=true）
        CP->>CP: 调 LLM → XML 结构化
    end
    
    CP->>DB: upsert_observation() 覆盖写入
    CP->>IX: index(obs)
    
    IX->>BM25: bm25.write().add(obs)
    
    alt 启用了嵌入
        IX->>EMB: embed(obs.content)
        EMB-->>IX: vector
        IX->>VEC: vec.write().add(obs_id, vector)
        IX->>WAL: append_vector_wal(obs_id, vector)
    else 未启用嵌入
        Note over IX: 跳过向量索引
    end

    Note over AL: 至此 Observation 已可检索<br/>延迟 < 5ms（合成压缩路径）
```

### 7.2 合成压缩算法

```rust
// === memory/pipeline/compressor.rs

impl Compressor {
    /// 从原始观测合成结构化 Observation。
    /// 不调用 LLM，通过工具类型启发式提取。
    pub fn build_synthetic(raw: &RawObservation) -> Observation {
        match (&raw.hook_type, raw.tool_name.as_deref()) {
            (HookType::PostToolUse, Some("read_file" | "file_read")) => {
                let path = extract_path(&raw.tool_input);
                Observation {
                    obs_type: ObservationType::FileRead,
                    title: format!("读取文件: {}", path),
                    facts: vec![format!("文件路径: {}", path)],
                    importance: 3,
                    ..self.base(raw)
                }
            }
            (HookType::PostToolUse, Some("write_file" | "file_write")) => {
                // ...
            }
            (HookType::PostToolUse, Some("edit_file" | "file_edit")) => {
                // 提取 diff 摘要
                let path = extract_path(&raw.tool_input);
                let summary = extract_diff_summary(&raw.tool_output);
                Observation {
                    obs_type: ObservationType::FileEdit,
                    title: format!("编辑 {}: {}", path, summary),
                    importance: 6,
                    ..self.base(raw)
                }
            }
            (HookType::PostToolUse, Some("terminal" | "command_run")) => {
                let cmd = extract_cmd(&raw.tool_input);
                let is_err = has_error(&raw.tool_output);
                Observation {
                    obs_type: if is_err { ObservationType::Error } else { ObservationType::CommandRun },
                    title: format!("{}: {}", if is_err { "命令报错" } else { "执行命令" }, cmd),
                    importance: if is_err { 8 } else { 4 },
                    ..self.base(raw)
                }
            }
            (HookType::PostToolUse, Some("grep" | "search")) => {
                let query = extract_query(&raw.tool_input);
                Observation {
                    obs_type: ObservationType::Search,
                    title: format!("搜索: {}", query),
                    importance: 2,
                    ..self.base(raw)
                }
            }
            (HookType::PostToolUse, Some(tool)) => {
                Observation {
                    title: format!("调用 {}", tool),
                    importance: 3,
                    ..self.base(raw)
                }
            }
            (HookType::PromptSubmit, _) => {
                let summary = raw.user_prompt
                    .as_deref().unwrap_or("")
                    .lines().next().unwrap_or("");
                Observation {
                    obs_type: ObservationType::Conversation,
                    title: truncate(summary, 80),
                    importance: 3,
                    ..self.base(raw)
                }
            }
            _ => Observation {
                obs_type: ObservationType::Other,
                importance: 1,
                ..self.base(raw)
            }
        }
    }
    
    fn base(&self, raw: &RawObservation) -> Observation {
        Observation {
            id: raw.id.clone(),
            session_id: raw.session_id.clone(),
            timestamp: raw.timestamp,
            subtitle: None,
            facts: vec![],
            narrative: String::new(),
            concepts: extract_concepts(raw),
            files: extract_files(raw),
            confidence: Some(0.8),
            ..Default::default() // obs_type, title, importance 由上层填充
        }
    }
}
```

### 7.3 去重

```rust
// === memory/pipeline/dedup.rs

use lru::LruCache;

/// 同一 session 中相同 tool_name + tool_input 的调用视为重复。
pub struct DedupMap {
    /// key: (session_id, tool_name, tool_input_hash)
    seen: LruCache<(String, String, u64), ()>,
}

impl DedupMap {
    pub fn new(capacity: usize) -> Self;
    
    pub fn check_and_mark(
        &mut self,
        session_id: &str,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> bool {
        let hash = self.hash_input(input);
        let key = (session_id.to_string(), tool_name.to_string(), hash);
        if self.seen.contains(&key) {
            return true;  // duplicate
        }
        self.seen.put(key, ());
        false  // not duplicate
    }
}
```

---

## 8. 检索管线

### 8.1 上下文构建

```rust
// === memory/core.rs — MemoryCore::build_context()

impl MemoryCore {
    /// 根据用户消息构建记忆上下文文本块，按 token 预算裁剪。
    ///
    /// 优先级：Pinned Slots > 当前会话摘要 > 相关 Observation > 高热度 Memory
    /// 总预算上限：`token_budget`（默认 4000 tokens → ~16000 字符）
    pub async fn build_context(
        &self,
        query: &str,
        token_budget: u32,
    ) -> String {
        let mut blocks: Vec<ContextBlock> = Vec::new();
        let mut tokens_used = 0u32;

        // 1. Pinned 工作记忆插槽（最高优先级）
        for slot in &self.pinned_slots {
            let text = format!("## [Slot: {}]\n{}\n(---slots end---)", slot.label, slot.content);
            let tokens = estimate_tokens(&text);
            if tokens_used + tokens <= token_budget {
                blocks.push(ContextBlock {
                    block_type: ContextBlockType::Slot,
                    content: text,
                    estimated_tokens: tokens,
                    source_ids: vec![slot.label.clone()],
                });
                tokens_used += tokens;
            }
        }

        // 2. 当前会话摘要（如有）
        if let Some(session_id) = self.current_session_id.as_ref() {
            if let Ok(Some(summary)) = self.db.get_session_summary(session_id) {
                let text = format!("## 当前会话摘要\n{}", summary.narrative);
                let tokens = estimate_tokens(&text);
                if tokens_used + tokens <= token_budget {
                    blocks.push(ContextBlock { content: text, .. });
                    tokens_used += tokens;
                }
            }
        }

        // 3. 混合搜索结果
        let filter = SearchFilter { project: self.project.clone(), .. };
        if let Ok(hits) = self.hybrid_search.search(query, 10, Some(filter)).await {
            for hit in &hits {
                let text = format!(
                    "[{}] {} — {}",
                    format!("{:?}", hit.observation.obs_type),
                    hit.observation.title,
                    truncate(&hit.observation.narrative, 200),
                );
                let tokens = estimate_tokens(&text);
                if tokens_used + tokens <= token_budget {
                    blocks.push(ContextBlock {
                        block_type: ContextBlockType::Observation,
                        content: text,
                        estimated_tokens: tokens,
                        source_ids: vec![hit.observation.id.clone()],
                    });
                    tokens_used += tokens;
                }
            }
        }

        // 4. 高热度记忆（top N by strength × access_count）
        for mem in &self.top_memories(5) {
            let text = format!("【{}】{}: {}", format!("{:?}", mem.mem_type), mem.title, mem.content);
            let tokens = estimate_tokens(&text);
            if tokens_used + tokens <= token_budget {
                blocks.push(ContextBlock {
                    block_type: ContextBlockType::Memory,
                    content: text,
                    estimated_tokens: tokens,
                    source_ids: vec![mem.id.clone()],
                });
                tokens_used += tokens;
            }
        }

        if blocks.is_empty() {
            return String::new();
        }

        let header = "\n\n## 记忆上下文（自动注入的上次会话记忆）\n".to_string();
        let body: String = blocks.iter().map(|b| b.content.clone()).collect::<Vec<_>>().join("\n\n");
        let footer = "\n(---memory end---)".to_string();
        format!("{}{}{}", header, body, footer)
    }
}

/// Token 估算：简单字符 / 3.5 规则
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as f64 / 3.5).ceil() as u32
}
```

### 8.2 Token 预算管理

注入到 system prompt 的记忆内容会占用上下文窗口。千寻已有 `compact_window` 和 `compact_config` 做预算管理，记忆注入需要与其协调：

```
上下文窗口分布（以 128K 模型为例）：
  ┌──────────────────────────────────────────────┐
  │ System prompt (base)             ~500 tokens  │
  │ 工作区上下文                      ~200 tokens  │
  │ 技能目录                          ~200 tokens  │
  │ 技能指令                           ~500 tokens  │
  ├──────────────────────────────────────────────┤
  │ 记忆上下文（build_context）      ~4000 tokens  │ ← 预算
  ├──────────────────────────────────────────────┤
  │ 对话消息                          ~80K tokens  │
  │ 预留输出                           ~8K tokens  │
  └──────────────────────────────────────────────┘
  总计: ~93K tokens < 128K 安全窗口
```

### 8.3 `memory_recall` Agent Tool

```rust
// === tools/memory_tools.rs

pub struct MemoryRecall {
    core: Arc<MemoryCore>,
}

impl AgentTool for MemoryRecall {
    fn name(&self) -> &str { "memory_recall" }
    fn description(&self) -> String {
        "搜索过去会话中的记忆。参数：query（搜索关键词）".into()
    }

    async fn call(&self, input: serde_json::Value) -> Result<ToolResult, ToolError> {
        let query = input["query"].as_str().ok_or(ToolError::InvalidInput("query 字段必填"))?;
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let results = self.core.search(query, limit).await?;
        let formatted: Vec<String> = results.iter().map(|r| {
            format!("[{}] {}\n  {}", format!("{:?}", r.observation.obs_type), r.observation.title, r.observation.narrative)
        }).collect();
        Ok(ToolResult::text(formatted.join("\n\n")))
    }
}

pub struct MemoryRemember {
    core: Arc<MemoryCore>,
}

impl AgentTool for MemoryRemember {
    fn name(&self) -> &str { "memory_remember" }
    fn description(&self) -> String {
        "手动保存一条跨会话的持久记忆。参数：content（记忆内容）, type（类型）, ttl_days（可选）".into()
    }

    async fn call(&self, input: serde_json::Value) -> Result<ToolResult, ToolError> {
        let content = input["content"].as_str().ok_or(ToolError::InvalidInput("content 必填"))?;
        let mem_type = input.get("type").and_then(|v| v.as_str()).unwrap_or("fact");
        let memory = self.core.remember(content, mem_type, None).await?;
        Ok(ToolResult::text(format!("记忆已保存: {}", memory.title)))
    }
}
```

---

## 9. 工作记忆 — Memory Slots

```rust
// === memory/slots.rs

/// Memory Slot 是可编辑的固定大小记忆块，注入到每次会话的 system prompt。
pub struct SlotManager {
    db: Arc<MemDb>,
}

impl SlotManager {
    pub fn new(db: Arc<MemDb>) -> Self;

    pub async fn list_slots(&self) -> Vec<MemorySlot>;
    
    /// 列出 pinned 插槽（始终注入 system prompt）
    pub async fn list_pinned_slots(&self) -> Vec<MemorySlot>;
    
    /// 追加内容（不超 size_limit）
    pub async fn append(&self, label: &str, content: &str) -> anyhow::Result<()>;
    
    /// 替换全部内容
    pub async fn replace(&self, label: &str, content: &str) -> anyhow::Result<()>;
    
    /// 创建新插槽
    pub async fn create(&self, label: &str, desc: &str, size_limit: usize) -> anyhow::Result<()>;
    
    pub async fn delete(&self, label: &str) -> anyhow::Result<()>;
}
```

### 常用插槽示例

| label | 用途 | 默认尺寸 |
|---|---|---|
| `project_overview` | 项目概述 | 2000 字符 |
| `coding_style` | 编码风格偏好 | 1000 字符 |
| `current_ticket` | 当前任务上下文 | 2000 字符 |
| `decision_log` | 关键决策记录 | 2000 字符 |

---

## 10. 隐私清洗

```rust
// === memory/pipeline/privacy.rs

/// 从工具输入/输出中清洗敏感信息。
///
/// 覆盖模式：
/// - API Key: sk-xxx, pk-xxx 等
/// - 令牌: eyJxxx (JWT), ghp_xxx (GitHub PAT)
/// - 密码: postgres://user:password@host
/// - AWS 密钥: AKIAxxx
/// - 私钥: -----BEGIN (RSA/EC/OPENSSH) PRIVATE KEY-----
/// - 连接字符串: 各种 DSN 中的密码字段
/// - 通用: 环境变量赋值中的敏感值
pub fn strip_private_data(input: &str) -> String {
    let mut result = input.to_string();
    
    // 1. API Keys: sk-xxx (OpenAI), pk-xxx, 等
    result = regex_replace_all(r#"(?i)(["']?\b(?:sk|pk|sk-ant|fkey|ghp|gho|ghu|ghs|ghr)[_-][a-zA-Z0-9]{20,})"#, &result, "$1_REDACTED");
    
    // 2. JWT tokens: eyJxxx.yyy.zzz
    result = regex_replace_all(r#"\beyJ[a-zA-Z0-9-_]+\.[a-zA-Z0-9-_]+\.[a-zA-Z0-9-_]+\b"#, &result, "JWT_REDACTED");
    
    // 3. 连接字符串密码: postgres://user:password@host  →  postgres://user:REDACTED@host
    result = regex_replace_all(r#"(://[^:@/]+):([^@/]+)@"#, &result, "${1}:REDACTED@");
    
    // 4. AWS 密钥
    result = regex_replace_all(r#"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"#, &result, "AWS_KEY_REDACTED");
    
    // 5. 私钥块
    result = regex_replace_all(r#"-----BEGIN[ A-Z]+PRIVATE KEY-----[\s\S]*?-----END[ A-Z]+PRIVATE KEY-----"#, &result, "---PRIVATE KEY REDACTED---");
    
    // 6. 环境变量中的敏感值 export KEY=secret123
    result = regex_replace_all(
        r#"(?i)\b(export\s+)?(?:API_KEY|SECRET|PASSWORD|TOKEN|ACCESS_KEY)\s*[=:]\s*['"]?[a-zA-Z0-9_\-!@#$%^&*()]{8,}['"]?"#,
        &result,
        "${1}${2}=REDACTED",
    );
    
    result
}

fn regex_replace_all(pattern: &str, text: &str, replacement: &str) -> String {
    // 使用 regex crate
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_strip_openai_key() {
        let result = strip_private_data("api_key = \"sk-proj-1234567890abcdef1234567890abcdef\"");
        assert!(!result.contains("sk-proj-1234567890abcdef1234567890abcdef"));
        assert!(result.contains("sk-proj-REDACTED"));
    }

    #[test]
    fn test_strip_postgres_url() {
        let result = strip_private_data("postgres://admin:supersecret@localhost:5432/db");
        assert!(!result.contains("supersecret"));
        assert!(result.contains("REDACTED"));
    }

    #[test]
    fn test_strip_private_key() {
        let pk = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
        let result = strip_private_data(pk);
        assert!(result.contains("PRIVATE KEY REDACTED"));
    }

    #[test]
    fn test_skipped_normal_text() {
        let text = "读取文件 src/main.rs，内容为 fn main() { println!(\"hello\"); }";
        assert_eq!(strip_private_data(text), text);
    }
}
```

---

## 11. 周期任务

### 11.1 Consolidation（Observation → Memory）

两阶段设计：

```
Phase 1: 统计聚类（无 LLM，每次 Session End 触发）
  1. 扫描当前 session 的所有 Observation
  2. 按 concepts 交集 > 0.5 聚类
  3. 对每个聚类：
     a. 计算平均 importance
     b. 如果 avg > 6 → 生成 Memory（type=Pattern/Fact）
     c. 检查 Jaccard 相似度 > 0.7 → 版本升级
     d. 更新访问计数

Phase 2: LLM 深度归纳（可选，可配置间隔）
  1. 收集过去 N 个 session 的 top-50 observations
  2. 调 LLM 提取：
     - 架构决策（type=Architecture）
     - 用户偏好（type=Preference）
     - 常见缺陷模式（type=Bug）
     - 工作流（type=Workflow）
  3. 生成的 Memory 标记 high priority
```

### 11.2 Eviction

```rust
/// 启动时 + 每 24 小时执行
async fn eviction_cycle(core: &MemoryCore) {
    // 1. TTL 过期
    for mem in core.db.list_memories()? {
        if let Some(expires) = mem.forget_after {
            if Utc::now() > expires {
                core.forget(&mem.id).await?;
            }
        }
    }

    // 2. 低热度清理：strength < 3 + access_count == 0 + 创建超过 30 天
    for mem in core.db.list_memories()? {
        if mem.strength < 3 && mem.access_count == 0 {
            let age = Utc::now() - mem.created_at;
            if age.num_days() > 30 {
                core.forget(&mem.id).await?;
            }
        }
    }

    // 3. compact vector WAL（清理已删除 obs 的遗留条目）
    let active_ids: HashSet<String> = /* 所有活跃 obs_id */;
    core.db.compact_vector_wal(&active_ids)?;
}
```

### 11.3 Retention Scoring

每次 `search()` 或 `context()` 访问一条记忆时：

```rust
impl MemoryCore {
    async fn record_access(&self, memory_id: &str) {
        if let Ok(Some(mut mem)) = self.db.get_memory(memory_id) {
            mem.access_count += 1;
            mem.last_accessed_at = Some(Utc::now());
            mem.strength = (mem.strength as f64 * 0.9 + 1.0).min(10.0) as u8;
            let _ = self.db.upsert_memory(&mem);
        }
    }
}

// 在 build_context() 和 search() 中调用
```

---

## 12. Server 模式 — VPS 节点发现 + 控制中台

### 12.1 架构总览

VPS 只做**控制面**（用户管理 + 节点发现 + 命令中转），不做**数据面**（不存代码、不存记忆、不调 LLM）。

```
VPS（公网可达）
└─ qx server
    ├─ 用户管理（管理员注册用户）
    ├─ 设备授权（OAuth 式 Web 授权）
    ├─ WebSocket 命令中转
    ├─ Web UI（登录 / 授权 / 管理）
    └─ 数据库（用户 + 设备 + 授权码）

Windows 开发机              Linux 开发机              App / Web
└─ qx node                 └─ qx node               └─ qx app
   ├─ AgentLoop（本地）       ├─ AgentLoop（本地）       ├─ 登录
   ├─ MemoryCore（本地）      ├─ MemoryCore（本地）      ├─ 查看节点
   ├─ 文件 I/O（本地）        ├─ 文件 I/O（本地）        ├─ 发送命令
   ├─ 本地工具（git等）       ├─ 本地工具（docker等）    └─ 接收结果
   └─ WS → VPS               └─ WS → VPS
```

### 12.2 VPS Server 不做的事

| 操作 | 谁做 | VPS 不做 |
|---|---|---|
| Agent 推理 | 开发机本地 | ❌ |
| 记忆存储 | 开发机本地 | ❌ |
| 文件读写 | 开发机本地 | ❌ |
| LLM API 调用 | 开发机本地 | ❌ |
| **用户管理** | **VPS** | ✅ |
| **节点发现** | **VPS** | ✅ |
| **命令中转** | **VPS** | ✅ |
| **消息理解/路由决策** | — | ❌ 只按 target 字段透传 |

VPS 无状态化：重启后设备 token 和用户数据持久化在数据库，节点重连 WS 即可恢复。

### 12.3 数据库模型

```rust
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,        // Admin | User
    pub created_at: DateTime<Utc>,
}

pub struct Device {
    pub id: String,
    pub host_id: String,       // "windows-pc"
    pub user_id: String,       // 所属用户
    pub token_hash: String,    // 设备 token 哈希
    pub host_type: String,     // "windows" | "linux" | "macos"
    pub projects: Vec<String>, // ["qianxun", "myblog"]
    pub workers: u32,
    pub caps: Vec<String>,     // ["read_file","terminal","git"]
    pub status: DeviceStatus,  // online | offline
    pub last_seen: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

pub struct AuthCode {
    pub code: String,
    pub device_id: String,
    pub user_id: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub status: AuthCodeStatus, // Pending | Authorized | Expired
}
```

### 12.4 设备授权流程

```
Step 1: 管理员在 Web UI 创建用户
Step 2: 用户在 Web UI 登录
Step 3: 用户在开发机执行 qx node auth

开发机 (Windows)           用户 (浏览器/手机)            VPS
──────────────             ────────────────          ──────────
qx node auth
  │ POST /api/device/auth-code                       │
  │← 返回 code=xyz + URL                              │
  │                                                   │
  │ 打印:                                              │
  │ "请打开以下链接授权此设备："                         │
  │ "https://vps/qx/authorize?code=xyz"                │
  │                                                   │
  │                        打开链接                     │
  │                          │                         │
  │                        Web UI 显示：                │
  │                        "授权设备 windows-pc？"      │
  │                        "能力: 文件系统/终端/Git"    │
  │                        "属于: user@example.com"     │
  │                          │                         │
  │                        点击 [授权]                   │
  │                        POST /api/device/authorize   │
  │                          │                         │
  │ 轮询 token                                          │
  │ GET /api/device/token?code=xyz                      │
  │← 返回 device_token                                  │
  │                                                     │
  │ 保存 token 到 ~/.qx/node.token                       │
  │ WS 连接: wss://vps/ws?token=dt_xxx                  │
```

| 凭证 | 有效期 | 用途 |
|---|---|---|
| 授权码 | 5 分钟 | 换取设备 token |
| 设备 token | 长期（可吊销） | WS 连接认证，绑定 user + device |
| 用户 JWT | 24 小时 | Web UI / App 登录 |

### 12.5 多用户隔离

```
用户 A 登录 App 后：
  → 只能看到自己的节点（windows-pc）
  → 只能给自己的节点发命令
  → 不知道用户 B 的存在

用户 B 登录 App 后：
  → 只能看到自己的节点（linux-vm）
  → 只能给自己的节点发命令

管理员登录 Web UI 后：
  → 可以看到所有用户和所有节点
  → 可以创建/禁用用户
  → 可以吊销设备 token
```

### 12.6 WebSocket 消息协议

```json
// Node 连接（带 token）
→ {"type":"auth","token":"dt_xxxxx"}
← {"type":"auth_ok","device_id":"d_abc","user_id":"u_123"}

// Node 注册能力和状态
→ {"type":"register","projects":["qianxun","myblog"],"caps":["read_file","terminal","git"]}
→ {"type":"status","status":"busy","workers":2}

// App 连接（带 JWT）
→ {"type":"auth","token":"eyJ..."}
← {"type":"node_list","nodes":[
    {"host_id":"windows-pc","projects":["qianxun"],"workers":2,"status":"online"},
    {"host_id":"linux-vm","projects":["ecommerce"],"workers":1,"status":"online"}
  ]}

// App → Node: 命令
→ {"type":"command","target":"windows-pc","seq":1,
   "payload":{"action":"read_file","path":"C:\\dev\\README.md"}}
// VPS 验证 target 属于当前用户 → 转发
→ (to windows-pc) {"type":"command","from":"app","seq":1,"payload":{...}}
← (from windows-pc) {"type":"command_result","seq":1,"data":{"content":"# qianxun\n..."}}
// VPS 转发回 App
← {"type":"command_result","host":"windows-pc","seq":1,"data":{"content":"# qianxun\n..."}}
```

### 12.7 VPS API 清单

```
公开:
  POST   /api/auth/login             登录 → 返回 JWT
  POST   /api/device/auth-code       生成授权码
  POST   /api/device/authorize       用户确认授权
  GET    /api/device/token           轮询获取 token

管理员:
  POST   /api/admin/users            创建用户
  GET    /api/admin/users            用户列表
  DELETE /api/admin/users/:id        禁用用户

Web UI:
  GET    /login                      登录页
  GET    /authorize?code=xxx         授权确认页
  GET    /admin/users                用户管理
  GET    /dashboard                  节点总览

WebSocket:
  wss://vps:8313/ws                  设备或 App 连接
```

### 12.8 离线降级

```
有网时：
  Node → VPS（App 可见，可远程控制）

断网时：
  Node 检测 WS 断开
  → 自动降级为纯本地模式（AgentLoop 正常工作）
  → 记忆写入本地 MemoryCore
  → 网络恢复后自动重连 WS
  → 不尝试同步离线期间的记忆（VPS 不存记忆）
```

### 12.9 对项目的 crate 影响

```
现有 crate 不变：
  qianxun-core      核心类型、trait
  qianxun-memory    记忆引擎
  qianxun-cli       本地终端

新增 crate：
  qianxun-server    运行在 VPS 的 Web 服务 + WS 中转
  qianxun-app       App 后端 API（或由 server 内置）
  qianxun-node      开发机守护进程（连接 VPS + 本地 AgentLoop）
## 13. 集成方案 — Crate 间协作

### 13.1 组装流程

```
工作区内的组装职责：

qianxun-core          qianxun-memory           qianxun-cli / qianxun-daemon
─────────────         ──────────────            ──────────────────────────────
                      1. MemoryCore::open()    2. 创建 MemoryCore
                         → 打开 redb             → qianxun-memory 依赖
                         → 重建索引               → 注入 memory_observer
                         → 初始化 SlotManager
                      3. 实现 MemoryObserver    4. cli Repl::new()
                         → observe()               → 持有 observer
                         → build_context()         → 传给 AgentLoop
                         → search()             5. 注入 observer
                         → remember()               → AgentLoop.memory_observer
                         → session_start/end()      → handle_user_message()
```

### 13.2 AgentLoop 改动

```diff
+ use qianxun_core::memory::MemoryObserver;

 pub struct AgentLoop {
     pub state: AgentState,
     pub turn_count: u32,
     pub retry_count: u32,
     pub config: AgentConfig,
     pub accumulated_usage: TokenUsage,
     pub compact_window: Option<CompactWindow>,
     pub compact_config: Option<CompactConfig>,
+    pub memory_observer: Option<Box<dyn MemoryObserver>>,
 }
```

### 13.3 handle_user_message 内的 observe 调用

```diff
  // 工具执行前
+ if let Some(ref mo) = agent.memory_observer {
+     mo.observe(HookType::PreToolUse, &name, Some(args.clone()), None).await;
+ }

  match tools.execute_async(name, args.clone()).await {
      Ok(output) => {
+         if let Some(ref mo) = agent.memory_observer {
+             mo.observe(HookType::PostToolUse, &name, None, Some(&output.content)).await;
+         }
          results.push((id.clone(), output.content, output.is_error));
      }
      Err(e) => {
+         if let Some(ref mo) = agent.memory_observer {
+             mo.observe(HookType::PostToolFailure, &name, None, Some(&e.to_string())).await;
+         }
          results.push((id.clone(), format!("Error: {e}"), true));
      }
  }
```

### 13.4 cli.rs 组装代码

```rust
// cli.rs — Repl::new()

use qianxun_memory::MemoryCore;

let memory_observer: Option<Box<dyn MemoryObserver>> = if let Some(ref root) = ws_root {
    let data_dir = dirs::home_dir().unwrap_or_default().join(".qianxun");
    match MemoryCore::open(data_dir, root.to_string_lossy().to_string()) {
        Ok(core) => Some(Box::new(core) as Box<dyn MemoryObserver>),
        Err(e) => {
            tracing::warn!("Failed to init memory: {e}");
            None
        }
    }
} else {
    None
};

// 注入 AgentLoop
agent_loop.memory_observer = memory_observer;

// 构建记忆上下文（每次 handle_message）
let memory_context = agent_loop.memory_observer
    .as_ref()
    .map(|mo| mo.build_context(&msg, 4000).await)
    .unwrap_or_default();

// 传入 handle_user_message（已有参数，不变）
handle_user_message(
    &mut agent_loop,
    &mut conversation,
    &provider,
    &tools,
    &sink,
    &memory_context,  // ← 已有
    &skills_catalog,
    &skill_injections,
    cancel_flag,
).await;
```

---

## 14. Crate 结构

```
qianxun/
├── Cargo.toml                         # workspace root
├── qianxun-core/
│   ├── Cargo.toml
│   └── src/
│       ├── memory.rs                  # 【新增】MemoryObserver trait（仅此一个文件）
│       ├── ...                        # 不变
│
├── qianxun-memory/                    # 【新增】独立 crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                     # MemoryCore 主入口
│       ├── types.rs                   # Session, Observation, Memory 等
│       ├── storage/
│       │   ├── mod.rs
│       │   ├── db.rs                  # MemDb — redb 封装
│       │   ├── schema.rs              # 表定义
│       │   └── file_store.rs          # 【新增】本地文件存储
│       ├── index/
│       │   ├── mod.rs
│       │   ├── search_index.rs        # BM25
│       │   ├── vector_index.rs        # 余弦相似度
│       │   ├── hybrid_search.rs       # RRF 融合 + 降级
│       │   ├── embedding.rs           # EmbeddingProvider trait
│       │   └── persistence.rs         # BM25 5s 防抖 + 向量 WAL
│       ├── pipeline/
│       │   ├── mod.rs
│       │   ├── compressor.rs
│       │   ├── compressor_synthetic.rs
│       │   ├── privacy.rs
│       │   ├── dedup.rs
│       │   ├── consolidation.rs
│       │   ├── eviction.rs
│       │   └── retention.rs
│       ├── slots.rs
│       └── daemon.rs                  # 【新增】Memory Daemon（Daemon IPC 服务）
│
├── qianxun-cli/
│   ├── Cargo.toml                     # + qianxun-memory 依赖
│   └── src/cli.rs                     # 创建 MemoryCore + 注入 AgentLoop
│
├── qianxun-acp/
│   ├── Cargo.toml                     # + qianxun-memory 依赖
│   └── src/...
│
└── qianxun-daemon/                    # 【Phase 4】Daemon 二进制
    ├── Cargo.toml
    └── src/main.rs
```

---

## 15. 依赖清单

### 15.1 Workspace Cargo.toml

```toml
# qianxun/Cargo.toml
[workspace]
members = ["qianxun-core", "qianxun-memory", "qianxun-cli", "qianxun-acp"]

[workspace.dependencies]
# ... 现有 ...

# 新增 workspace 依赖
redb = "4.1"
rust-stemmers = "1"
jieba-rs = "0.7"
regex = "1"
```

### 15.2 qianxun-memory/Cargo.toml

```toml
[package]
name = "qianxun-memory"
version.workspace = true
edition.workspace = true

[dependencies]
qianxun-core = { path = "../qianxun-core" }
redb = { workspace = true }
rust-stemmers = { workspace = true }
jieba-rs = { workspace = true }
regex = { workspace = true }
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
chrono = { workspace = true, features = ["serde"] }
uuid = { workspace = true }
lru = { workspace = true }
notify = { workspace = true }     # file watcher（Daemon 模式）
```

### 15.3 qianxun-core 不受影响

`qianxun-core/Cargo.toml` 不增加任何依赖。新增的 `memory.rs` 只引入 `async-trait`（已有）和 `serde_json`（已有）。

### 15.4 传递依赖检查

| crate | 传递依赖 | 合规 |
|---|---|---|
| `redb` | ~30 | ✅ < 100 |
| `jieba-rs` | ~20 | ✅ |
| `rust-stemmers` | ~3 | ✅ |
| `regex` | ~5 | ✅ |
| 全部合计 | < 60 | ✅ |

---

## 16. 里程碑建议

### Phase 3a — 独立 crate + 核心存储 + 基础捕获（3 周）

| 任务 | crate | 预估 |
|---|---|---|
| 创建 `qianxun-memory` crate + workspace 配置 | workspace | 0.5 天 |
| `MemoryObserver` trait（qianxun-core 单文件） | core | 0.5 天 |
| MemDb + 表定义 | memory | 1 天 |
| 数据类型定义 | memory | 1 天 |
| BM25 + jieba CJK 分词 | memory | 3 天 |
| 合成压缩 | memory | 2 天 |
| 隐私清洗 | memory | 1 天 |
| 本地文件存储 + 文件格式 | memory | 1 天 |
| cli 集成（组装 + 注入） | cli | 1 天 |
| AgentLoop observe 钩子 | core | 1 天 |
| 单元测试 | memory | 2 天 |

### Phase 3b — 语义检索（2 周）

| 任务 | crate | 预估 |
|---|---|---|
| VectorIndex | memory | 2 天 |
| EmbeddingProvider + HTTP 实现 | memory | 2 天 |
| 向量 WAL | memory | 1 天 |
| HybridSearch（RRF + 降级 + 过滤） | memory | 3 天 |
| IndexPersistence | memory | 1 天 |
| build_context() + SlotManager | memory | 2 天 |
| memory_recall Tool | memory | 1 天 |
| 集成测试 | — | 2 天 |

### Phase 3c — 高级功能 + Daemon 预留（2 周）

| 任务 | crate | 预估 |
|---|---|---|
| Consolidation 两阶段 | memory | 3 天 |
| Eviction + WAL compact | memory | 2 天 |
| Retention scoring | memory | 1 天 |
| memory_remember Tool | memory | 1 天 |
| MemoryDaemon IPC 结构 | memory | 2 天 |
| 文件同步（file watcher） | memory | 2 天 |
| 旧 .md 迁移 | cli | 1 天 |
| 集成测试 + 压测 | — | 3 天 |

---

## 附录 A: v0.1 → v0.3 主要改进对照

| 问题 | v0.1 | v0.2 | v0.3 |
|---|---|---|---|
| **AgentLoop 集成** | 无 | 5 个集成点 + diff | 改用 MemoryObserver trait |
| **向量持久化** | JSON+Base64 150MB | WAL + 重建 | — |
| **CJK 搜索** | bigram | jieba-rs | — |
| **隐私清洗** | 无 | 正则方案 | — |
| **并发模型** | Mutex | Arc<RwLock> | — |
| **降级容错** | 无 | BM25-only | — |
| **Memory 耦合** | 嵌入 core | 仍嵌入 core | ✅ **独立 crate** |
| **存储** | 仅 redb | 仅 redb | ✅ **redb + 本地文件** |
| **Daemon 模式** | — | — | ✅ **MemoryDaemon + IPC + file watcher** |
| **AgentLoop 持有** | — | core::MemoryCore | ✅ **Box<dyn MemoryObserver>** |

## 附录 B: 文件变动清单

### 新增文件

| 文件 | 说明 |
|---|---|
| `qianxun-core/src/memory.rs` | `MemoryObserver` trait（~30 行） |
| `qianxun-memory/Cargo.toml` | 新 crate |
| `qianxun-memory/src/lib.rs` | `MemoryCore` 主入口 + `impl MemoryObserver` |
| `qianxun-memory/src/types.rs` | 数据类型 |
| `qianxun-memory/src/storage/db.rs` | MemDb（redb 封装） |
| `qianxun-memory/src/storage/schema.rs` | 表定义 |
| `qianxun-memory/src/storage/file_store.rs` | 本地文件存储 |
| `qianxun-memory/src/index/search_index.rs` | BM25 |
| `qianxun-memory/src/index/vector_index.rs` | 向量索引 |
| `qianxun-memory/src/index/hybrid_search.rs` | 混合搜索 |
| `qianxun-memory/src/index/embedding.rs` | EmbeddingProvider |
| `qianxun-memory/src/index/persistence.rs` | 索引持久化 |
| `qianxun-memory/src/pipeline/*.rs` | 7 个管线文件 |
| `qianxun-memory/src/slots.rs` | 工作记忆插槽 |
| `qianxun-memory/src/daemon.rs` | Daemon IPC 服务 |

### 修改文件

| 文件 | 改动 |
|---|---|
| `Cargo.toml`（workspace） | 加入 `qianxun-memory` member + workspace deps |
| `qianxun-core/src/lib.rs` | 增加 `pub mod memory;` |
| `qianxun-core/src/agent/engine.rs` | AgentLoop 增加 `memory_observer` 字段 + observe 钩子 |
| `qianxun-cli/Cargo.toml` | + `qianxun-memory` 依赖 |
| `qianxun-cli/src/cli.rs` | 替换 `MemoryManager` → `MemoryCore` |
| `qianxun-acp/Cargo.toml` | + `qianxun-memory` 依赖 |

### 删除文件

| 文件 | 原因 |
|---|---|
| `qianxun-core/src/context/memory.rs` | file-based MemoryManager 废弃 |




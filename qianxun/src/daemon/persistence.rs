//! Session 持久化 — Daemon 端的 session 元数据 + conversation 快照 + 事件流.
//!
//! # 设计 (见 docs/30_子项目规划/01-daemon.md §6)
//!
//! 3 张新表 (`daemon_` 前缀, 不与 `qianxun-memory` 的 `sessions` / `observations` 冲突):
//! - `daemon_sessions` — session 元数据 + 状态机
//! - `daemon_conversation_snapshots` — 增量 conversation 快照
//! - `daemon_event_log` — 事件流 (SSE transcript)
//!
//! # Stage 3 简化
//!
//! - 每 5 个 message 写一次 snapshot, 60s 写一次 (Stage 4 优化增量)
//! - 不接 SQLite 加密 (Stage 4 才是)
//! - 不接 compaction / 压缩 (后续)
//! - 不接 VPS 同步 (Stage 4 才是)
//!
//! # 关键 API
//!
//! - `create` / `list_active` — session CRUD
//! - `save_snapshot` / `load_latest_snapshot` — 增量 conversation
//! - `append_event` / `load_events` — 事件流
//!
//! 所有 SQLite 操作都通过 `spawn_blocking` 派发到 blocking 线程池 (设计文档 §4.3.4),
//! 避免在 async 上下文中持 std::sync::Mutex 锁阻塞 tokio reactor.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use qianxun_core::agent::conversation::Conversation;
use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

/// Session 持久化错误.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("conversation format error: {0}")]
    ConversationFormat(#[from] qianxun_core::agent::conversation::ConversationFormatError),

    #[error("connection lock poisoned")]
    LockPoisoned,
}

impl From<std::sync::PoisonError<std::sync::MutexGuard<'_, Connection>>> for SessionStoreError {
    fn from(_: std::sync::PoisonError<std::sync::MutexGuard<'_, Connection>>) -> Self {
        SessionStoreError::LockPoisoned
    }
}

/// Session 元数据 (用于 `list_active`).
#[derive(Debug, Clone)]
#[allow(dead_code)] // 部分字段 Stage 4 才被 `restore_from_disk` 完整使用
pub struct SessionMeta {
    pub id: String,
    pub project_root: Option<String>,
    pub status: String,
    pub created_at: String,
    pub last_active_at: String,
    pub message_count: u32,
}

/// 事件日志条目 (用于 `load_events`).
#[derive(Debug, Clone)]
#[allow(dead_code)] // 字段在 Stage 4 恢复路径才被读
pub struct EventEntry {
    pub seq: u32,
    pub event_type: String,
    pub event_json: String,
}

/// SessionStore — 3 张表 CRUD 封装.
///
/// 设计为 `Arc<SessionStore>`, 多个 task (prompt_handler / 恢复 / 管理 API)
/// 共享同一份连接. 内部 `db: Mutex<Connection>` 串行化写, 配合
/// `spawn_blocking` 使用.
pub struct SessionStore {
    db: Arc<Mutex<Connection>>,
}

impl SessionStore {
    /// 暴露共享 connection (给 MVP-3 KanbanDb 复用 daemon.db, v6 §7.3 决策).
    pub fn db_arc(&self) -> std::sync::Arc<std::sync::Mutex<rusqlite::Connection>> {
        self.db.clone()
    }

    /// 打开 / 创建 SQLite 数据库, 初始化 3 张表.
    pub fn new(path: &Path) -> Result<Self, SessionStoreError> {
        // 确保父目录存在
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(path)?;
        // 启用外键约束 (CASCADE 删除依赖)
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        // WAL 模式支持并发读
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        create_tables(&conn)?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开内存数据库 (用于测试).
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, SessionStoreError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        create_tables(&conn)?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// 创建 session + 写空 snapshot (ordinal=0, 空 JSON).
    pub fn create(
        &self,
        id: &str,
        project_root: Option<&str>,
        config_json: &str,
    ) -> Result<(), SessionStoreError> {
        let conn = self.db.lock()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO daemon_sessions \
             (id, project_root, config_json, status, created_at, last_active_at, message_count) \
             VALUES (?1, ?2, ?3, 'active', ?4, ?4, 0)",
            params![id, project_root, config_json, now],
        )?;
        // 写 ordinal=0 的空 snapshot
        conn.execute(
            "INSERT INTO daemon_conversation_snapshots \
             (session_id, ordinal, data_json, created_at) \
             VALUES (?1, 0, '{\"messages\":[]}', ?2)",
            params![id, now],
        )?;
        Ok(())
    }

    /// 增量 snapshot (Stage 3 简化: 调用方每 5 message 或 60s 调一次).
    pub fn save_snapshot(
        &self,
        session_id: &str,
        ordinal: u32,
        conversation_json: &str,
    ) -> Result<(), SessionStoreError> {
        let conn = self.db.lock()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO daemon_conversation_snapshots \
             (session_id, ordinal, data_json, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, ordinal, conversation_json, now],
        )?;
        // 更新 last_active_at
        conn.execute(
            "UPDATE daemon_sessions SET last_active_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }

    /// 追加一条事件 (SSE 流式增量 log).
    pub fn append_event(
        &self,
        session_id: &str,
        seq: u32,
        event_type: &str,
        event_json: &str,
    ) -> Result<(), SessionStoreError> {
        let conn = self.db.lock()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO daemon_event_log (session_id, seq, event_type, event_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, seq, event_type, event_json, now],
        )?;
        Ok(())
    }

    /// 列出所有 active session (按 last_active_at 倒序).
    pub fn list_active(&self) -> Result<Vec<SessionMeta>, SessionStoreError> {
        let conn = self.db.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, project_root, status, created_at, last_active_at, message_count \
             FROM daemon_sessions WHERE status = 'active' \
             ORDER BY last_active_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionMeta {
                id: row.get(0)?,
                project_root: row.get(1)?,
                status: row.get(2)?,
                created_at: row.get(3)?,
                last_active_at: row.get(4)?,
                message_count: row.get::<_, i64>(5)? as u32,
            })
        })?;
        let out: Vec<SessionMeta> = rows.filter_map(|r| r.ok()).collect();
        Ok(out)
    }

    /// 启动恢复: 加载最新 snapshot (按 ordinal 最大).
    pub fn load_latest_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<(u32, String)>, SessionStoreError> {
        let conn = self.db.lock()?;
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT ordinal, data_json FROM daemon_conversation_snapshots \
                 WHERE session_id = ?1 ORDER BY ordinal DESC LIMIT 1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(o, j)| (o as u32, j)))
    }

    /// Stage 4: 增量 snapshot — 接收真实 `Conversation` 引用, 调
    /// `Conversation::to_jsonl_string()` 拿到 JSONL 串, 写入 snapshots 表.
    ///
    /// 行为跟 `save_snapshot` (字符串版) 一致: `INSERT OR REPLACE` (同
    /// `(session_id, ordinal)` 覆盖) + 更新 `daemon_sessions.last_active_at`.
    pub fn save_conversation_snapshot(
        &self,
        session_id: &str,
        ordinal: u32,
        conversation: &Conversation,
    ) -> Result<(), SessionStoreError> {
        let jsonl = conversation.to_jsonl_string();
        self.save_snapshot(session_id, ordinal, &jsonl)
    }

    /// Stage 4: 启动恢复 — 加载最新 snapshot 并反序列化为 `Conversation`.
    ///
    /// 容错: ordinal=0 的占位 snapshot (`{"messages":[]}`) 不是 JSONL 格式,
    /// `from_jsonl_str` 会自然忽略 (没有 system 行, 也没有 message 行,
    /// 返一个空的 `Conversation`), 等价于"无 conversation 状态".
    ///
    /// 字段对齐: 旧 session 可能用了 `system_prompt = None`, 加载后保持 None.
    pub fn load_latest_conversation(
        &self,
        session_id: &str,
    ) -> Result<Option<(u32, Conversation)>, SessionStoreError> {
        let Some((ordinal, jsonl)) = self.load_latest_snapshot(session_id)? else {
            return Ok(None);
        };
        // ordinal=0 的占位 snapshot 是 `{"messages":[]}` 不是 JSONL,
        // from_jsonl_str 会自然返一个 system_prompt=None, messages=[] 的
        // 空 Conversation. 保持原样返回 (ordinal=0 也能让 caller 区分
        // "刚刚 create" vs "有 turn" — 见 caller 端的处理).
        let conversation = Conversation::from_jsonl_str(&jsonl)?;
        Ok(Some((ordinal, conversation)))
    }

    /// 启动恢复: 加载事件流 (从 seq > from_seq 开始, 按 seq 升序).
    #[allow(dead_code)] // Stage 3 暂未在生产路径调 (后续 Stage 4 完整恢复用)
    pub fn load_events(
        &self,
        session_id: &str,
        from_seq: u32,
    ) -> Result<Vec<EventEntry>, SessionStoreError> {
        let conn = self.db.lock()?;
        let mut stmt = conn.prepare(
            "SELECT seq, event_type, event_json FROM daemon_event_log \
             WHERE session_id = ?1 AND seq > ?2 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session_id, from_seq], |row| {
            Ok(EventEntry {
                seq: row.get::<_, i64>(0)? as u32,
                event_type: row.get(1)?,
                event_json: row.get(2)?,
            })
        })?;
        let out: Vec<EventEntry> = rows.filter_map(|r| r.ok()).collect();
        Ok(out)
    }

    /// 删除 session (FK CASCADE 自动清理 snapshots + events).
    ///
    /// 加在 7 个核心方法之外, 用于 `AgentLoopHost::delete_session` 同步
    /// 持久化层. 任务规范列了 7 个方法, 这第 8 个是自然补充.
    pub fn delete(&self, session_id: &str) -> Result<(), SessionStoreError> {
        let conn = self.db.lock()?;
        conn.execute(
            "DELETE FROM daemon_sessions WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Stage 10b: 优雅关闭时强制 checkpoint WAL, 确保所有挂起写入磁盘.
    ///
    /// 行为:
    /// 1. 拿 db lock (跟其他方法一致)
    /// 2. 执行 `PRAGMA wal_checkpoint(TRUNCATE)` — 把 WAL 文件截断, 等价
    ///    完整 checkpoint. SQLite 内部 atomic, 即使有 in-flight 写也会
    ///    等它们完成后做 checkpoint.
    /// 3. 返回 Ok(()) 表示成功
    ///
    /// 失败模式: lock poison (其他方法一致, panic) 或 SQLite I/O 错
    /// (返 Err). Graceful shutdown 调用方应 `match`, 失败时 warn 但不
    /// 中断退出流程 (强迫进程退出).
    pub fn flush(&self) -> Result<(), SessionStoreError> {
        let conn = self.db.lock()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

// ─── 内部: 3 张表 DDL ────────────────────────────────────────

fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        -- === 1. session 元数据 + 状态机 ===
        CREATE TABLE IF NOT EXISTS daemon_sessions (
            id              TEXT PRIMARY KEY,
            project_root    TEXT,
            config_json     TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'active',
            created_at      TEXT NOT NULL,
            last_active_at  TEXT NOT NULL,
            message_count   INTEGER NOT NULL DEFAULT 0
        );

        -- === 2. conversation 快照 (增量) ===
        CREATE TABLE IF NOT EXISTS daemon_conversation_snapshots (
            session_id      TEXT NOT NULL REFERENCES daemon_sessions(id) ON DELETE CASCADE,
            ordinal         INTEGER NOT NULL,
            data_json       TEXT NOT NULL,
            created_at      TEXT NOT NULL,
            PRIMARY KEY (session_id, ordinal)
        );

        -- === 3. 事件流 (SSE transcript) ===
        CREATE TABLE IF NOT EXISTS daemon_event_log (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id      TEXT NOT NULL REFERENCES daemon_sessions(id) ON DELETE CASCADE,
            seq             INTEGER NOT NULL,
            event_type      TEXT NOT NULL,
            event_json      TEXT NOT NULL,
            created_at      TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_event_log_session_seq
            ON daemon_event_log(session_id, seq);

        -- === 4-12. Kanban 子系统 (MVP-2 plan 1) — 8 张 kanban_* 表 ===
        -- 见 v6 §6.5 + §3.6.2. 沿用 daemon.db, 跟 daemon_sessions 同一文件.
        -- 字段名 t_/r_ 前缀 (v6 §6.2 决策, 避免 SQL JOIN 混淆).

        -- §3.6.2 Project (v5 新增) — 1:N Board
        CREATE TABLE IF NOT EXISTS kanban_projects (
            id            TEXT PRIMARY KEY,         -- 'proj_xxx'
            name          TEXT NOT NULL,
            description   TEXT NOT NULL,
            default_root  TEXT NOT NULL,
            extra_roots   TEXT NOT NULL DEFAULT '[]',  -- JSON array
            status        TEXT NOT NULL DEFAULT 'active',  -- active | archived
            owner         TEXT NOT NULL DEFAULT 'local',
            created_at    TEXT NOT NULL,            -- RFC3339
            updated_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_projects_status ON kanban_projects(status);

        -- §6.5 Board — 1:1 Project (v1 简化, v2 多 board)
        CREATE TABLE IF NOT EXISTS kanban_boards (
            id            TEXT PRIMARY KEY,         -- 'kb_xxx'
            project_id    TEXT REFERENCES kanban_projects(id) ON DELETE CASCADE,
            name          TEXT NOT NULL,
            project_root  TEXT NOT NULL,
            default_role  TEXT NOT NULL DEFAULT 'coordinator',
            status        TEXT NOT NULL DEFAULT 'active',  -- active | archived
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_boards_status ON kanban_boards(status);
        CREATE INDEX IF NOT EXISTS idx_kanban_boards_project ON kanban_boards(project_id);

        -- §6.5 Role — 角色模板 (跟 Profile 1:N)
        CREATE TABLE IF NOT EXISTS kanban_role_defs (
            id            TEXT PRIMARY KEY,         -- 'role_xxx'
            name          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL,
            instructions  TEXT NOT NULL,
            default_profile_id TEXT,                -- FK kanban_profiles.id
            allowed_tool_categories TEXT NOT NULL,  -- JSON: ['Read', 'Search', ...]
            created_at    TEXT NOT NULL
        );

        -- §6.5 Profile — Agent 实例定义 (Hermes 角色内化)
        CREATE TABLE IF NOT EXISTS kanban_profiles (
            id            TEXT PRIMARY KEY,         -- 'prof_xxx'
            name          TEXT NOT NULL UNIQUE,
            kind          TEXT NOT NULL DEFAULT 'local',  -- local | remote
            working_dir   TEXT NOT NULL,
            tool_filter   TEXT NOT NULL,            -- JSON
            max_turns     INTEGER NOT NULL DEFAULT 32,
            model         TEXT,
            system_prompt_template TEXT NOT NULL,
            created_at    TEXT NOT NULL
        );

        -- §6.5 Task — 主表 (字段 t_/r_ 前缀)
        CREATE TABLE IF NOT EXISTS kanban_tasks (
            id            TEXT PRIMARY KEY,         -- 'task_xxx'
            board_id      TEXT NOT NULL REFERENCES kanban_boards(id) ON DELETE CASCADE,
            parent_id     TEXT REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            title         TEXT NOT NULL,
            body          TEXT NOT NULL,
            assignee_role TEXT NOT NULL,            -- FK kanban_role_defs.id
            status        TEXT NOT NULL DEFAULT 'triage',
            priority      INTEGER NOT NULL DEFAULT 128,
            deadline      TEXT,
            metadata      TEXT NOT NULL DEFAULT '{}',  -- JSON
            created_at    TEXT NOT NULL,
            t_started_at  TEXT,
            t_completed_at TEXT,
            last_heartbeat_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_board ON kanban_tasks(board_id);
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_parent ON kanban_tasks(parent_id);
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_status ON kanban_tasks(status);
        CREATE INDEX IF NOT EXISTS idx_kanban_tasks_assignee ON kanban_tasks(assignee_role);

        -- §6.5 TaskLink — DAG 边
        CREATE TABLE IF NOT EXISTS kanban_task_links (
            parent_id     TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            child_id      TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            dep_type      TEXT NOT NULL DEFAULT 'sequential',
            created_at    TEXT NOT NULL,
            PRIMARY KEY (parent_id, child_id)
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_task_links_child ON kanban_task_links(child_id);

        -- §6.5 AgentRun — 执行历史 (重试时新建 row)
        CREATE TABLE IF NOT EXISTS kanban_runs (
            id            TEXT PRIMARY KEY,         -- 'run_xxx'
            task_id       TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            profile_id    TEXT NOT NULL REFERENCES kanban_profiles(id),
            status        TEXT NOT NULL DEFAULT 'pending',
            claim_id      TEXT NOT NULL,            -- UUID v4, 重新认领时换
            r_heartbeat_at TEXT,
            started_at    TEXT NOT NULL,
            ended_at      TEXT,
            outcome       TEXT NOT NULL DEFAULT 'success',
            summary       TEXT NOT NULL DEFAULT '',
            error         TEXT,
            token_input   INTEGER NOT NULL DEFAULT 0,
            token_output  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_runs_task ON kanban_runs(task_id);
        CREATE INDEX IF NOT EXISTS idx_kanban_runs_status ON kanban_runs(status);

        -- §4 模式 2 Blackboard — 独立表化 (v6 决策, 不塞 task_comments)
        CREATE TABLE IF NOT EXISTS kanban_blackboard (
            task_id       TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            key           TEXT NOT NULL,
            value         TEXT NOT NULL,            -- JSON
            author        TEXT NOT NULL,
            updated_at    TEXT NOT NULL,
            PRIMARY KEY (task_id, key)
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_blackboard_task ON kanban_blackboard(task_id);

        -- §6.3 23 种事件 (audit + 实时推送)
        CREATE TABLE IF NOT EXISTS kanban_events (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id       TEXT,                     -- NULL 表示 board 级事件
            run_id        TEXT,
            kind          TEXT NOT NULL,
            payload       TEXT NOT NULL,            -- JSON
            created_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_kanban_events_task ON kanban_events(task_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_kanban_events_kind ON kanban_events(kind);
        ",
    )?;
    // MVP-2 plan 1: ALTER TABLE 迁移 (2 个) + default project 注入 + 老数据归位
    init_kanban_schema(conn)?;
    Ok(())
}

// ─── 内部: Kanban 迁移 (MVP-2 plan 1) ─────────────────────

/// 2 个 ALTER TABLE 迁移 (v6 §3.6.2 + §6.5).
///
/// SQLite 不支持 `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, 用 try_each 模式:
/// 每次尝试, 失败 (列已存在) 时 skip.
///
/// 老 board/session 默认归到 "default" project (id = 'proj_default'),
/// 由 `ensure_default_project()` 创建.
pub fn init_kanban_schema(conn: &Connection) -> rusqlite::Result<()> {
    // 1. ALTER: kanban_boards 加 project_id
    let _ = conn.execute(
        "ALTER TABLE kanban_boards ADD COLUMN project_id TEXT REFERENCES kanban_projects(id)",
        [],
    );
    // 2. ALTER: daemon_sessions 加 project_id
    let _ = conn.execute(
        "ALTER TABLE daemon_sessions ADD COLUMN project_id TEXT REFERENCES kanban_projects(id)",
        [],
    );
    // 3. 索引 (ALTER 之后才能创建基于列的索引)
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_kanban_boards_project_alter ON kanban_boards(project_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_daemon_sessions_project ON daemon_sessions(project_id)",
        [],
    );
    // 4. 注入 default project
    ensure_default_project(conn)?;
    // 5. 老 board / session 归到 default (只跑一次, 已归的 WHERE 命中 0 行)
    let _ = conn.execute(
        "UPDATE kanban_boards SET project_id = 'proj_default' WHERE project_id IS NULL",
        [],
    );
    let _ = conn.execute(
        "UPDATE daemon_sessions SET project_id = 'proj_default' WHERE project_id IS NULL",
        [],
    );
    Ok(())
}

/// 确保 default project 存在 (id = 'proj_default').
///
/// 用 try_each 模式: 如果 `kanban_projects` 表不存在 (上游 DDL 没建),
/// 优雅返回 Ok 而不是 panic, 让调用方决定下一步.
pub fn ensure_default_project(conn: &Connection) -> rusqlite::Result<()> {
    // 1. 检查 kanban_projects 表是否存在
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='kanban_projects'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !table_exists {
        // 表不存在, 跳过 (上游 DDL 还没跑)
        return Ok(());
    }
    // 2. 检查 proj_default 行是否存在
    let row_exists: bool = conn
        .query_row(
            "SELECT 1 FROM kanban_projects WHERE id = ?1",
            rusqlite::params!["proj_default"],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !row_exists {
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO kanban_projects \
             (id, name, description, default_root, extra_roots, status, owner, created_at, updated_at) \
             VALUES ('proj_default', 'default', 'Auto-created default project', '', '[]', 'active', 'local', ?1, ?1)",
            rusqlite::params![now],
        )?;
    }
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_list_session() {
        let store = SessionStore::in_memory().expect("in_memory");

        // 空表 → 0 个 active session
        let initial = store.list_active().expect("list");
        assert_eq!(initial.len(), 0, "in-memory store should start empty");

        // 创建 3 个
        let cfg = r#"{"model":"deepseek-v4-flash"}"#;
        store.create("sess_a", Some("/work/a"), cfg).expect("create a");
        store.create("sess_b", None, cfg).expect("create b");
        store.create("sess_c", Some("/work/c"), cfg).expect("create c");

        // list_active 拿到 3 个
        let active = store.list_active().expect("list");
        assert_eq!(active.len(), 3, "expected 3 active sessions");

        // 验证字段
        let by_id: std::collections::HashMap<&str, &SessionMeta> = active
            .iter()
            .map(|m| (m.id.as_str(), m))
            .collect();
        assert!(by_id.contains_key("sess_a"));
        assert!(by_id.contains_key("sess_b"));
        assert!(by_id.contains_key("sess_c"));
        assert_eq!(by_id["sess_a"].project_root.as_deref(), Some("/work/a"));
        assert_eq!(by_id["sess_b"].project_root, None);
        assert_eq!(by_id["sess_c"].status, "active");
        assert_eq!(by_id["sess_a"].message_count, 0);
    }

    #[test]
    fn test_save_and_load_snapshot() {
        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_snap", Some("/work"), cfg).expect("create");

        // 初始 snapshot ordinal=0
        let initial = store.load_latest_snapshot("sess_snap").expect("load");
        assert!(initial.is_some());
        let (ord, json) = initial.unwrap();
        assert_eq!(ord, 0, "initial snapshot should be ordinal 0");
        assert!(json.contains("messages"), "initial snapshot should have empty messages array");

        // 写第 1 个 snapshot
        store
            .save_snapshot("sess_snap", 1, r#"{"messages":[{"role":"user","content":"hi"}]}"#)
            .expect("save 1");
        let loaded1 = store.load_latest_snapshot("sess_snap").expect("load 1");
        let (ord1, json1) = loaded1.expect("should have snapshot");
        assert_eq!(ord1, 1);
        assert!(json1.contains("user"));

        // 写第 2 个 snapshot
        store
            .save_snapshot(
                "sess_snap",
                2,
                r#"{"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]}"#,
            )
            .expect("save 2");
        let loaded2 = store.load_latest_snapshot("sess_snap").expect("load 2");
        let (ord2, json2) = loaded2.expect("should have snapshot");
        // load_latest_snapshot 返回 ordinal 最大的
        assert_eq!(ord2, 2, "expected latest ordinal = 2");
        assert!(json2.contains("assistant"));
        assert!(json2.contains("hello"));

        // 不存在的 session 返回 None
        let missing = store.load_latest_snapshot("sess_does_not_exist").expect("load missing");
        assert!(missing.is_none());
    }

    #[test]
    fn test_append_and_load_events() {
        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_evt", Some("/work"), cfg).expect("create");

        // 空 → load_events 返回 []
        let initial = store.load_events("sess_evt", 0).expect("load empty");
        assert_eq!(initial.len(), 0);

        // 追加 5 个事件 (注意: 乱序追加, 验证 load 时按 seq 排序)
        store
            .append_event("sess_evt", 1, "message_start", r#"{"type":"message_start"}"#)
            .expect("evt 1");
        store
            .append_event("sess_evt", 3, "text_delta", r#"{"type":"text_delta","i":0,"text":"hi"}"#)
            .expect("evt 3");
        store
            .append_event("sess_evt", 2, "content_block_start", r#"{"type":"content_block_start"}"#)
            .expect("evt 2");
        store
            .append_event("sess_evt", 4, "usage", r#"{"type":"usage","input":10,"output":5}"#)
            .expect("evt 4");
        store
            .append_event("sess_evt", 5, "message_stop", r#"{"type":"message_stop"}"#)
            .expect("evt 5");

        // load_events 拿全部 (from_seq=0)
        let all = store.load_events("sess_evt", 0).expect("load all");
        assert_eq!(all.len(), 5);
        // 验证 seq 升序
        let seqs: Vec<u32> = all.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5], "events should be sorted by seq ASC");

        // 验证 event_type 顺序
        let types: Vec<&str> = all.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            types,
            vec![
                "message_start",
                "content_block_start",
                "text_delta",
                "usage",
                "message_stop",
            ]
        );

        // from_seq=3 → 只拿 seq > 3 的 (即 4, 5)
        let tail = store.load_events("sess_evt", 3).expect("load tail");
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].seq, 4);
        assert_eq!(tail[1].seq, 5);
        assert_eq!(tail[0].event_type, "usage");
        assert_eq!(tail[1].event_type, "message_stop");

        // 验证 event_json 完整保留
        assert!(tail[0].event_json.contains("\"input\":10"));
        assert!(tail[0].event_json.contains("\"output\":5"));
    }

    #[test]
    fn test_session_delete_cascades() {
        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_cascade", Some("/work"), cfg).expect("create");

        // 写 2 个 snapshot + 3 个 event
        store
            .save_snapshot("sess_cascade", 0, r#"{"messages":[]}"#)
            .expect("snap 0");
        store
            .save_snapshot("sess_cascade", 1, r#"{"messages":[{"role":"user","content":"x"}]}"#)
            .expect("snap 1");
        store
            .append_event("sess_cascade", 1, "message_start", "{}")
            .expect("evt 1");
        store
            .append_event("sess_cascade", 2, "text_delta", r#"{"text":"a"}"#)
            .expect("evt 2");
        store
            .append_event("sess_cascade", 3, "message_stop", "{}")
            .expect("evt 3");

        // 验证数据存在
        let conn = store.db.lock().expect("lock");
        let snap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM daemon_conversation_snapshots WHERE session_id = ?1",
                params!["sess_cascade"],
                |row| row.get(0),
            )
            .expect("count snap");
        let evt_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM daemon_event_log WHERE session_id = ?1",
                params!["sess_cascade"],
                |row| row.get(0),
            )
            .expect("count evt");
        assert_eq!(snap_count, 2);
        assert_eq!(evt_count, 3);
        drop(conn);

        // 删 session → 级联删除 snapshots + events
        {
            let conn = store.db.lock().expect("lock");
            conn.execute("DELETE FROM daemon_sessions WHERE id = ?1", params!["sess_cascade"])
                .expect("delete");
        }

        // 验证 cascade
        let conn = store.db.lock().expect("lock");
        let snap_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM daemon_conversation_snapshots WHERE session_id = ?1",
                params!["sess_cascade"],
                |row| row.get(0),
            )
            .expect("count snap after");
        let evt_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM daemon_event_log WHERE session_id = ?1",
                params!["sess_cascade"],
                |row| row.get(0),
            )
            .expect("count evt after");
        let sess_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM daemon_sessions WHERE id = ?1",
                params!["sess_cascade"],
                |row| row.get(0),
            )
            .expect("count sess after");
        assert_eq!(sess_count, 0, "session should be deleted");
        assert_eq!(snap_count_after, 0, "snapshots should cascade delete");
        assert_eq!(evt_count_after, 0, "events should cascade delete");
    }

    // ─── Stage 4: 真实 Conversation 持久化 ─────────────────────

    /// 验证 save_conversation_snapshot 把 Conversation 序列化成 JSONL
    /// (含 system 行 + message 行) 写进 SQLite, load_latest_conversation
    /// 反序列化后字段完全一致.
    #[test]
    fn test_save_and_load_conversation_roundtrip() {
        use qianxun_core::agent::message::{ContentBlock, Message};
        use qianxun_core::agent::conversation::Conversation;

        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_conv", Some("/work"), cfg).expect("create");

        // 构造一个真实 Conversation: system_prompt + 1 user msg + 1 assistant msg
        let mut conv = Conversation::new(Some("You are a helper.".to_string()));
        conv.push_user_message(vec![ContentBlock::text("hello")]);
        conv.push_message(Message::assistant(vec![ContentBlock::text("hi there!")]));

        // save_conversation_snapshot 写 ordinal=1
        store
            .save_conversation_snapshot("sess_conv", 1, &conv)
            .expect("save conv");

        // load_latest_conversation 反序列化
        let (ord, loaded) = store
            .load_latest_conversation("sess_conv")
            .expect("load conv")
            .expect("should have snapshot");

        assert_eq!(ord, 1);
        assert_eq!(loaded.messages().len(), 2, "user + assistant");

        // 验证 user message
        match &loaded.messages()[0] {
            Message::User { content, .. } => {
                assert_eq!(content.len(), 1);
                assert_eq!(content[0].text.as_deref(), Some("hello"));
            }
            other => panic!("expected User message, got {other:?}"),
        }

        // 验证 assistant message
        match &loaded.messages()[1] {
            Message::Assistant { content, .. } => {
                assert_eq!(content.len(), 1);
                assert_eq!(content[0].text.as_deref(), Some("hi there!"));
            }
            other => panic!("expected Assistant message, got {other:?}"),
        }

        // 验证 ordinal=1 是 max (load 返最大)
        let (ord_max, _) = store
            .load_latest_conversation("sess_conv")
            .expect("load max")
            .expect("present");
        assert_eq!(ord_max, 1, "load_latest should return max ordinal");
    }

    /// 验证空 Conversation (无 messages) 能正确 save/load.
    #[test]
    fn test_save_load_empty_conversation() {
        use qianxun_core::agent::conversation::Conversation;

        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_empty", None, cfg).expect("create");

        // 空 conv (无 system_prompt, 无 messages)
        let conv = Conversation::new(None);
        assert_eq!(conv.messages().len(), 0);

        store
            .save_conversation_snapshot("sess_empty", 1, &conv)
            .expect("save empty");

        let (ord, loaded) = store
            .load_latest_conversation("sess_empty")
            .expect("load empty")
            .expect("present");
        assert_eq!(ord, 1);
        assert_eq!(loaded.messages().len(), 0, "empty conv should load as empty");
    }

    /// 验证 ordinal=0 的占位 snapshot (create 时写入的 `{"messages":[]}`)
    /// 通过 load_latest_conversation 加载时不会 panic, 自然返一个空 Conversation.
    #[test]
    fn test_load_placeholder_snapshot_returns_empty_conv() {
        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_placeholder", None, cfg).expect("create");

        // 不写任何 save_conversation_snapshot, 只用 create 留下的 ordinal=0 占位
        let (ord, loaded) = store
            .load_latest_conversation("sess_placeholder")
            .expect("load placeholder")
            .expect("present");
        assert_eq!(ord, 0, "placeholder should be ordinal 0");
        assert_eq!(loaded.messages().len(), 0);
    }

    /// 验证 save_conversation_snapshot 走 INSERT OR REPLACE: 同 ordinal 二次写
    /// 会覆盖, 后续 load 拿到最新版.
    #[test]
    fn test_save_conversation_overwrites_same_ordinal() {
        use qianxun_core::agent::message::ContentBlock;
        use qianxun_core::agent::conversation::Conversation;

        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_overwrite", None, cfg).expect("create");

        // 第 1 次写 ordinal=1: 1 user msg
        let mut conv1 = Conversation::new(None);
        conv1.push_user_message(vec![ContentBlock::text("first")]);
        store
            .save_conversation_snapshot("sess_overwrite", 1, &conv1)
            .expect("save 1");

        // 第 2 次写 ordinal=1 (覆盖): 2 user msg
        let mut conv2 = Conversation::new(None);
        conv2.push_user_message(vec![ContentBlock::text("first")]);
        conv2.push_user_message(vec![ContentBlock::text("second")]);
        store
            .save_conversation_snapshot("sess_overwrite", 1, &conv2)
            .expect("save 2 overwrite");

        // load 拿到的是覆盖后的版本
        let (_, loaded) = store
            .load_latest_conversation("sess_overwrite")
            .expect("load")
            .expect("present");
        assert_eq!(loaded.messages().len(), 2, "ordinal 1 was overwritten");
    }

    /// 验证 SessionStore 的字符串版 save_snapshot 和新加的
    /// save_conversation_snapshot 互不干扰 (可以混用, 各自走自己的路径).
    #[test]
    fn test_string_and_conversation_snapshot_interop() {
        use qianxun_core::agent::conversation::Conversation;
        use qianxun_core::agent::message::ContentBlock;

        let store = SessionStore::in_memory().expect("in_memory");
        let cfg = r#"{"model":"x"}"#;
        store.create("sess_mix", None, cfg).expect("create");

        // ordinal=1 用字符串版 (任意 JSON 内容)
        store
            .save_snapshot("sess_mix", 1, r#"{"messages":[],"legacy":"true"}"#)
            .expect("save string");

        // ordinal=2 用 conversation 版
        let mut conv = Conversation::new(Some("sys".into()));
        conv.push_user_message(vec![ContentBlock::text("hi")]);
        store
            .save_conversation_snapshot("sess_mix", 2, &conv)
            .expect("save conv");

        // load_latest_conversation 应该拿到 ordinal=2 (max)
        let (ord, loaded) = store
            .load_latest_conversation("sess_mix")
            .expect("load")
            .expect("present");
        assert_eq!(ord, 2, "max ordinal is 2");
        assert_eq!(loaded.messages().len(), 1);

        // load_latest_snapshot (字符串版) 也应该拿到 ordinal=2 的内容
        let (ord_str, json_str) = store
            .load_latest_snapshot("sess_mix")
            .expect("load str")
            .expect("present");
        assert_eq!(ord_str, 2);
        // JSONL 格式: system 行含 prompt, 后续含消息
        assert!(json_str.contains("\"type\":\"system\""), "should have system line");
        assert!(json_str.contains("User"), "should have user message line");
    }

    // ========== Kanban Schema (MVP-2 plan 1) ==========

    /// 验证 init_kanban_schema 8 张表 DDL 全部落地.
    /// CREATE TABLE IF NOT EXISTS, 老表已存在不会破坏.
    #[test]
    fn test_init_kanban_schema_creates_all_tables() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").expect("pragma");
        // 先建 kanban_projects (FK 目标), 才能 ALTER 引用
        conn.execute_batch(KANBAN_BASE_DDL).expect("base ddl");
        init_kanban_schema(&conn).expect("init_kanban");

        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'kanban_%' ORDER BY name")
                .expect("prepare");
            stmt.query_map([], |row| row.get(0))
                .expect("query_map")
                .filter_map(|r| r.ok())
                .collect()
        };
        let expected = [
            "kanban_blackboard",
            "kanban_boards",
            "kanban_events",
            "kanban_profiles",
            "kanban_projects",
            "kanban_role_defs",
            "kanban_runs",
            "kanban_task_links",
            "kanban_tasks",
        ];
        assert_eq!(
            tables.len(),
            expected.len(),
            "kanban_* tables count mismatch: got {tables:?}"
        );
        for name in expected {
            assert!(tables.contains(&name.to_string()), "missing table {name}");
        }
    }

    /// 验证 init_kanban_schema 幂等: 跑 2 次不报错 (ALTER 失败 skip, IF NOT EXISTS
    /// 跳过).
    #[test]
    fn test_init_kanban_schema_idempotent() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").expect("pragma");
        conn.execute_batch(KANBAN_BASE_DDL).expect("base ddl");
        init_kanban_schema(&conn).expect("init 1st");
        init_kanban_schema(&conn).expect("init 2nd (idempotent)");
        init_kanban_schema(&conn).expect("init 3rd");
    }

    /// 验证 kanban_boards.project_id 列存在.
    #[test]
    fn test_kanban_boards_project_id_column_exists() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch(KANBAN_BASE_DDL).expect("base ddl");
        init_kanban_schema(&conn).expect("init_kanban");

        let cols: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(kanban_boards)")
                .expect("pragma");
            stmt.query_map([], |row| row.get::<_, String>(1))
                .expect("query_map")
                .filter_map(|r| r.ok())
                .collect()
        };
        assert!(
            cols.contains(&"project_id".to_string()),
            "kanban_boards should have project_id column, got: {cols:?}"
        );
    }

    /// 验证 daemon_sessions.project_id 列存在.
    #[test]
    fn test_daemon_sessions_project_id_column_exists() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").expect("pragma");
        conn.execute_batch(SESSION_DDL).expect("session ddl");
        // 单独建 kanban_projects (init_kanban_schema 依赖它)
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kanban_projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL, default_root TEXT NOT NULL, extra_roots TEXT NOT NULL DEFAULT '[]', status TEXT NOT NULL DEFAULT 'active', owner TEXT NOT NULL DEFAULT 'local', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);"
        ).expect("projects ddl");
        init_kanban_schema(&conn).expect("init_kanban");

        let cols: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(daemon_sessions)")
                .expect("pragma");
            stmt.query_map([], |row| row.get::<_, String>(1))
                .expect("query_map")
                .filter_map(|r| r.ok())
                .collect()
        };
        assert!(
            cols.contains(&"project_id".to_string()),
            "daemon_sessions should have project_id column, got: {cols:?}"
        );
    }

    /// 验证 default project 自动注入 (id = 'proj_default').
    #[test]
    fn test_default_project_inserted() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").expect("pragma");
        conn.execute_batch(KANBAN_BASE_DDL).expect("base ddl");
        init_kanban_schema(&conn).expect("init_kanban");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kanban_projects WHERE id = 'proj_default'",
                [],
                |row| row.get(0),
            )
            .expect("query default project");
        assert_eq!(count, 1, "default project should be inserted exactly once");
    }

    /// 验证老 kanban_boards 归到 default project (WHERE project_id IS NULL UPDATE).
    ///
    /// 模拟迁移场景: 先跑 init_kanban_schema (建表 + 注入 default project),
    /// 然后插入老 board (没 project_id), 再跑一次 init_kanban_schema 触发
    /// UPDATE 迁移.
    #[test]
    fn test_old_kanban_boards_assigned_to_default_project() {
        let conn = Connection::open_in_memory().expect("in_memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").expect("pragma");
        conn.execute_batch(KANBAN_BASE_DDL).expect("base ddl");
        init_kanban_schema(&conn).expect("init 1st (setup)");

        // 插入一个老 board (无 project_id) — 模拟迁移前的老数据
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO kanban_boards (id, name, project_root, status, created_at, updated_at) \
             VALUES ('kb_old', 'old board', '/tmp', 'active', ?1, ?1)",
            rusqlite::params![now],
        )
        .expect("insert old board");

        // 跑第二次 init_kanban_schema, 触发 UPDATE 迁移
        init_kanban_schema(&conn).expect("init 2nd (trigger UPDATE migration)");

        let pid: Option<String> = conn
            .query_row(
                "SELECT project_id FROM kanban_boards WHERE id = 'kb_old'",
                [],
                |row| row.get(0),
            )
            .expect("query board");
        assert_eq!(
            pid.as_deref(),
            Some("proj_default"),
            "old board should be assigned to default project after migration"
        );
    }

    // ---- 辅助 DDL (测试用) ----
    // 跟 create_tables 里 DDL 一样, 这里复制出来让测试独立.
    const SESSION_DDL: &str = "
        CREATE TABLE IF NOT EXISTS daemon_sessions (
            id              TEXT PRIMARY KEY,
            project_root    TEXT,
            config_json     TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'active',
            created_at      TEXT NOT NULL,
            last_active_at  TEXT NOT NULL,
            message_count   INTEGER NOT NULL DEFAULT 0
        );
    ";

    // 基础 9 张 kanban_* 表 DDL (跟 create_tables 里一致)
    const KANBAN_BASE_DDL: &str = "
        CREATE TABLE IF NOT EXISTS kanban_projects (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            description   TEXT NOT NULL,
            default_root  TEXT NOT NULL,
            extra_roots   TEXT NOT NULL DEFAULT '[]',
            status        TEXT NOT NULL DEFAULT 'active',
            owner         TEXT NOT NULL DEFAULT 'local',
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kanban_boards (
            id            TEXT PRIMARY KEY,
            project_id    TEXT REFERENCES kanban_projects(id) ON DELETE CASCADE,
            name          TEXT NOT NULL,
            project_root  TEXT NOT NULL,
            default_role  TEXT NOT NULL DEFAULT 'coordinator',
            status        TEXT NOT NULL DEFAULT 'active',
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kanban_role_defs (
            id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT NOT NULL,
            instructions TEXT NOT NULL, default_profile_id TEXT,
            allowed_tool_categories TEXT NOT NULL, created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kanban_profiles (
            id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT NOT NULL DEFAULT 'local',
            working_dir TEXT NOT NULL, tool_filter TEXT NOT NULL,
            max_turns INTEGER NOT NULL DEFAULT 32, model TEXT,
            system_prompt_template TEXT NOT NULL, created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kanban_tasks (
            id TEXT PRIMARY KEY,
            board_id TEXT NOT NULL REFERENCES kanban_boards(id) ON DELETE CASCADE,
            parent_id TEXT REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            title TEXT NOT NULL, body TEXT NOT NULL, assignee_role TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'triage',
            priority INTEGER NOT NULL DEFAULT 128, deadline TEXT,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL, t_started_at TEXT, t_completed_at TEXT,
            last_heartbeat_at TEXT
        );
        CREATE TABLE IF NOT EXISTS kanban_task_links (
            parent_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            child_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            dep_type TEXT NOT NULL DEFAULT 'sequential',
            created_at TEXT NOT NULL, PRIMARY KEY (parent_id, child_id)
        );
        CREATE TABLE IF NOT EXISTS kanban_runs (
            id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES kanban_profiles(id),
            status TEXT NOT NULL DEFAULT 'pending', claim_id TEXT NOT NULL,
            r_heartbeat_at TEXT, started_at TEXT NOT NULL, ended_at TEXT,
            outcome TEXT NOT NULL DEFAULT 'success', summary TEXT NOT NULL DEFAULT '',
            error TEXT, token_input INTEGER NOT NULL DEFAULT 0, token_output INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS kanban_blackboard (
            task_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
            key TEXT NOT NULL, value TEXT NOT NULL, author TEXT NOT NULL,
            updated_at TEXT NOT NULL, PRIMARY KEY (task_id, key)
        );
        CREATE TABLE IF NOT EXISTS kanban_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT, run_id TEXT,
            kind TEXT NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL
        );
    ";
}

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
use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

/// Session 持久化错误.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

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
        ",
    )
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
}

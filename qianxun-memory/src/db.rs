use rusqlite::{Connection, Result as SqlResult};
use std::path::Path;

/// 当前数据库 schema 版本。
const SCHEMA_VERSION: u32 = 1;

/// 打开或创建 SQLite 数据库，执行迁移。
pub fn open(path: impl AsRef<Path>) -> SqlResult<Connection> {
    let conn = Connection::open(path)?;

    // 启用 WAL 模式（支持多读一写）
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    // 启用外键约束
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    // 启用 64 位行 ID（FTS5 需要）
    conn.execute_batch("PRAGMA mmap_size=268435456;")?; // 256MB

    migrate(&conn)?;
    Ok(conn)
}

/// 执行数据库迁移。
fn migrate(conn: &Connection) -> SqlResult<()> {
    let version: u32 = conn
        .prepare("PRAGMA user_version")?
        .query_row([], |row| row.get(0))?;

    if version >= SCHEMA_VERSION {
        return Ok(());
    }

    // 从版本 0 → 1 的迁移
    if version < 1 {
        create_tables(conn)?;
        conn.execute_batch("PRAGMA user_version=1;")?;
    }

    Ok(())
}

/// 创建所有表（版本 1）。
fn create_tables(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "
        -- === 会话
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            project TEXT NOT NULL,
            cwd TEXT NOT NULL,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            status TEXT NOT NULL DEFAULT 'active',
            observation_count INTEGER NOT NULL DEFAULT 0,
            model TEXT,
            summary TEXT
        );

        -- === 压缩后的观测
        CREATE TABLE IF NOT EXISTS observations (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            timestamp TEXT NOT NULL,
            data TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_obs_session ON observations(session_id);
        CREATE INDEX IF NOT EXISTS idx_obs_timestamp ON observations(timestamp);
        CREATE INDEX IF NOT EXISTS idx_obs_type
            ON observations(json_extract(data, '$.obs_type'));

        -- === FTS5 全文索引
        CREATE VIRTUAL TABLE IF NOT EXISTS obs_fts USING fts5(
            title, narrative, facts, concepts, files,
            content='observations',
            content_rowid='rowid',
            tokenize='unicode61'
        );

        -- === 向量索引
        CREATE TABLE IF NOT EXISTS observation_vectors (
            obs_id TEXT PRIMARY KEY REFERENCES observations(id),
            embedding BLOB NOT NULL,
            dimensions INTEGER NOT NULL,
            model TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- === 跨会话持久记忆
        CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            mem_type TEXT NOT NULL,
            data TEXT NOT NULL,
            created_at_trig TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(mem_type);
        CREATE INDEX IF NOT EXISTS idx_memories_project
            ON memories(json_extract(data, '$.project'));

        -- === 会话摘要
        CREATE TABLE IF NOT EXISTS session_summaries (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            summary TEXT NOT NULL,
            model TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_summary_session ON session_summaries(session_id);

        -- === 工作记忆插槽
        CREATE TABLE IF NOT EXISTS slots (
            label TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            size_limit INTEGER NOT NULL DEFAULT 2000,
            description TEXT NOT NULL DEFAULT '',
            pinned INTEGER NOT NULL DEFAULT 0,
            scope TEXT NOT NULL DEFAULT 'project',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- === 原始观测（可选，默认不启用）
        CREATE TABLE IF NOT EXISTS raw_observations (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            timestamp TEXT NOT NULL,
            hook_type TEXT NOT NULL,
            tool_name TEXT,
            tool_input TEXT,
            tool_output TEXT,
            user_prompt TEXT,
            assistant_response TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_raw_session ON raw_observations(session_id);

        -- === Consolidation 日志（用于回滚保护）
        CREATE TABLE IF NOT EXISTS consolidation_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            action TEXT NOT NULL,
            details TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_consolidation_session
            ON consolidation_log(session_id);
    ",
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        // 验证表存在
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 9, "expected at least 9 tables, got {count}");
    }

    #[test]
    fn test_insert_and_query_observation() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();

        // 插入 session
        conn.execute(
            "INSERT INTO sessions (id, project, cwd, started_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["sess_01", "test", "/tmp", "2026-06-01T00:00:00Z", "active"],
        )
        .unwrap();

        // 插入 observation
        let data = serde_json::json!({
            "obs_type": "FileRead",
            "title": "读取文件: test.rs",
            "facts": [],
            "narrative": "读取了 test.rs",
            "concepts": ["test"],
            "files": ["test.rs"],
            "importance": 3
        });

        conn.execute(
            "INSERT INTO observations (id, session_id, timestamp, data)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                "obs_01",
                "sess_01",
                "2026-06-01T00:00:01Z",
                data.to_string()
            ],
        )
        .unwrap();

        // 查询验证
        let (obs_id, title): (String, String) = conn
            .query_row(
                "SELECT id, json_extract(data, '$.title') FROM observations WHERE id=?1",
                params!["obs_01"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(obs_id, "obs_01");
        assert!(title.contains("读取文件"));
    }
}

//! Kanban schema 迁移 (从 persistence.rs 抽, 2026-06-04 Commit 11)
//!
//! 2 个 ALTER TABLE + default project 注入 + 老数据归位.

use rusqlite::{Connection, OptionalExtension};

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
    // 3. 索引
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
    // 5. 老 board / session 归到 default
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
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='kanban_projects'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !table_exists {
        return Ok(());
    }
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

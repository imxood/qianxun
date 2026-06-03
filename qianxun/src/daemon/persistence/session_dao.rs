//! Session DAO (从 persistence.rs 抽, 2026-06-04 Commit 11)
//!
//! 3 张 daemon_sessions 表 CRUD: create / list_active / delete.

use chrono::Utc;
use rusqlite::params;

use super::error::SessionStoreError;
use super::types::SessionMeta;
use super::SessionStore;

impl SessionStore {
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

    /// 删除 session (FK CASCADE 自动清理 snapshots + events).
    pub fn delete(&self, session_id: &str) -> Result<(), SessionStoreError> {
        let conn = self.db.lock()?;
        conn.execute(
            "DELETE FROM daemon_sessions WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }
}

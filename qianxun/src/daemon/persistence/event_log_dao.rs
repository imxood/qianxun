//! Event Log DAO (从 persistence.rs 抽, 2026-06-04 Commit 11)
//!
//! 2 个方法: append_event / load_events.

use chrono::Utc;
use rusqlite::params;

use super::error::SessionStoreError;
use super::types::EventEntry;
use super::SessionStore;

impl SessionStore {
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

    /// 启动恢复: 加载事件流 (从 seq > from_seq 开始, 按 seq 升序).
    #[allow(dead_code)] // Stage 3 暂未在生产路径调
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
}

//! Conversation Snapshot DAO (从 persistence.rs 抽, 2026-06-04 Commit 11)
//!
//! 4 个方法: save_snapshot / load_latest_snapshot /
//! save_conversation_snapshot / load_latest_conversation.

use chrono::Utc;
use qianxun_core::agent::conversation::Conversation;
use rusqlite::{params, OptionalExtension};

use super::error::SessionStoreError;
use super::SessionStore;

impl SessionStore {
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

    /// Stage 4: 增量 snapshot — 接收真实 `Conversation` 引用.
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
    pub fn load_latest_conversation(
        &self,
        session_id: &str,
    ) -> Result<Option<(u32, Conversation)>, SessionStoreError> {
        let Some((ordinal, jsonl)) = self.load_latest_snapshot(session_id)? else {
            return Ok(None);
        };
        let conversation = Conversation::from_jsonl_str(&jsonl)?;
        Ok(Some((ordinal, conversation)))
    }
}

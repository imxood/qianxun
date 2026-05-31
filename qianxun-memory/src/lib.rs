pub mod types;
pub mod db;
pub mod compressor;
pub mod privacy;
pub mod vector;
pub mod slot;
pub mod search;
pub mod consolidation;

use async_trait::async_trait;
use qianxun_core::context::{MemoryObserver, SearchResult};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

/// MemoryCore — 记忆引擎入口。
///
/// 持有 SQLite 连接，实现 `MemoryObserver` trait。
pub struct MemoryCore {
    pub db: Arc<std::sync::Mutex<rusqlite::Connection>>,
}

impl MemoryCore {
    /// 打开或创建记忆数据库。
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = db::open(path)?;
        Ok(Self {
            db: Arc::new(std::sync::Mutex::new(conn)),
        })
    }

    /// 创建内存数据库（用于测试或 fallback）。
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::db::create_tables(&conn)?;
        Ok(Self {
            db: Arc::new(std::sync::Mutex::new(conn)),
        })
    }
}

#[async_trait]
impl MemoryObserver for MemoryCore {
    async fn observe(
        &self,
        hook_type: &str,
        tool_name: &str,
        tool_input: Option<Value>,
        tool_output: Option<&str>,
    ) {
        let obs_id = format!("obs_{}", uuid::Uuid::new_v4());
        let session_id = "global";

        let clean_output = tool_output.map(privacy::strip_private_data);
        let observation = compressor::build_synthetic(
            obs_id.clone(),
            session_id.to_string(),
            hook_type,
            tool_name,
            tool_input.as_ref(),
            clean_output.as_deref(),
        );

        let data = serde_json::to_string(&observation).unwrap_or_default();
        let timestamp = observation.timestamp.to_rfc3339();
        if let Ok(conn) = self.db.lock() {
            let _ = conn.execute(
                "INSERT INTO observations (id, session_id, timestamp, data) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![obs_id, session_id, timestamp, data],
            );
        }
    }

    async fn build_context(&self, query: &str, token_budget: u32) -> String {
        let mut parts = Vec::new();
        if let Ok(conn) = self.db.lock() {
            // 1. FTS5 搜索 Observations
            if !query.is_empty() {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT json_extract(o.data, '$.title') FROM obs_fts f JOIN observations o ON o.rowid = f.rowid WHERE obs_fts MATCH ?1 ORDER BY rank LIMIT 5"
                ) {
                    if let Ok(rows) = stmt.query_map(rusqlite::params![query], |row| {
                        let title: String = row.get(0).unwrap_or_default();
                        Ok(title.trim_matches('"').to_string())
                    }) {
                        for row in rows.flatten() {
                            if !row.is_empty() {
                                parts.push(format!("- {row}"));
                            }
                        }
                    }
                }
            }

            // 2. 最近的持久记忆（按创建时间倒序，显示内容摘要）
            if let Ok(mut stmt) = conn.prepare(
                "SELECT json_extract(data, '$.title'), json_extract(data, '$.mem_type'), json_extract(data, '$.content') FROM memories ORDER BY created_at DESC LIMIT 10"
            ) {
                if let Ok(rows) = stmt.query_map([], |row| {
                    let title: String = row.get(0).unwrap_or_default();
                    let mtype: String = row.get(1).unwrap_or_default();
                    let content: String = row.get(2).unwrap_or_default();
                    Ok((title.trim_matches('"').to_string(), mtype, content.trim_matches('"').to_string()))
                }) {
                    for row in rows.flatten() {
                        if !row.0.is_empty() {
                            let preview: String = row.2.chars().take(120).collect();
                            parts.push(format!("  [{}] {} — {}", row.1, row.0, preview));
                        }
                    }
                }
            }
        }
        let ctx = parts.join("\n");
        if ctx.len() as u32 > token_budget * 4 {
            let end = ctx.char_indices().nth(token_budget as usize * 4).map(|(i,_)| i).unwrap_or(ctx.len());
            format!("{}\n...（已截断）", &ctx[..end])
        } else {
            ctx
        }
    }

    async fn remember(&self, content: &str, mem_type: &str) -> anyhow::Result<String> {
        let id = format!("mem_{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let data = serde_json::json!({
            "title": &content[..content.len().min(80)],
            "content": content,
            "mem_type": mem_type,
            "strength": 5,
            "version": 1,
        });
        let conn = self.db.lock().expect("MemoryCore db lock");
        conn.execute(
            "INSERT INTO memories (id, created_at, updated_at, mem_type, data) VALUES (?1, ?2, ?2, ?3, ?4)",
            rusqlite::params![id, now, mem_type, data.to_string()],
        )?;
        Ok(id)
    }

    async fn search(&self, _query: &str, _limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        Ok(Vec::new())
    }

    async fn session_start(&self, _session_id: &str, _project: &str, _cwd: &str) {}

    async fn session_end(&self) {}
}

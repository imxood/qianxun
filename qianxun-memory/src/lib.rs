pub mod compressor;
pub mod consolidation;
pub mod db;
pub mod privacy;
pub mod search;
pub mod slot;
pub mod types;
pub mod vector;

use async_trait::async_trait;
use qianxun_core::context::{MemoryObserver, SearchResult};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::warn;

/// 记忆数据库轻量统计 (供 `GET /v1/memory/ping` 使用).
///
/// 三张核心表:
/// - `observations` — 压缩后的单步观测 (observe 写入)
/// - `memories`     — 跨会话持久记忆 (remember 写入)
/// - `sessions`     — session 生命周期记录 (session_start 写入)
///
/// `i64` 对应 SQLite INTEGER PRIMARY KEY / COUNT(*); 用 i64 是因为
/// `rusqlite::types::Value::Integer` 走 `FromSql for i64` 最稳, u64 容易
/// 在大于 i64::MAX 的极端值 (实际不会) 触发溢出警告.
#[derive(Debug, Clone, Copy)]
pub struct MemoryStats {
    pub observation_count: i64,
    pub memory_count: i64,
    pub session_count: i64,
}

/// 当前活跃 session 的上下文。
///
/// observe() 期间需要用真实 session_id 替代早期实现的硬编码 "global"。
/// 这里用进程内 Mutex 持有，最后一次 session_start 的值生效。
#[derive(Debug, Clone)]
#[allow(dead_code)] // project/cwd 暂未在 hot path 读取，保留用于未来 audit 日志
struct CurrentSession {
    session_id: String,
    project: String,
    cwd: String,
}

/// MemoryCore — 记忆引擎入口。
///
/// 持有 SQLite 连接，实现 `MemoryObserver` trait。
/// 所有 SQLite 操作通过 `spawn_blocking` 派发到 blocking 线程池，
/// 避免在 async 上下文中持 std::sync::Mutex 锁阻塞 tokio reactor。
pub struct MemoryCore {
    db: Arc<Mutex<Connection>>,
    current_session: Arc<Mutex<Option<CurrentSession>>>,
}

impl MemoryCore {
    /// 打开或创建记忆数据库。
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = db::open(path)?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            current_session: Arc::new(Mutex::new(None)),
        })
    }

    /// 创建内存数据库（用于测试或 fallback）。
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        crate::db::create_tables(&conn)?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            current_session: Arc::new(Mutex::new(None)),
        })
    }

    /// Stage 7b: 同步删除单个 observation (供 `DELETE /v1/memory/observations/{id}`).
    ///
    /// 走 `spawn_blocking` 避免持 `std::sync::Mutex` 锁阻塞 tokio reactor.
    /// 触发器 `obs_ad_fts` 自动同步 FTS5 索引.
    pub async fn delete_observation(&self, id: &str) -> anyhow::Result<bool> {
        let db = self.db.clone();
        let id = id.to_string();
        let affected = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let conn = db.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            conn.execute("DELETE FROM observations WHERE id = ?1", params![id])
        })
        .await
        .map_err(|e| anyhow::anyhow!("delete_observation join: {e}"))??;
        Ok(affected > 0)
    }

    /// Stage 7b: 同步删除整个 memory session 及其级联 observations (供
    /// `DELETE /v1/memory/sessions/{id}`). 走 `spawn_blocking`.
    ///
    /// 返回删除的 session 数 (0 或 1).
    /// 注: observations / session_summaries 表的 FK 没有 CASCADE 声明, 需
    /// 手动先删观测再删 session.
    pub async fn delete_session(&self, id: &str) -> anyhow::Result<usize> {
        let db = self.db.clone();
        let id = id.to_string();
        let n = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let conn = db.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            // 先删依赖 (observations, session_summaries, raw_observations)
            // 注: PRAGMA foreign_keys 在 db::open 已 ON, 手动删避免 FK NO ACTION 报错
            conn.execute(
                "DELETE FROM observations WHERE session_id = ?1",
                rusqlite::params![id],
            )?;
            conn.execute(
                "DELETE FROM session_summaries WHERE session_id = ?1",
                rusqlite::params![id],
            )?;
            conn.execute(
                "DELETE FROM raw_observations WHERE session_id = ?1",
                rusqlite::params![id],
            )?;
            // 最后删 session 行
            conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
        })
        .await
        .map_err(|e| anyhow::anyhow!("delete_session join: {e}"))??;
        Ok(n)
    }

    /// Day-3.2: 轻量统计 (observations / memories / sessions 三表行数).
    ///
    /// 供 `GET /v1/memory/ping` 使用 — 验证 MemoryCore 可达 + 给出当前
    /// 数据库体量. 走 `spawn_blocking` 避免持 `std::sync::Mutex` 锁阻塞
    /// tokio reactor (跟 `delete_observation` / `delete_session` 同模式).
    ///
    /// 返回 `MemoryStats`, 调用方决定如何序列化. 任何 SQLite 错误都向上
    /// 抛 `anyhow::Error` (handler 端映射 500).
    pub async fn stats(&self) -> anyhow::Result<MemoryStats> {
        let db = self.db.clone();
        let stats = tokio::task::spawn_blocking(move || -> rusqlite::Result<MemoryStats> {
            let conn = db.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let observation_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))?;
            let memory_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
            let session_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
            Ok(MemoryStats {
                observation_count,
                memory_count,
                session_count,
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("stats spawn_blocking join: {e}"))??;
        Ok(stats)
    }
}

/// 在 char 边界安全地按"字符预算"截取字符串。
///
/// 中文 UTF-8 占 3 字节，按字节切片会切到字符中间 panic。
/// 该函数按 char 迭代，最多保留 `max_chars` 个字符（向上取整 4 字节估算 token）。
fn truncate_to_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars * 4));
    for c in s.chars().take(max_chars) {
        out.push(c);
    }
    out
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
        // 1. 读 current_session
        let session = {
            let guard = self.current_session.lock().ok();
            guard.and_then(|g| g.clone())
        };
        let Some(session) = session else {
            // 没有 active session 时丢弃 observation（与早期 hardcode "global" 的行为相比，
            // 现在的语义更明确：不写入未关联 session 的垃圾数据）
            tracing::debug!(
                "[memory] observe({tool_name}) dropped: no active session (call session_start first)"
            );
            return;
        };

        // 2. 合成 observation
        let obs_id = format!("obs_{}", uuid::Uuid::new_v4());
        let clean_output = tool_output.map(privacy::strip_private_data);
        let observation = compressor::build_synthetic(
            obs_id.clone(),
            session.session_id.clone(),
            hook_type,
            tool_name,
            tool_input.as_ref(),
            clean_output.as_deref(),
        );
        let data = match serde_json::to_string(&observation) {
            Ok(s) => s,
            Err(e) => {
                warn!("[memory] serialize observation failed: {e}");
                return;
            }
        };
        let timestamp = observation.timestamp.to_rfc3339();

        // 3. 写 SQLite（spawn_blocking 避免持锁阻塞 reactor）
        let db = self.db.clone();
        let result = tokio::task::spawn_blocking(move || {
            let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock poisoned: {e}"))?;
            conn.execute(
                "INSERT INTO observations (id, session_id, timestamp, data) VALUES (?1, ?2, ?3, ?4)",
                params![obs_id, session.session_id, timestamp, data],
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .await;

        if let Err(e) = result {
            warn!("[memory] observe spawn_blocking join error: {e}");
        }
    }

    async fn build_context(&self, query: &str, token_budget: u32) -> String {
        let db = self.db.clone();
        let query = query.to_string();

        let ctx = tokio::task::spawn_blocking(move || build_context_sync(&db, &query, token_budget))
            .await
            .unwrap_or_else(|e| {
                warn!("[memory] build_context spawn_blocking failed: {e}");
                String::new()
            });

        // token 预算裁剪（按字符数近似，1 token ≈ 4 字符）
        let char_budget = (token_budget as usize).saturating_mul(4);
        if ctx.chars().count() > char_budget {
            let truncated = truncate_to_chars(&ctx, char_budget);
            format!("{truncated}\n...（已截断）")
        } else {
            ctx
        }
    }

    async fn remember(&self, content: &str, mem_type: &str) -> anyhow::Result<String> {
        // 1. char 边界安全截断（中文按字符计，预算 80 字符）
        let title = truncate_to_chars(content, 80);
        let id = format!("mem_{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let data = serde_json::json!({
            "title": title,
            "content": content,
            "mem_type": mem_type,
            "strength": 5,
            "version": 1,
        })
        .to_string();
        // mem_type 是 &str,必须 to_string() 才能 move 进 spawn_blocking 闭包
        let mem_type = mem_type.to_string();

        let db = self.db.clone();
        let id_clone = id.clone();
        tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
            let conn = db.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            conn.execute(
                "INSERT INTO memories (id, created_at, updated_at, mem_type, data) VALUES (?1, ?2, ?2, ?3, ?4)",
                params![id_clone, now, mem_type, data],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("remember spawn_blocking join: {e}"))?
        .map_err(|e| anyhow::anyhow!("remember insert: {e}"))?;

        Ok(id)
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let db = self.db.clone();
        let query = query.to_string();

        let result = tokio::task::spawn_blocking(move || search_sync(&db, &query, limit)).await;

        match result {
            Ok(r) => r,
            Err(e) => {
                warn!("[memory] search spawn_blocking failed: {e}");
                Ok(Vec::new())
            }
        }
    }

    async fn session_start(&self, session_id: &str, project: &str, cwd: &str) {
        // 1. 更新内存中的 current_session
        {
            let mut guard = match self.current_session.lock() {
                Ok(g) => g,
                Err(e) => {
                    warn!("[memory] session_start current_session lock poisoned: {e}");
                    return;
                }
            };
            *guard = Some(CurrentSession {
                session_id: session_id.to_string(),
                project: project.to_string(),
                cwd: cwd.to_string(),
            });
        }

        // 2. 写 sessions 表
        let db = self.db.clone();
        let sid = session_id.to_string();
        let proj = project.to_string();
        let cwd_s = cwd.to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let result = tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
            let conn = db.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            conn.execute(
                "INSERT INTO sessions (id, project, cwd, started_at, status) \
                 VALUES (?1, ?2, ?3, ?4, 'active') \
                 ON CONFLICT(id) DO UPDATE SET \
                    project = excluded.project, \
                    cwd = excluded.cwd, \
                    started_at = excluded.started_at, \
                    status = 'active', \
                    ended_at = NULL",
                params![sid, proj, cwd_s, now],
            )?;
            Ok(())
        })
        .await;

        if let Err(e) = result {
            warn!("[memory] session_start spawn_blocking join: {e}");
        }
    }

    async fn session_end(&self) {
        // 1. 取 session_id
        let session = {
            let mut guard = match self.current_session.lock() {
                Ok(g) => g,
                Err(e) => {
                    warn!("[memory] session_end current_session lock poisoned: {e}");
                    return;
                }
            };
            guard.take()
        };

        let Some(session) = session else {
            return;
        };

        // 2. 更新 sessions 表
        let db = self.db.clone();
        let now = chrono::Utc::now().to_rfc3339();
        let sid = session.session_id.clone();

        let result = tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
            let conn = db.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
            conn.execute(
                "UPDATE sessions SET ended_at = ?1, status = 'ended' WHERE id = ?2",
                params![now, sid],
            )?;
            Ok(())
        })
        .await;

        if let Err(e) = result {
            warn!("[memory] session_end spawn_blocking join: {e}");
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// 同步辅助函数：在 spawn_blocking 闭包内执行
// ──────────────────────────────────────────────────────────────────────────

/// 构建记忆上下文：FTS5 搜索 observations + 列出最近 memories。
fn build_context_sync(db: &Arc<Mutex<Connection>>, query: &str, _token_budget: u32) -> String {
    let conn = match db.lock() {
        Ok(c) => c,
        Err(e) => {
            warn!("[memory] build_context db lock: {e}");
            return String::new();
        }
    };
    let mut parts: Vec<String> = Vec::new();

    // 1. FTS5 搜索 observations（仅当 query 非空）
    if !query.trim().is_empty() {
        let fts_query: String = query
            .split_whitespace()
            .filter(|w| w.chars().count() > 1)
            .collect::<Vec<_>>()
            .join(" ");

        if !fts_query.is_empty() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT o.id, json_extract(o.data, '$.title'), json_extract(o.data, '$.narrative') \
                 FROM obs_fts f JOIN observations o ON o.rowid = f.rowid \
                 WHERE obs_fts MATCH ?1 ORDER BY rank LIMIT 5",
            ) {
                if let Ok(rows) = stmt.query_map(params![fts_query], |row| {
                    let title: String = row.get(1).unwrap_or_default();
                    let narrative: String = row.get(2).unwrap_or_default();
                    Ok((title, narrative))
                }) {
                    for row in rows.flatten() {
                        let title = row.0.trim_matches('"').to_string();
                        let narrative = row.1.trim_matches('"').to_string();
                        if !title.is_empty() {
                            let preview = truncate_to_chars(&narrative, 80);
                            if preview.is_empty() || preview == title {
                                parts.push(format!("- {title}"));
                            } else {
                                parts.push(format!("- {title} — {preview}"));
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. 最近的持久 memories
    if let Ok(mut stmt) = conn.prepare(
        "SELECT json_extract(data, '$.title'), json_extract(data, '$.mem_type'), \
                json_extract(data, '$.content') \
         FROM memories ORDER BY created_at DESC LIMIT 10",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            let title: String = row.get(0).unwrap_or_default();
            let mtype: String = row.get(1).unwrap_or_default();
            let content: String = row.get(2).unwrap_or_default();
            Ok((title, mtype, content))
        }) {
            for row in rows.flatten() {
                let title = row.0.trim_matches('"').to_string();
                if !title.is_empty() {
                    let preview = truncate_to_chars(row.2.trim_matches('"'), 80);
                    if preview == title {
                        parts.push(format!("  [{}] {}", row.1, title));
                    } else {
                        parts.push(format!("  [{}] {} — {}", row.1, title, preview));
                    }
                }
            }
        }
    }

    parts.join("\n")
}

/// FTS5 搜索 observations，返回 BM25 排序的 SearchResult。
fn search_sync(
    db: &Arc<Mutex<Connection>>,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<SearchResult>> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("db lock: {e}"))?;

    let fts_query: String = query
        .split_whitespace()
        .filter(|w| w.chars().count() > 1)
        .collect::<Vec<_>>()
        .join(" ");

    if fts_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT o.id, o.session_id, o.timestamp, \
                json_extract(o.data, '$.title'), \
                json_extract(o.data, '$.narrative'), \
                json_extract(o.data, '$.concepts'), \
                json_extract(o.data, '$.files'), \
                json_extract(o.data, '$.importance'), \
                rank \
         FROM obs_fts f JOIN observations o ON o.rowid = f.rowid \
         WHERE obs_fts MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;

    let results: Vec<SearchResult> = stmt
        .query_map(params![fts_query, limit as i64], |row| {
            let id: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            let timestamp: String = row.get(2)?;
            let title: String = row.get(3).unwrap_or_default();
            let narrative: String = row.get(4).unwrap_or_default();
            let concepts_json: String = row.get(5).unwrap_or_else(|_| "[]".into());
            let files_json: String = row.get(6).unwrap_or_else(|_| "[]".into());
            let importance: u8 = row.get(7).unwrap_or(0);
            let score: f64 = row.get(8).unwrap_or(0.0);

            let concepts: Vec<String> = serde_json::from_str(&concepts_json).unwrap_or_default();
            let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();

            Ok(SearchResult {
                id,
                session_id,
                timestamp,
                title: title.trim_matches('"').to_string(),
                narrative: narrative.trim_matches('"').to_string(),
                concepts,
                files,
                importance,
                score,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qianxun_core::context::MemoryObserver;
    use serde_json::json;

    /// 创建带 active session 的 in-memory MemoryCore。
    async fn fresh() -> MemoryCore {
        let core = MemoryCore::open_in_memory().expect("open_in_memory");
        core.session_start("sess_test_01", "test-project", "/tmp/test")
            .await;
        core
    }

    #[tokio::test]
    async fn truncate_to_chars_handles_cjk() {
        // 中文 4 个字符 = 12 字节；按字节切到 5 会 panic
        let s = "千寻项目状态更新：Phase C 内存";
        // 10 个字符（8 CJK + ":" + "P"）= "千寻项目状态更新：P"
        let out = truncate_to_chars(s, 10);
        assert_eq!(out, "千寻项目状态更新：P");
        assert!(out.is_char_boundary(out.len()));

        // 截到 0 / 超过长度都不应 panic
        assert_eq!(truncate_to_chars(s, 0), "");
        let long = truncate_to_chars(s, 1000);
        assert_eq!(long, s);
    }

    #[tokio::test]
    async fn remember_with_chinese_title_does_not_panic() {
        let core = fresh().await;
        // 80 字节会切到中文字符中间；现在用 char 边界
        let content: String = "千寻".repeat(100); // 300 字节
        let id = core.remember(&content, "pattern").await.expect("remember ok");
        assert!(id.starts_with("mem_"));
    }

    #[tokio::test]
    async fn observe_writes_observation_with_real_session_id() {
        let core = fresh().await;
        let input = json!({"path": "src/main.rs"});
        core.observe("PostToolUse", "read_file", Some(input), Some("content"))
            .await;

        // 验证 observation 写入且 session_id 是真实值
        let db = core.db.clone();
        let rows: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
            let conn = db.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, session_id FROM observations")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        })
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "sess_test_01"); // 不再是 "global"
    }

    #[tokio::test]
    async fn observe_without_session_is_dropped_silently() {
        // 没有 session_start → observe 应当 no-op 而不是 panic
        let core = MemoryCore::open_in_memory().unwrap();
        core.observe("PostToolUse", "read_file", Some(json!({})), Some("x"))
            .await;

        let db = core.db.clone();
        let count: i64 = tokio::task::spawn_blocking(move || {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn fts_trigger_indexes_new_observations() {
        let core = fresh().await;
        // observe 一条带特定 path 的记录（title 和 narrative 都会含 path token）
        // 注：compressor::compress_read 暂未把 tool_output 写入 narrative，
        // 所以本测试用 path token 验证 FTS 同步链路。
        let input = json!({"path": "config.toml"});
        core.observe("PostToolUse", "read_file", Some(input), Some("hello world"))
            .await;

        // 触发 FTS 同步（trigger 自动）后，search 应能找到
        let results = core.search("config", 10).await.expect("search");
        assert!(!results.is_empty(), "FTS should find the observation");
        assert!(results[0].narrative.contains("config") || results[0].title.contains("config"));
    }

    #[tokio::test]
    async fn build_context_returns_recent_observations_and_memories() {
        let core = fresh().await;
        core.observe("PostToolUse", "read_file", Some(json!({"path": "alpha.rs"})), Some("alpha content"))
            .await;
        core.remember("这是用户偏好：偏好深色模式", "preference")
            .await
            .unwrap();

        // query 与 obs 匹配 → FTS 路径
        let ctx = core.build_context("alpha", 500).await;
        assert!(ctx.contains("alpha.rs") || ctx.contains("alpha content"));

        // 空 query → 不调 FTS，只列 memories
        let ctx = core.build_context("", 500).await;
        assert!(ctx.contains("深色模式") || ctx.contains("preference"));
    }

    #[tokio::test]
    async fn session_lifecycle_writes_sessions_table() {
        let core = MemoryCore::open_in_memory().unwrap();
        core.session_start("sess_lifecycle", "my-proj", "/work")
            .await;

        let db = core.db.clone();
        let row: (String, String, String, String) = tokio::task::spawn_blocking(move || {
            let conn = db.lock().unwrap();
            conn.query_row(
                "SELECT id, project, cwd, status FROM sessions WHERE id = ?1",
                params!["sess_lifecycle"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
        })
        .await
        .unwrap();

        assert_eq!(row.0, "sess_lifecycle");
        assert_eq!(row.1, "my-proj");
        assert_eq!(row.2, "/work");
        assert_eq!(row.3, "active");

        // session_end → 状态变 ended
        core.session_end().await;
        let db = core.db.clone();
        let status: String = tokio::task::spawn_blocking(move || {
            let conn = db.lock().unwrap();
            conn.query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params!["sess_lifecycle"],
                |row| row.get(0),
            )
            .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(status, "ended");
    }

    #[tokio::test]
    async fn multi_session_observations_are_isolated_by_session_id() {
        // session A 写一条
        let core_a = MemoryCore::open_in_memory().unwrap();
        core_a.session_start("sess_A", "proj-A", "/a").await;
        core_a.observe("PostToolUse", "read_file", Some(json!({"path": "a_unique.rs"})), Some("token A"))
            .await;

        // session B 写一条
        let core_b = MemoryCore::open_in_memory().unwrap();
        core_b.session_start("sess_B", "proj-B", "/b").await;
        core_b.observe("PostToolUse", "read_file", Some(json!({"path": "b_unique.rs"})), Some("token B"))
            .await;

        // 它们是独立的 db，但都是 in-memory — 各自验证 path token 隔离
        let r_a = core_a.search("a_unique", 10).await.unwrap();
        let r_b = core_b.search("b_unique", 10).await.unwrap();
        assert_eq!(r_a.len(), 1, "session A should find its own obs");
        assert!(r_a[0].narrative.contains("a_unique"));
        assert_eq!(r_b.len(), 1, "session B should find its own obs");
        assert!(r_b[0].narrative.contains("b_unique"));
    }
}

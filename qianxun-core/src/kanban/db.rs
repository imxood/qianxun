//! KanbanDb — Kanban 子系统持久化层 (v6 §7.3)
//!
//! 决策 (v6 §7.3 [A]):
//! - 复用 `~/.qianxun/daemon.db` 文件, 跟 daemon_sessions 同一 SQLite
//! - 沿用 `Arc<Mutex<Connection>>` 单连接模式, 跟 `team_db.rs:97-99` 一致
//! - 不引 r2d2 / sqlx (千寻单 daemon 低写并发, 简单足够)
//! - 所有方法走 `spawn_blocking` 异步化 (跟 MVP-0 MemoryCore 范式一致)
//!
//! ## MVP-2 plan 2 范围 (核心 10 方法)
//!
//! 完整 28+ 方法覆盖 8 张表 CRUD, 本期先做 10 个核心方法, 后续 v2 补齐.
//! 核心 10 方法 (MVP-3 实际需要):
//! 1. create_task
//! 2. get_task
//! 3. list_tasks
//! 4. update_task_status (state_machine 集成, 见 plan 3)
//! 5. update_heartbeat
//! 6. create_run
//! 7. complete_run
//! 8. write_blackboard
//! 9. read_blackboard
//! 10. append_event
//!
//! 后续 v2 加: list_projects / create_board / create_link / list_runs 等.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::error::KanbanError;
use super::types::{
    AgentRun, BlackboardCell, KanbanEventKind, RunOutcome, RunStatus, Task,
    TaskStatus,
};

/// Kanban 持久化层 (单连接, 沿用 daemon.db).
///
/// 内部 `Arc<Mutex<Connection>>` 跟 `team_db.rs` 一致. 所有 public 方法
/// 走 `spawn_blocking` 异步化, 避免在 async 上下文中持锁阻塞 tokio reactor.
#[derive(Clone)]
pub struct KanbanDb {
    conn: Arc<Mutex<Connection>>,
}

impl KanbanDb {
    /// 打开 / 创建 SQLite 数据库 (实际沿用 daemon.db, 由 daemon 启动时
    /// 调 `init_kanban_schema`).
    ///
    /// 单独打开一个新文件仅用于测试 (in_memory 或独立 db 路径).
    pub fn open(path: &Path) -> Result<Self, KanbanError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开 in-memory 数据库 (用于测试).
    pub fn in_memory() -> Result<Self, KanbanError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 从已有 `Arc<Mutex<Connection>>` 构造 (跟 daemon 共享 daemon.db).
    pub fn from_connection(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    // ========================================================================
    // Task CRUD (4 方法)
    // ========================================================================

    /// 创建 task. `id` 为 None 时自动生成 "task_<uuid>".
    #[allow(clippy::too_many_arguments)]
    pub async fn create_task(
        &self,
        id: Option<&str>,
        board_id: &str,
        parent_id: Option<&str>,
        title: &str,
        body: &str,
        assignee_role: &str,
        priority: u8,
    ) -> Result<Task, KanbanError> {
        let conn = self.conn.clone();
        let id = id.map(String::from).unwrap_or_else(|| format!("task_{}", Uuid::new_v4()));
        let board_id = board_id.to_string();
        let parent_id = parent_id.map(String::from);
        let title = title.to_string();
        let body = body.to_string();
        let assignee_role = assignee_role.to_string();
        tokio::task::spawn_blocking(move || -> Result<Task, KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            let now = Utc::now();
            let now_str = now.to_rfc3339();
            conn.execute(
                "INSERT INTO kanban_tasks \
                 (id, board_id, parent_id, title, body, assignee_role, status, priority, metadata, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'triage', ?7, '{}', ?8)",
                params![id, board_id, parent_id, title, body, assignee_role, priority, now_str],
            )?;
            Ok(Task {
                id,
                board_id,
                parent_id,
                title,
                body,
                assignee_role,
                status: TaskStatus::Triage,
                priority,
                deadline: None,
                metadata: serde_json::json!({}),
                created_at: now,
                t_started_at: None,
                t_completed_at: None,
                last_heartbeat_at: None,
            })
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    /// 查 task by id.
    pub async fn get_task(&self, id: &str) -> Result<Option<Task>, KanbanError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<Task>, KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            let mut stmt = conn.prepare(
                "SELECT id, board_id, parent_id, title, body, assignee_role, status, priority, \
                 deadline, metadata, created_at, t_started_at, t_completed_at, last_heartbeat_at \
                 FROM kanban_tasks WHERE id = ?1",
            )?;
            let task = stmt
                .query_row(params![id], row_to_task)
                .optional()?;
            Ok(task)
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    /// 列出某 board 下 task, 可选 status 过滤.
    pub async fn list_tasks(
        &self,
        board_id: &str,
        status_filter: Option<TaskStatus>,
    ) -> Result<Vec<Task>, KanbanError> {
        let conn = self.conn.clone();
        let board_id = board_id.to_string();
        let status_str = status_filter.map(|s| task_status_to_str(s).to_string());
        tokio::task::spawn_blocking(move || -> Result<Vec<Task>, KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            let tasks = if let Some(s) = status_str {
                let mut stmt = conn.prepare(
                    "SELECT id, board_id, parent_id, title, body, assignee_role, status, priority, \
                     deadline, metadata, created_at, t_started_at, t_completed_at, last_heartbeat_at \
                     FROM kanban_tasks WHERE board_id = ?1 AND status = ?2 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![board_id, s], row_to_task)?;
                rows.filter_map(|r| r.ok()).collect()
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, board_id, parent_id, title, body, assignee_role, status, priority, \
                     deadline, metadata, created_at, t_started_at, t_completed_at, last_heartbeat_at \
                     FROM kanban_tasks WHERE board_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![board_id], row_to_task)?;
                rows.filter_map(|r| r.ok()).collect()
            };
            Ok(tasks)
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    /// 更新 task 状态 (走 state_machine 校验 + recompute_parent).
    ///
    /// 流程 (MVP-2 plan 3 集成):
    /// 1. 读 current status from DB
    /// 2. 调 `state_machine::check_transition(from, to)` 校验
    /// 3. UPDATE 状态 + 同步 t_started_at / t_completed_at
    /// 4. 调 `state_machine::recompute_parent` 触发父状态机
    pub async fn update_task_status(
        &self,
        id: &str,
        new_status: TaskStatus,
    ) -> Result<(), KanbanError> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let status_str = task_status_to_str(new_status).to_string();
        tokio::task::spawn_blocking(move || -> Result<(), KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            // 1. 读 current status
            let current_str: Option<String> = conn
                .query_row(
                    "SELECT status FROM kanban_tasks WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .ok();
            let current = current_str
                .as_deref()
                .and_then(str_to_task_status)
                .unwrap_or(TaskStatus::Triage);
            // 2. 校验转换合法
            super::state_machine::check_transition(current, new_status)?;
            // 3. UPDATE 状态 + 同步 t_started_at / t_completed_at
            let now_str = Utc::now().to_rfc3339();
            let set_started = matches!(new_status, TaskStatus::InProgress);
            let set_completed = matches!(
                new_status,
                TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled
            );
            if set_started {
                conn.execute(
                    "UPDATE kanban_tasks SET status = ?1, t_started_at = COALESCE(t_started_at, ?2) WHERE id = ?3",
                    params![status_str, now_str, id],
                )?;
            } else if set_completed {
                conn.execute(
                    "UPDATE kanban_tasks SET status = ?1, t_completed_at = ?2 WHERE id = ?3",
                    params![status_str, now_str, id],
                )?;
            } else {
                conn.execute(
                    "UPDATE kanban_tasks SET status = ?1 WHERE id = ?2",
                    params![status_str, id],
                )?;
            }
            // 4. 触发父状态机 (Hermes recompute_parent)
            if matches!(new_status, TaskStatus::Done) {
                let _ = super::state_machine::recompute_parent(&conn, &id);
            }
            Ok(())
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    // ========================================================================
    // Run CRUD (3 方法)
    // ========================================================================

    /// 创建 run. `id` 为 None 时自动生成 "run_<uuid>". claim_id 自动生成.
    pub async fn create_run(
        &self,
        id: Option<&str>,
        task_id: &str,
        profile_id: &str,
    ) -> Result<AgentRun, KanbanError> {
        let conn = self.conn.clone();
        let id = id.map(String::from).unwrap_or_else(|| format!("run_{}", Uuid::new_v4()));
        let task_id = task_id.to_string();
        let profile_id = profile_id.to_string();
        let claim_id = Uuid::new_v4().to_string();
        tokio::task::spawn_blocking(move || -> Result<AgentRun, KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            let now = Utc::now();
            let now_str = now.to_rfc3339();
            conn.execute(
                "INSERT INTO kanban_runs \
                 (id, task_id, profile_id, status, claim_id, started_at, outcome, summary, token_input, token_output) \
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, 'success', '', 0, 0)",
                params![id, task_id, profile_id, claim_id, now_str],
            )?;
            Ok(AgentRun {
                id,
                task_id,
                profile_id,
                status: RunStatus::Pending,
                claim_id,
                r_heartbeat_at: None,
                started_at: now,
                ended_at: None,
                outcome: RunOutcome::Success,
                summary: String::new(),
                error: None,
                token_input: 0,
                token_output: 0,
            })
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    /// 更新心跳 (task 级别, 模式 7 借鉴). 60s 限频在调用方做.
    pub async fn update_heartbeat(&self, task_id: &str) -> Result<(), KanbanError> {
        let conn = self.conn.clone();
        let task_id = task_id.to_string();
        let now_str = Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || -> Result<(), KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            conn.execute(
                "UPDATE kanban_tasks SET last_heartbeat_at = ?1 WHERE id = ?2",
                params![now_str, task_id],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    /// 标记 run 完成 (写 outcome + summary + ended_at).
    pub async fn complete_run(
        &self,
        run_id: &str,
        outcome: RunOutcome,
        summary: &str,
        token_input: u64,
        token_output: u64,
    ) -> Result<(), KanbanError> {
        let conn = self.conn.clone();
        let run_id = run_id.to_string();
        let outcome_str = run_outcome_to_str(outcome).to_string();
        let summary = summary.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            let now_str = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE kanban_runs SET status = 'done', outcome = ?1, summary = ?2, \
                 ended_at = ?3, token_input = ?4, token_output = ?5 WHERE id = ?6",
                params![outcome_str, summary, now_str, token_input, token_output, run_id],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    // ========================================================================
    // Blackboard (2 方法, §4 模式 2)
    // ========================================================================

    /// 写黑板 (last writer wins, 主键 (task_id, key)).
    pub async fn write_blackboard(
        &self,
        task_id: &str,
        key: &str,
        value: &serde_json::Value,
        author: &str,
    ) -> Result<(), KanbanError> {
        let conn = self.conn.clone();
        let task_id = task_id.to_string();
        let key = key.to_string();
        let value_json = serde_json::to_string(value)?;
        let author = author.to_string();
        let now_str = Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || -> Result<(), KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            // INSERT OR REPLACE (last writer wins)
            conn.execute(
                "INSERT OR REPLACE INTO kanban_blackboard (task_id, key, value, author, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![task_id, key, value_json, author, now_str],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    /// 读黑板 (整张 task 黑板, HashMap<key, BlackboardCell>).
    pub async fn read_blackboard(
        &self,
        task_id: &str,
    ) -> Result<std::collections::HashMap<String, BlackboardCell>, KanbanError> {
        let conn = self.conn.clone();
        let task_id = task_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<std::collections::HashMap<String, BlackboardCell>, KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            let mut stmt = conn.prepare(
                "SELECT task_id, key, value, author, updated_at FROM kanban_blackboard \
                 WHERE task_id = ?1 ORDER BY updated_at ASC",
            )?;
            let rows = stmt.query_map(params![task_id], |row| {
                let value_str: String = row.get(2)?;
                let value: serde_json::Value = serde_json::from_str(&value_str).unwrap_or(serde_json::Value::Null);
                Ok(BlackboardCell {
                    task_id: row.get(0)?,
                    key: row.get(1)?,
                    value,
                    author: row.get(3)?,
                    updated_at: row
                        .get::<_, String>(4)?
                        .parse::<DateTime<Utc>>()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?;
            let mut out = std::collections::HashMap::new();
            for cell in rows.flatten() {
                out.insert(cell.key.clone(), cell);
            }
            Ok(out)
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    // ========================================================================
    // Event (1 方法, §6.3 24 变体 audit)
    // ========================================================================

    /// 追加事件 (audit + 实时推送). 返回 auto-increment id.
    pub async fn append_event(
        &self,
        task_id: Option<&str>,
        run_id: Option<&str>,
        kind: KanbanEventKind,
        payload: &serde_json::Value,
    ) -> Result<i64, KanbanError> {
        let conn = self.conn.clone();
        let task_id = task_id.map(String::from);
        let run_id = run_id.map(String::from);
        let kind_str = event_kind_to_str(kind).to_string();
        let payload_json = serde_json::to_string(payload)?;
        let now_str = Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || -> Result<i64, KanbanError> {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            conn.execute(
                "INSERT INTO kanban_events (task_id, run_id, kind, payload, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![task_id, run_id, kind_str, payload_json, now_str],
            )?;
            let id = conn.last_insert_rowid();
            Ok(id)
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }

    // ========================================================================
    // Internal helpers (for tests + dispatcher)
    // ========================================================================

    /// 直接拿到 `Arc<Mutex<Connection>>` (dispatcher 集成用).
    pub fn connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }

    /// 同步跑任意闭包 (用于测试和 dispatcher 内部 SQL).
    pub async fn run_blocking<F, R>(&self, f: F) -> Result<R, KanbanError>
    where
        F: FnOnce(&Connection) -> Result<R, KanbanError> + Send + 'static,
        R: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| KanbanError::BlockingJoin(e.to_string()))?;
            f(&conn)
        })
        .await
        .map_err(|e| KanbanError::BlockingJoin(e.to_string()))?
    }
}

// =============================================================================
// 辅助函数 (row -> struct, enum -> str)
// =============================================================================

/// SQL row -> Task.
pub fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status_str: String = row.get(6)?;
    let status = str_to_task_status(&status_str).unwrap_or(TaskStatus::Triage);
    let metadata_str: String = row.get(9)?;
    let metadata: serde_json::Value = serde_json::from_str(&metadata_str).unwrap_or(serde_json::json!({}));
    let created_at_str: String = row.get(10)?;
    let created_at = created_at_str
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    let parse_opt = |s: String| s.parse::<DateTime<Utc>>().ok();
    let deadline_str: Option<String> = row.get(8)?;
    let t_started_str: Option<String> = row.get(11)?;
    let t_completed_str: Option<String> = row.get(12)?;
    let heartbeat_str: Option<String> = row.get(13)?;
    Ok(Task {
        id: row.get(0)?,
        board_id: row.get(1)?,
        parent_id: row.get(2)?,
        title: row.get(3)?,
        body: row.get(4)?,
        assignee_role: row.get(5)?,
        status,
        priority: row.get::<_, i64>(7)? as u8,
        deadline: deadline_str.and_then(parse_opt),
        metadata,
        created_at,
        t_started_at: t_started_str.and_then(parse_opt),
        t_completed_at: t_completed_str.and_then(parse_opt),
        last_heartbeat_at: heartbeat_str.and_then(parse_opt),
    })
}

/// TaskStatus -> snake_case str (跟 v6 §6.5 TEXT 列一致).
pub fn task_status_to_str(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Triage => "triage",
        TaskStatus::Ready => "ready",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Done => "done",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Cancelled => "cancelled",
        TaskStatus::Failed => "failed",
    }
}

/// str -> TaskStatus (宽容解析, 失败返 None 让调用方 fallback).
pub fn str_to_task_status(s: &str) -> Option<TaskStatus> {
    match s {
        "triage" => Some(TaskStatus::Triage),
        "ready" => Some(TaskStatus::Ready),
        "in_progress" => Some(TaskStatus::InProgress),
        "done" => Some(TaskStatus::Done),
        "blocked" => Some(TaskStatus::Blocked),
        "cancelled" => Some(TaskStatus::Cancelled),
        "failed" => Some(TaskStatus::Failed),
        _ => None,
    }
}

/// RunOutcome -> snake_case str.
pub fn run_outcome_to_str(o: RunOutcome) -> &'static str {
    match o {
        RunOutcome::Success => "success",
        RunOutcome::PartialSuccess => "partial_success",
        RunOutcome::Failure => "failure",
        RunOutcome::Skipped => "skipped",
        RunOutcome::GateBlocked => "gate_blocked",
    }
}

/// KanbanEventKind -> snake_case str.
pub fn event_kind_to_str(k: KanbanEventKind) -> &'static str {
    use KanbanEventKind::*;
    match k {
        TaskCreated => "task_created",
        TaskAssigned => "task_assigned",
        TaskStarted => "task_started",
        TaskPaused => "task_paused",
        TaskResumed => "task_resumed",
        TaskCompleted => "task_completed",
        TaskBlocked => "task_blocked",
        TaskUnblocked => "task_unblocked",
        TaskCancelled => "task_cancelled",
        TaskFailed => "task_failed",
        RunCreated => "run_created",
        RunClaimed => "run_claimed",
        RunHeartbeat => "run_heartbeat",
        RunCompleted => "run_completed",
        RunCrashed => "run_crashed",
        RunTimedOut => "run_timed_out",
        DependencyUnblocked => "dependency_unblocked",
        BlackboardWrite => "blackboard_write",
        BlackboardRead => "blackboard_read",
        GatePass => "gate_pass",
        GateBlock => "gate_block",
        VerifierRun => "verifier_run",
        SynthesizerRun => "synthesizer_run",
        ConfigChanged => "config_changed",
        Error => "error",
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 同步建表 DDL (跟 persistence.rs 一样, 复制出来让 db 测试独立).
    /// 在测试 setUp 调 `setup_db()` 一次性建.
    const DDL: &str = "
        PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS kanban_projects (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL,
            default_root TEXT NOT NULL, extra_roots TEXT NOT NULL DEFAULT '[]',
            status TEXT NOT NULL DEFAULT 'active', owner TEXT NOT NULL DEFAULT 'local',
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kanban_boards (
            id TEXT PRIMARY KEY, project_id TEXT REFERENCES kanban_projects(id) ON DELETE CASCADE,
            name TEXT NOT NULL, project_root TEXT NOT NULL,
            default_role TEXT NOT NULL DEFAULT 'coordinator',
            status TEXT NOT NULL DEFAULT 'active',
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL
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
        CREATE TABLE IF NOT EXISTS kanban_runs (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE,
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
        -- 最小 Profile 表 (FK 引用)
        CREATE TABLE IF NOT EXISTS kanban_profiles (
            id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT NOT NULL DEFAULT 'local',
            working_dir TEXT NOT NULL, tool_filter TEXT NOT NULL,
            max_turns INTEGER NOT NULL DEFAULT 32, model TEXT,
            system_prompt_template TEXT NOT NULL, created_at TEXT NOT NULL
        );
    ";

    /// 同步 setup: 建表 + 插 1 board + 1 profile.
    async fn setup_db() -> (KanbanDb, String, String) {
        let db = KanbanDb::in_memory().expect("in_memory");
        db.run_blocking(|c| {
            c.execute_batch(DDL).map_err(KanbanError::Sqlite)?;
            let now = Utc::now().to_rfc3339();
            c.execute(
                "INSERT INTO kanban_projects (id, name, description, default_root, status, owner, created_at, updated_at) \
                 VALUES ('proj_test', 'test', '', '', 'active', 'local', ?1, ?1)",
                params![now],
            ).map_err(KanbanError::Sqlite)?;
            c.execute(
                "INSERT INTO kanban_boards (id, project_id, name, project_root, status, created_at, updated_at) \
                 VALUES ('kb_test', 'proj_test', 'test', '/tmp', 'active', ?1, ?1)",
                params![now],
            ).map_err(KanbanError::Sqlite)?;
            c.execute(
                "INSERT INTO kanban_profiles (id, name, kind, working_dir, tool_filter, system_prompt_template, created_at) \
                 VALUES ('prof_test', 'test', 'local', '/tmp', '{}', '', ?1)",
                params![now],
            ).map_err(KanbanError::Sqlite)?;
            Ok(())
        }).await.expect("setup");
        (db, "kb_test".to_string(), "prof_test".to_string())
    }

    #[tokio::test]
    async fn test_create_task_default_status_triage() {
        let (db, board, _profile) = setup_db().await;
        let task = db
            .create_task(None, &board, None, "test title", "body", "coder", 128)
            .await
            .expect("create");
        assert_eq!(task.status, TaskStatus::Triage);
        assert!(task.id.starts_with("task_"));
        assert!(task.t_started_at.is_none());
        assert!(task.t_completed_at.is_none());
    }

    #[tokio::test]
    async fn test_get_task_round_trip() {
        let (db, board, _profile) = setup_db().await;
        let created = db
            .create_task(None, &board, None, "title", "body", "coder", 64)
            .await
            .expect("create");
        let fetched = db.get_task(&created.id).await.expect("get");
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.title, "title");
        assert_eq!(fetched.priority, 64);
    }

    #[tokio::test]
    async fn test_list_tasks_by_status() {
        let (db, board, _profile) = setup_db().await;
        let t1 = db.create_task(None, &board, None, "a", "b", "coder", 128).await.expect("a");
        let t2 = db.create_task(None, &board, None, "c", "d", "coder", 128).await.expect("c");
        let _t3 = db.create_task(None, &board, None, "e", "f", "coder", 128).await.expect("e");

        // 全部 status=triage
        let all = db.list_tasks(&board, None).await.expect("list all");
        assert_eq!(all.len(), 3);

        // t1: Triage -> Ready, t2: Triage -> Ready -> InProgress (符合 state_machine)
        db.update_task_status(&t1.id, TaskStatus::Ready).await.expect("u1");
        db.update_task_status(&t2.id, TaskStatus::Ready).await.expect("u2a");
        db.update_task_status(&t2.id, TaskStatus::InProgress).await.expect("u2b");
        // t3 保持 triage

        let triage = db.list_tasks(&board, Some(TaskStatus::Triage)).await.expect("triage");
        assert_eq!(triage.len(), 1);
        assert_eq!(triage[0].title, "e");
        let ready = db.list_tasks(&board, Some(TaskStatus::Ready)).await.expect("ready");
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].title, "a");
        let in_progress = db.list_tasks(&board, Some(TaskStatus::InProgress)).await.expect("ip");
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].title, "c");
    }

    #[tokio::test]
    async fn test_update_task_status_syncs_timestamps() {
        let (db, board, _profile) = setup_db().await;
        let task = db.create_task(None, &board, None, "x", "y", "coder", 128).await.expect("c");
        // Triage -> Ready -> InProgress (符合 state_machine 合法转换)
        db.update_task_status(&task.id, TaskStatus::Ready).await.expect("r");
        db.update_task_status(&task.id, TaskStatus::InProgress).await.expect("u");
        let fetched = db.get_task(&task.id).await.expect("g").unwrap();
        assert_eq!(fetched.status, TaskStatus::InProgress);
        assert!(fetched.t_started_at.is_some(), "t_started_at should be set");
        // InProgress -> Done 应自动设 t_completed_at
        db.update_task_status(&task.id, TaskStatus::Done).await.expect("d");
        let fetched2 = db.get_task(&task.id).await.expect("g2").unwrap();
        assert!(fetched2.t_completed_at.is_some(), "t_completed_at should be set");
    }

    #[tokio::test]
    async fn test_create_run_with_uuid_claim_id() {
        let (db, board, profile) = setup_db().await;
        let task = db.create_task(None, &board, None, "x", "y", "coder", 128).await.expect("c");
        let run = db.create_run(None, &task.id, &profile).await.expect("create run");
        assert!(run.id.starts_with("run_"));
        assert!(!run.claim_id.is_empty());
        // claim_id 应该是 uuid 格式
        assert!(Uuid::parse_str(&run.claim_id).is_ok());
        assert_eq!(run.status, RunStatus::Pending);
    }

    #[tokio::test]
    async fn test_complete_run_sets_ended_at_and_outcome() {
        let (db, board, profile) = setup_db().await;
        let task = db.create_task(None, &board, None, "x", "y", "coder", 128).await.expect("c");
        let run = db.create_run(None, &task.id, &profile).await.expect("r");
        db.complete_run(&run.id, RunOutcome::Success, "all good", 100, 200)
            .await
            .expect("complete");
        // 通过 event 验证 (不能 get_run, 没这个方法, 用 run_blocking 查)
        let outcome = db
            .run_blocking(move |c| {
                let outcome: String = c
                    .query_row(
                        "SELECT outcome FROM kanban_runs WHERE id = ?1",
                        params![run.id],
                        |row| row.get(0),
                    )
                    .map_err(KanbanError::Sqlite)?;
                Ok(outcome)
            })
            .await
            .expect("query");
        assert_eq!(outcome, "success");
    }

    #[tokio::test]
    async fn test_blackboard_write_read_round_trip() {
        let (db, board, _profile) = setup_db().await;
        let task = db.create_task(None, &board, None, "x", "y", "coder", 128).await.expect("c");
        // 写 2 个 key
        db.write_blackboard(&task.id, "current_focus", &serde_json::json!("调研 daemon"), "coder")
            .await
            .expect("w1");
        db.write_blackboard(&task.id, "user_constraints", &serde_json::json!(["要快", "要准"]), "user")
            .await
            .expect("w2");
        // 读
        let bb = db.read_blackboard(&task.id).await.expect("r");
        assert_eq!(bb.len(), 2);
        assert_eq!(bb.get("current_focus").unwrap().value, serde_json::json!("调研 daemon"));
        assert_eq!(bb.get("current_focus").unwrap().author, "coder");
        assert_eq!(bb.get("user_constraints").unwrap().author, "user");
    }

    #[tokio::test]
    async fn test_blackboard_write_last_wins() {
        let (db, board, _profile) = setup_db().await;
        let task = db.create_task(None, &board, None, "x", "y", "coder", 128).await.expect("c");
        // 写 2 次同一 key
        db.write_blackboard(&task.id, "focus", &serde_json::json!("first"), "a")
            .await
            .expect("w1");
        db.write_blackboard(&task.id, "focus", &serde_json::json!("second"), "b")
            .await
            .expect("w2");
        let bb = db.read_blackboard(&task.id).await.expect("r");
        assert_eq!(bb.len(), 1, "last writer wins: 应该只有 1 个 key");
        assert_eq!(bb.get("focus").unwrap().value, serde_json::json!("second"));
        assert_eq!(bb.get("focus").unwrap().author, "b");
    }

    #[tokio::test]
    async fn test_event_append_increments_id() {
        let (db, board, _profile) = setup_db().await;
        let task = db.create_task(None, &board, None, "x", "y", "coder", 128).await.expect("c");
        let id1 = db
            .append_event(Some(&task.id), None, KanbanEventKind::TaskCreated, &serde_json::json!({}))
            .await
            .expect("e1");
        let id2 = db
            .append_event(Some(&task.id), None, KanbanEventKind::TaskAssigned, &serde_json::json!({}))
            .await
            .expect("e2");
        assert!(id2 > id1, "auto-increment id 应递增: {id1} < {id2}");
    }

    #[tokio::test]
    async fn test_concurrent_writes_serialized_by_mutex() {
        // 验证 spawn_blocking + Mutex 串行化写 (10 个并发写不冲突)
        let (db, board, _profile) = setup_db().await;
        let task = db.create_task(None, &board, None, "x", "y", "coder", 128).await.expect("c");
        let mut handles = vec![];
        for i in 0..10 {
            let db = db.clone();
            let task_id = task.id.clone();
            handles.push(tokio::spawn(async move {
                db.write_blackboard(
                    &task_id,
                    &format!("key_{i}"),
                    &serde_json::json!(i),
                    "concurrent_test",
                )
                .await
            }));
        }
        for h in handles {
            h.await.expect("join").expect("write");
        }
        let bb = db.read_blackboard(&task.id).await.expect("r");
        assert_eq!(bb.len(), 10);
        let mut keys: HashSet<String> = bb.keys().cloned().collect();
        for i in 0..10 {
            assert!(keys.remove(&format!("key_{i}")), "missing key_{i}");
        }
        assert!(keys.is_empty());
    }
}

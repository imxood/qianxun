//! Kanban tools (v6 §7.2, MVP-2 plan 5 精简版)
//!
//! 12 个 kanban_* 工具核心 4 个先落地 (Worker/Orchestrator scope 护栏):
//! - `kanban_create` (Orchestrator only) - 创建 task
//! - `kanban_complete` (Worker only) - 标记 task done + 写 outcome
//! - `kanban_heartbeat` (Worker only) - 更新 heartbeat (60s 限频)
//! - `kanban_write_blackboard` (公共) - 写黑板 (last writer wins)
//!
//! 剩余 8 个工具 (link/assign/list/unblock/block/comment/read_blackboard/decompose)
//! 留 v2 扩展, 全部按相同模式 (`#[async_trait] impl AgentTool`) 加.

use async_trait::async_trait;
use rusqlite::params;
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use tokio::sync::RwLock;

use super::{AgentTool, ToolError, ToolOutput};
use crate::kanban::db::KanbanDb;
use crate::kanban::types::{KanbanScope, RunOutcome, TaskStatus, WorkerScope};
use crate::kanban::KanbanError;

// =============================================================================
// KanbanToolContext (共享 scope + db + heartbeat 限频状态)
// =============================================================================

/// Kanban tool 共享上下文: 持有 KanbanDb + 当前 scope + heartbeat 限频 last 时刻.
#[derive(Clone)]
pub struct KanbanToolContext {
    pub db: KanbanDb,
    pub scope: Arc<RwLock<KanbanScope>>,
    /// 每 task_id 上次 heartbeat 时刻 (60s 限频, MVP-2 plan 5 决策)
    pub heartbeat_last: Arc<Mutex<std::collections::HashMap<String, Instant>>>,
}

impl KanbanToolContext {
    pub fn new(db: KanbanDb, scope: KanbanScope) -> Self {
        Self {
            db,
            scope: Arc::new(RwLock::new(scope)),
            heartbeat_last: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// 替换 scope (dispatcher 派工时调用)
    pub async fn set_scope(&self, scope: KanbanScope) {
        let mut s = self.scope.write().await;
        *s = scope;
    }

    /// 读 scope (snapshot clone)
    pub async fn snapshot_scope(&self) -> KanbanScope {
        self.scope.read().await.clone()
    }

    /// 校验 scope 允许当前 role 调此工具
    pub async fn check_scope(&self, required: WorkerScope, tool_name: &str) -> Result<(), ToolError> {
        let scope = self.scope.read().await;
        if scope.role != required {
            return Err(ToolError::NotAllowedInCurrentMode {
                tool: tool_name.to_string(),
                mode: format!("{:?}", scope.role),
            });
        }
        Ok(())
    }

    /// KanbanError -> ToolError 转换
    pub fn map_err(e: KanbanError) -> ToolError {
        ToolError::ExecutionFailed(e.to_string())
    }
}

// =============================================================================
// KanbanCreate (Orchestrator only)
// =============================================================================

/// 创建 task. 调 db.create_task + 写 TaskCreated event.
pub struct KanbanCreateTool {
    pub ctx: KanbanToolContext,
}

#[async_trait]
impl AgentTool for KanbanCreateTool {
    fn name(&self) -> &str {
        "kanban_create"
    }
    fn description(&self) -> &str {
        "Orchestrator 创建 Kanban 任务 (Triage 状态). 入参: board_id, title, body, assignee_role, parent_id (可选), priority (可选)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "board_id": {"type": "string", "description": "目标 board id"},
                "title": {"type": "string", "description": "任务标题 <=120 chars"},
                "body": {"type": "string", "description": "任务描述 (Markdown)"},
                "assignee_role": {"type": "string", "description": "角色名 (techlead/coder/verifier/researcher)"},
                "parent_id": {"type": "string", "description": "父 task id (可选, root 任务无父)"},
                "priority": {"type": "integer", "description": "0=low, 128=normal, 255=urgent", "default": 128}
            },
            "required": ["board_id", "title", "body", "assignee_role"]
        })
    }
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.ctx.check_scope(WorkerScope::Orchestrator, "kanban_create").await?;
        let board_id = args["board_id"].as_str().ok_or_else(|| ToolError::InvalidArguments("board_id required".into()))?;
        let title = args["title"].as_str().ok_or_else(|| ToolError::InvalidArguments("title required".into()))?;
        let body = args["body"].as_str().ok_or_else(|| ToolError::InvalidArguments("body required".into()))?;
        let assignee_role = args["assignee_role"].as_str().ok_or_else(|| ToolError::InvalidArguments("assignee_role required".into()))?;
        let parent_id = args["parent_id"].as_str();
        let priority = args["priority"].as_u64().unwrap_or(128) as u8;

        let task = self.ctx.db.create_task(None, board_id, parent_id, title, body, assignee_role, priority)
            .await
            .map_err(KanbanToolContext::map_err)?;
        // 写 TaskCreated event (best-effort, 失败不阻塞)
        let _ = self.ctx.db.append_event(
            Some(&task.id), None, crate::kanban::types::KanbanEventKind::TaskCreated,
            &json!({"title": title, "assignee_role": assignee_role}),
        ).await;

        Ok(ToolOutput {
            content: json!({"task_id": task.id, "status": "triage"}).to_string(),
            is_error: false,
        })
    }
}

// =============================================================================
// KanbanComplete (Worker only)
// =============================================================================

/// 标记 task done + 写 outcome.
pub struct KanbanCompleteTool {
    pub ctx: KanbanToolContext,
}

#[async_trait]
impl AgentTool for KanbanCompleteTool {
    fn name(&self) -> &str {
        "kanban_complete"
    }
    fn description(&self) -> &str {
        "Worker 标记当前 task 完成. 入参: task_id, summary, outcome (success/partial_success/failure), token_input, token_output."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "summary": {"type": "string", "description": "LLM 总结"},
                "outcome": {"type": "string", "enum": ["success", "partial_success", "failure"], "default": "success"},
                "token_input": {"type": "integer", "default": 0},
                "token_output": {"type": "integer", "default": 0}
            },
            "required": ["task_id", "summary"]
        })
    }
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.ctx.check_scope(WorkerScope::Worker, "kanban_complete").await?;
        let scope = self.ctx.snapshot_scope().await;
        let task_id = args["task_id"].as_str().ok_or_else(|| ToolError::InvalidArguments("task_id required".into()))?;
        let summary = args["summary"].as_str().ok_or_else(|| ToolError::InvalidArguments("summary required".into()))?;
        let outcome_str = args["outcome"].as_str().unwrap_or("success");
        let outcome = match outcome_str {
            "success" => RunOutcome::Success,
            "partial_success" => RunOutcome::PartialSuccess,
            "failure" => RunOutcome::Failure,
            _ => return Err(ToolError::InvalidArguments(format!("unknown outcome: {outcome_str}"))),
        };
        let token_input = args["token_input"].as_u64().unwrap_or(0);
        let token_output = args["token_output"].as_u64().unwrap_or(0);

        // 防 prompt injection: Worker 只能完成自己 scope 里的 task
        if let Some(assigned) = &scope.assigned_task_id {
            if assigned != task_id {
                return Err(ToolError::ExecutionFailed(
                    format!("scope violation: worker assigned to {assigned}, cannot complete {task_id}")
                ));
            }
        }

        // 1. 完成最新 run (找 task 的最新 run, 写 outcome)
        let task_id_owned = task_id.to_string();
        let latest_run_id = self.ctx.db.run_blocking(move |c| -> Result<Option<String>, KanbanError> {
            let mut stmt = c.prepare(
                "SELECT id FROM kanban_runs WHERE task_id = ?1 ORDER BY started_at DESC LIMIT 1"
            )?;
            Ok(stmt.query_row(params![task_id_owned], |row| row.get(0)).ok())
        }).await.map_err(KanbanToolContext::map_err)?;

        if let Some(run_id) = latest_run_id {
            self.ctx.db.complete_run(&run_id, outcome, summary, token_input, token_output)
                .await
                .map_err(KanbanToolContext::map_err)?;
        }

        // 2. 更新 task: InProgress -> Done (state_machine 校验)
        self.ctx.db.update_task_status(task_id, TaskStatus::Done).await
            .map_err(KanbanToolContext::map_err)?;

        // 3. 写 TaskCompleted event
        let _ = self.ctx.db.append_event(
            Some(task_id), None, crate::kanban::types::KanbanEventKind::TaskCompleted,
            &json!({"outcome": outcome_str, "summary": summary}),
        ).await;

        Ok(ToolOutput {
            content: json!({"task_id": task_id, "status": "done", "outcome": outcome_str}).to_string(),
            is_error: false,
        })
    }
}

// =============================================================================
// KanbanHeartbeat (Worker only, 60s 限频)
// =============================================================================

/// Worker 写心跳 (60s 限频).
pub struct KanbanHeartbeatTool {
    pub ctx: KanbanToolContext,
}

#[async_trait]
impl AgentTool for KanbanHeartbeatTool {
    fn name(&self) -> &str {
        "kanban_heartbeat"
    }
    fn description(&self) -> &str {
        "Worker 更新 task 心跳 (60s 限频, 同 task_id 在 60s 内重复调用 skip). 防 'worker 死了但 task 卡 in_progress'."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"}
            },
            "required": ["task_id"]
        })
    }
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.ctx.check_scope(WorkerScope::Worker, "kanban_heartbeat").await?;
        let task_id = args["task_id"].as_str().ok_or_else(|| ToolError::InvalidArguments("task_id required".into()))?.to_string();

        // 60s 限频
        let now = Instant::now();
        {
            let mut last = self.ctx.heartbeat_last.lock().unwrap();
            if let Some(prev) = last.get(&task_id) {
                if now.duration_since(*prev).as_secs() < 60 {
                    return Ok(ToolOutput {
                        content: json!({"throttled": true, "task_id": task_id, "skip": true}).to_string(),
                        is_error: false,
                    });
                }
            }
            last.insert(task_id.clone(), now);
        }

        self.ctx.db.update_heartbeat(&task_id).await.map_err(KanbanToolContext::map_err)?;
        let _ = self.ctx.db.append_event(
            Some(&task_id), None, crate::kanban::types::KanbanEventKind::RunHeartbeat,
            &json!({}),
        ).await;

        Ok(ToolOutput {
            content: json!({"task_id": task_id, "throttled": false, "updated": true}).to_string(),
            is_error: false,
        })
    }
}

// =============================================================================
// KanbanWriteBlackboard (公共, last writer wins)
// =============================================================================

/// 写黑板 (Worker + Orchestrator 都能调).
pub struct KanbanWriteBlackboardTool {
    pub ctx: KanbanToolContext,
}

#[async_trait]
impl AgentTool for KanbanWriteBlackboardTool {
    fn name(&self) -> &str {
        "kanban_write_blackboard"
    }
    fn description(&self) -> &str {
        "写黑板条目 (last writer wins). Worker 写自己 task 的中间结果 / context, Orchestrator 写共享 context. \
         入参: task_id, key, value (任意 JSON), author."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "key": {"type": "string", "description": "e.g. 'current_focus' / 'user_constraints' / 'intermediate_result'"},
                "value": {"description": "任意 JSON 值"},
                "author": {"type": "string", "description": "profile_id | 'user' | 'system'"}
            },
            "required": ["task_id", "key", "value", "author"]
        })
    }
    async fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let task_id = args["task_id"].as_str().ok_or_else(|| ToolError::InvalidArguments("task_id required".into()))?;
        let key = args["key"].as_str().ok_or_else(|| ToolError::InvalidArguments("key required".into()))?;
        let value = args.get("value").cloned().unwrap_or(Value::Null);
        let author = args["author"].as_str().unwrap_or("system");

        self.ctx.db.write_blackboard(task_id, key, &value, author)
            .await
            .map_err(KanbanToolContext::map_err)?;
        let _ = self.ctx.db.append_event(
            Some(task_id), None, crate::kanban::types::KanbanEventKind::BlackboardWrite,
            &json!({"key": key, "value_preview": value.to_string().chars().take(200).collect::<String>()}),
        ).await;

        Ok(ToolOutput {
            content: json!({"task_id": task_id, "key": key, "written": true}).to_string(),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kanban::db::KanbanDb;

    async fn setup() -> (KanbanDb, String) {
        let db = KanbanDb::in_memory().expect("in_memory");
        db.run_blocking(|c| {
            c.execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE kanban_projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL, default_root TEXT NOT NULL, extra_roots TEXT NOT NULL DEFAULT '[]', status TEXT NOT NULL DEFAULT 'active', owner TEXT NOT NULL DEFAULT 'local', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE kanban_boards (id TEXT PRIMARY KEY, project_id TEXT REFERENCES kanban_projects(id) ON DELETE CASCADE, name TEXT NOT NULL, project_root TEXT NOT NULL, default_role TEXT NOT NULL DEFAULT 'coordinator', status TEXT NOT NULL DEFAULT 'active', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE kanban_tasks (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES kanban_boards(id) ON DELETE CASCADE, parent_id TEXT REFERENCES kanban_tasks(id) ON DELETE CASCADE, title TEXT NOT NULL, body TEXT NOT NULL, assignee_role TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'triage', priority INTEGER NOT NULL DEFAULT 128, deadline TEXT, metadata TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL, t_started_at TEXT, t_completed_at TEXT, last_heartbeat_at TEXT);
                 CREATE TABLE kanban_runs (id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE, profile_id TEXT NOT NULL REFERENCES kanban_profiles(id), status TEXT NOT NULL DEFAULT 'pending', claim_id TEXT NOT NULL, r_heartbeat_at TEXT, started_at TEXT NOT NULL, ended_at TEXT, outcome TEXT NOT NULL DEFAULT 'success', summary TEXT NOT NULL DEFAULT '', error TEXT, token_input INTEGER NOT NULL DEFAULT 0, token_output INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE kanban_profiles (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT NOT NULL DEFAULT 'local', working_dir TEXT NOT NULL, tool_filter TEXT NOT NULL, max_turns INTEGER NOT NULL DEFAULT 32, model TEXT, system_prompt_template TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE kanban_blackboard (task_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE, key TEXT NOT NULL, value TEXT NOT NULL, author TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (task_id, key));
                 CREATE TABLE kanban_events (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT, run_id TEXT, kind TEXT NOT NULL, payload TEXT NOT NULL, created_at TEXT NOT NULL);
                 INSERT INTO kanban_projects (id, name, description, default_root, status, owner, created_at, updated_at) VALUES ('p1', 'p', '', '', 'active', 'local', '2026-01-01', '2026-01-01');
                 INSERT INTO kanban_boards (id, project_id, name, project_root, status, created_at, updated_at) VALUES ('b1', 'p1', 'b', '/tmp', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO kanban_profiles (id, name, kind, working_dir, tool_filter, system_prompt_template, created_at) VALUES ('prof_coder', 'coder', 'local', '/tmp', '{}', '', '2026-01-01');"
            ).map_err(KanbanError::Sqlite)?;
            Ok(())
        }).await.expect("setup");
        (db, "b1".to_string())
    }

    #[tokio::test]
    async fn test_kanban_create_orchestrator_only() {
        let (db, board) = setup().await;
        let ctx_orch = KanbanToolContext::new(db.clone(), KanbanScope {
            board_id: board.clone(),
            role: WorkerScope::Orchestrator,
            assigned_task_id: None,
            profiles_available: vec!["coder".into()],
        });
        let tool = KanbanCreateTool { ctx: ctx_orch };
        let args = json!({"board_id": board, "title": "test", "body": "b", "assignee_role": "coder"});
        let out = tool.execute(args).await.expect("ok");
        assert!(!out.is_error);
        assert!(out.content.contains("task_id"));
    }

    #[tokio::test]
    async fn test_kanban_create_worker_blocked() {
        let (db, board) = setup().await;
        let ctx_worker = KanbanToolContext::new(db.clone(), KanbanScope {
            board_id: board.clone(),
            role: WorkerScope::Worker,
            assigned_task_id: Some("task_x".into()),
            profiles_available: vec![],
        });
        let tool = KanbanCreateTool { ctx: ctx_worker };
        let args = json!({"board_id": board, "title": "t", "body": "b", "assignee_role": "coder"});
        let r = tool.execute(args).await;
        assert!(matches!(r, Err(ToolError::NotAllowedInCurrentMode { .. })));
    }

    #[tokio::test]
    async fn test_kanban_complete_worker_blocked_by_orchestrator() {
        let (db, board) = setup().await;
        let ctx_orch = KanbanToolContext::new(db.clone(), KanbanScope {
            board_id: board.clone(),
            role: WorkerScope::Orchestrator,
            assigned_task_id: None,
            profiles_available: vec![],
        });
        let tool = KanbanCompleteTool { ctx: ctx_orch };
        let r = tool.execute(json!({"task_id": "t", "summary": "s"})).await;
        assert!(matches!(r, Err(ToolError::NotAllowedInCurrentMode { .. })));
    }

    #[tokio::test]
    async fn test_kanban_heartbeat_60s_throttle() {
        let (db, _board) = setup().await;
        let ctx_worker = KanbanToolContext::new(db.clone(), KanbanScope {
            board_id: "b1".into(),
            role: WorkerScope::Worker,
            assigned_task_id: Some("task_x".into()),
            profiles_available: vec![],
        });
        let tool = KanbanHeartbeatTool { ctx: ctx_worker };
        // 第一次: 写入
        let r1 = tool.execute(json!({"task_id": "task_x"})).await.expect("ok");
        assert!(r1.content.contains("\"throttled\":false"));
        // 第二次: throttled (60s 内)
        let r2 = tool.execute(json!({"task_id": "task_x"})).await.expect("ok");
        assert!(r2.content.contains("\"throttled\":true"));
    }

    #[tokio::test]
    async fn test_kanban_write_blackboard_any_scope() {
        let (db, board) = setup().await;
        // Worker scope 也能调 (公共工具), 但需要先建 task (FK 约束)
        db.create_task(Some("task_x"), &board, None, "x", "y", "coder", 128).await.expect("c");
        let ctx = KanbanToolContext::new(db.clone(), KanbanScope {
            board_id: board,
            role: WorkerScope::Worker,
            assigned_task_id: Some("task_x".into()),
            profiles_available: vec![],
        });
        let tool = KanbanWriteBlackboardTool { ctx };
        let r = tool.execute(json!({"task_id": "task_x", "key": "focus", "value": "调研 daemon", "author": "coder"}))
            .await.expect("ok");
        assert!(r.content.contains("\"written\":true"));
    }

    #[tokio::test]
    async fn test_kanban_complete_scope_violation_other_task() {
        // Worker assigned to task_x, but tries to complete task_y -> blocked
        let (db, board) = setup().await;
        let ctx = KanbanToolContext::new(db.clone(), KanbanScope {
            board_id: board,
            role: WorkerScope::Worker,
            assigned_task_id: Some("task_x".into()),
            profiles_available: vec![],
        });
        let tool = KanbanCompleteTool { ctx };
        let r = tool.execute(json!({"task_id": "task_y", "summary": "wrong"})).await;
        assert!(matches!(r, Err(ToolError::ExecutionFailed(_))));
    }
}

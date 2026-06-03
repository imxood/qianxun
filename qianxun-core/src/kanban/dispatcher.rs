//! KanbanDispatcher (v6 §7.4 骨架, MVP-2 plan 4)
//!
//! 拾取 ready task + 找 idle profile + 新建 run row + 更新 task 状态.
//! MVP-2 阶段不真 spawn session (留 MVP-3 接 AgentLoopHost), 只返
//! `DispatchedRun` 占位.

use std::sync::Arc;
use std::time::Duration;

use rusqlite::OptionalExtension;
use tokio::sync::Mutex;

use super::db::KanbanDb;
use super::error::KanbanError;
use super::team::TeamConfig;
use super::team::TeamRegistry;
use super::types::TaskStatus;

/// Dispatcher 一次拾取的结果.
#[derive(Debug, Clone)]
pub struct DispatchedRun {
    pub task_id: String,
    pub run_id: String,
    pub profile_name: String,
}

/// KanbanDispatcher — 中央调度器 (v6 §7.4).
///
/// MVP-2 阶段: 拾取 ready task + 找 idle profile + 新建 run row + 更新
/// task 状态. 不真 spawn session (留 MVP-3 接 AgentLoopHost).
pub struct KanbanDispatcher {
    pub db: KanbanDb,
    pub team_registry: TeamRegistry,
    pub config: Arc<Mutex<TeamConfig>>,
    /// 标记 dispatcher 是否在跑 (防止 run_forever 重复)
    running: Arc<Mutex<bool>>,
}

impl KanbanDispatcher {
    /// 创建 dispatcher (MVP-2 阶段 config 默认值, MVP-3 接配置加载).
    pub fn new(db: KanbanDb, team_registry: TeamRegistry) -> Self {
        Self {
            db,
            team_registry,
            config: Arc::new(Mutex::new(TeamConfig::default())),
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// 拾取一个 ready task, 找 idle profile, 新建 run row.
    /// 返 None 当: 没 ready task / 所有 profile 都在忙 / orchestrator 关闭.
    pub async fn dispatch_once(&self) -> Result<Option<DispatchedRun>, KanbanError> {
        // 0. 紧急刹车
        if !self.config.lock().await.orchestrator_enabled {
            return Err(KanbanError::OrchestratorDisabled);
        }
        // 1. 找 ready task (status=ready, 最旧优先)
        let task = self.find_next_ready_task().await?;
        let Some(task) = task else {
            return Ok(None);
        };
        // 2. 找 idle profile
        let max_concurrent = self.config.lock().await.max_concurrent_children;
        let profile = self
            .team_registry
            .find_idle_profile_for_role(&task.assignee_role, max_concurrent)
            .await;
        let Some(profile) = profile else {
            // 排队 (v6 §6.4 模式 6)
            return Ok(None);
        };
        // 3. 新建 run row
        let run = self
            .db
            .create_run(None, &task.id, &profile.id)
            .await?;
        // 4. 更新 task: ready -> in_progress
        self.db
            .update_task_status(&task.id, TaskStatus::InProgress)
            .await?;
        // 5. 标记 profile 活跃
        self.team_registry.mark_run_started(&profile.id).await;
        // 6. 返 DispatchedRun
        Ok(Some(DispatchedRun {
            task_id: task.id,
            run_id: run.id,
            profile_name: profile.name,
        }))
    }

    /// 找下一个 ready task (status=ready, 最旧优先). MVP-2 用最简 SQL, 不上索引.
    async fn find_next_ready_task(
        &self,
    ) -> Result<Option<super::types::Task>, KanbanError> {
        self.db
            .run_blocking(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, board_id, parent_id, title, body, assignee_role, status, priority, \
                     deadline, metadata, created_at, t_started_at, t_completed_at, last_heartbeat_at \
                     FROM kanban_tasks WHERE status = 'ready' ORDER BY created_at ASC LIMIT 1",
                )?;
                let task = stmt
                    .query_row([], super::db::row_to_task)
                    .optional()
                    .map_err(KanbanError::Sqlite)?;
                Ok(task)
            })
            .await
    }

    /// 后台循环: 每 N 秒调 dispatch_once.
    /// 跑前检查 running flag, 防止重复.
    /// MVP-3 阶段会接入 daemon/mod.rs::run() 启动一次.
    pub async fn run_forever(self: Arc<Self>) {
        {
            let mut running = self.running.lock().await;
            if *running {
                tracing::warn!("[kanban] dispatcher already running, skipping run_forever");
                return;
            }
            *running = true;
        }
        let interval_secs = 2;
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        tracing::info!("[kanban] dispatcher run_forever started (interval={}s)", interval_secs);
        loop {
            interval.tick().await;
            match self.dispatch_once().await {
                Ok(Some(r)) => {
                    tracing::info!(
                        "[kanban] dispatched task={} run={} profile={}",
                        r.task_id, r.run_id, r.profile_name
                    );
                }
                Ok(None) => {
                    // 没 ready task 或 profile 满, 静默
                }
                Err(e) => {
                    tracing::warn!("[kanban] dispatch error: {e}");
                }
            }
        }
    }

    /// 测试用: 检查 dispatcher 是否在跑
    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    /// 测试用: 临时禁用 orchestrator (验证紧急刹车).
    /// 改用 Mutex 锁保护 config, async 测试也能调.
    pub async fn disable_for_test(&self) {
        let mut cfg = self.config.lock().await;
        cfg.orchestrator_enabled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    async fn setup() -> (Arc<KanbanDispatcher>, String) {
        let db = KanbanDb::in_memory().expect("in_memory");
        // 建表 + 插 1 board + 1 profile
        db.run_blocking(|c| {
            c.execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE kanban_projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL, default_root TEXT NOT NULL, extra_roots TEXT NOT NULL DEFAULT '[]', status TEXT NOT NULL DEFAULT 'active', owner TEXT NOT NULL DEFAULT 'local', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE kanban_boards (id TEXT PRIMARY KEY, project_id TEXT REFERENCES kanban_projects(id) ON DELETE CASCADE, name TEXT NOT NULL, project_root TEXT NOT NULL, default_role TEXT NOT NULL DEFAULT 'coordinator', status TEXT NOT NULL DEFAULT 'active', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE kanban_profiles (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT NOT NULL DEFAULT 'local', working_dir TEXT NOT NULL, tool_filter TEXT NOT NULL, max_turns INTEGER NOT NULL DEFAULT 32, model TEXT, system_prompt_template TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE kanban_tasks (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES kanban_boards(id) ON DELETE CASCADE, parent_id TEXT REFERENCES kanban_tasks(id) ON DELETE CASCADE, title TEXT NOT NULL, body TEXT NOT NULL, assignee_role TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'triage', priority INTEGER NOT NULL DEFAULT 128, deadline TEXT, metadata TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL, t_started_at TEXT, t_completed_at TEXT, last_heartbeat_at TEXT);
                 CREATE TABLE kanban_runs (id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES kanban_tasks(id) ON DELETE CASCADE, profile_id TEXT NOT NULL REFERENCES kanban_profiles(id), status TEXT NOT NULL DEFAULT 'pending', claim_id TEXT NOT NULL, r_heartbeat_at TEXT, started_at TEXT NOT NULL, ended_at TEXT, outcome TEXT NOT NULL DEFAULT 'success', summary TEXT NOT NULL DEFAULT '', error TEXT, token_input INTEGER NOT NULL DEFAULT 0, token_output INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO kanban_projects (id, name, description, default_root, status, owner, created_at, updated_at) VALUES ('p1', 'p', '', '', 'active', 'local', '2026-01-01', '2026-01-01');
                 INSERT INTO kanban_boards (id, project_id, name, project_root, status, created_at, updated_at) VALUES ('b1', 'p1', 'b', '/tmp', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO kanban_profiles (id, name, kind, working_dir, tool_filter, system_prompt_template, created_at) VALUES ('prof_coder', 'coder', 'local', '/tmp', '{}', '', '2026-01-01');"
            ).map_err(KanbanError::Sqlite)?;
            Ok(())
        }).await.expect("setup");
        let reg = TeamRegistry::load_default();
        let dispatcher = Arc::new(KanbanDispatcher::new(db, reg));
        (dispatcher, "b1".to_string())
    }

    #[tokio::test]
    async fn test_dispatcher_no_ready_task_returns_none() {
        let (d, _board) = setup().await;
        let r = d.dispatch_once().await.expect("dispatch");
        assert!(r.is_none(), "no ready task should return None");
    }

    #[tokio::test]
    async fn test_dispatcher_orchestrator_disabled_errors() {
        let (d, _board) = setup().await;
        d.disable_for_test().await;
        let r = d.dispatch_once().await;
        assert!(matches!(r, Err(KanbanError::OrchestratorDisabled)));
    }

    #[tokio::test]
    async fn test_dispatcher_picks_ready_task_and_creates_run() {
        let (d, board) = setup().await;
        // 创建 1 个 task, status=triage, 通过 db.update_task_status 改成 ready
        // (Triage -> Ready 合法)
        let task = d
            .db
            .create_task(None, &board, None, "do something", "body", "coder", 128)
            .await
            .expect("create");
        d.db
            .update_task_status(&task.id, TaskStatus::Ready)
            .await
            .expect("ready");
        // dispatch
        let r = d.dispatch_once().await.expect("dispatch").expect("some");
        assert_eq!(r.task_id, task.id);
        assert!(r.run_id.starts_with("run_"));
        assert_eq!(r.profile_name, "coder");
        // task status 应变 in_progress
        let fetched = d.db.get_task(&task.id).await.expect("get").unwrap();
        assert_eq!(fetched.status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn test_dispatcher_no_idle_profile_returns_none() {
        let (d, board) = setup().await;
        // 创建 1 个 ready task
        let task = d
            .db
            .create_task(None, &board, None, "x", "y", "coder", 128)
            .await
            .expect("c");
        d.db
            .update_task_status(&task.id, TaskStatus::Ready)
            .await
            .expect("r");
        // 手动标记 coder profile 已经满
        let max_concurrent = d.config.lock().await.max_concurrent_children;
        for _ in 0..(max_concurrent + 1) {
            d.team_registry.mark_run_started("prof_coder").await;
        }
        // dispatch 应 None (没有 idle profile)
        let r = d.dispatch_once().await.expect("dispatch");
        assert!(r.is_none(), "should return None when no idle profile");
        let _ = HashSet::<String>::new(); // suppress unused import
    }
}

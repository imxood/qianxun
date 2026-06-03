//! Kanban Host (v6 §14.1 MVP-3 plan 0)
//!
//! daemon 端 Kanban 中央集成: KanbanDb + Dispatcher 后台 task +
//! 5 个 Kanban SSE 事件 emit 通道. 真 spawn_session 留 MVP-3 plan 1
//! (跟 AgentLoopHost 集成).

use std::sync::Arc;

use qianxun_core::kanban::{
    db::KanbanDb, dispatcher::KanbanDispatcher, team::TeamRegistry, DispatchedRun,
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

/// 5 个 Kanban SSE 事件 (扩 daemon/sse.rs 12 → 17, v6 §8.4)
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KanbanSseEvent {
    /// 任务分配 (前端可以"切到 worker 视图")
    KanbanTaskAssigned {
        task_id: String,
        run_id: String,
        profile_name: String,
        title: String,
    },
    /// 任务进度 (worker 调 kanban_comment / kanban_write_blackboard 时发)
    KanbanTaskProgress {
        task_id: String,
        run_id: String,
        event_kind: String,
        preview: String,
    },
    /// 任务完成 (worker 调 kanban_complete 时发)
    KanbanTaskCompleted {
        task_id: String,
        run_id: String,
        outcome: String,
        summary: String,
        token_input: u64,
        token_output: u64,
        elapsed_ms: u64,
    },
    /// 派生子任务 (techlead 调 kanban_create 时发)
    KanbanTaskSpawned {
        parent_task_id: Option<String>,
        child_task_id: String,
        title: String,
        assignee_role: String,
    },
    /// 黑板变更 (techlead 角色实时观察)
    KanbanBlackboardUpdate {
        task_id: String,
        key: String,
        value_preview: String,
    },
}

impl KanbanSseEvent {
    /// 序列化为 SSE event data (JSON 字符串)
    pub fn to_sse_data(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// event type name (跟 daemon/sse.rs SseEvent::type_name() 一致)
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::KanbanTaskAssigned { .. } => "kanban_task_assigned",
            Self::KanbanTaskProgress { .. } => "kanban_task_progress",
            Self::KanbanTaskCompleted { .. } => "kanban_task_completed",
            Self::KanbanTaskSpawned { .. } => "kanban_task_spawned",
            Self::KanbanBlackboardUpdate { .. } => "kanban_blackboard_update",
        }
    }
}

/// KanbanHost — daemon 端 Kanban 中央集成 (v6 §14.1 MVP-3).
///
/// 持有:
/// - KanbanDb (跟 daemon.db 共享, MVP-2 plan 1 已建)
/// - TeamRegistry (4 默认 role, MVP-2 plan 4 已建)
/// - KanbanDispatcher (后台 task, run_forever 循环)
/// - SSE 事件 broadcast channel (前端订阅)
pub struct KanbanHost {
    pub db: KanbanDb,
    pub team_registry: TeamRegistry,
    pub dispatcher: Arc<KanbanDispatcher>,
    /// 5 个 Kanban SSE 事件 broadcast
    pub sse_tx: broadcast::Sender<KanbanSseEvent>,
}

impl KanbanHost {
    /// 创建 host (不启动 dispatcher, 用 `start()` 启动后台 task)
    pub fn new(db: KanbanDb, team_registry: TeamRegistry) -> Self {
        // 幂等 init 8 张 kanban_* 表 (独立 in-memory / db 文件场景用).
        // daemon 启动时 create_tables 已经建过, 这次再跑 0 副作用.
        if let Err(e) = db.init_schema() {
            tracing::warn!("[kanban_host] init_schema 失败 (继续运行): {e}");
        }
        let dispatcher = Arc::new(KanbanDispatcher::new(db.clone(), team_registry.clone()));
        let (sse_tx, _) = broadcast::channel(256);
        Self {
            db,
            team_registry,
            dispatcher,
            sse_tx,
        }
    }

    /// 启动 dispatcher 后台 task (每 2 秒调 dispatch_once, 拾到 ready task 后
    /// 调 `agent_host.create_session_for_kanban_task` 真 spawn session, v2 接
    /// processing_loop).
    pub fn start(self: Arc<Self>, agent_host: std::sync::Arc<crate::daemon::agent_host::AgentLoopHost>) {
        let me = self.clone();
        let agent_host = agent_host.clone();
        tokio::spawn(async move {
            tracing::info!("[kanban_host] dispatcher run_forever starting");
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                interval.tick().await;
                match me.dispatcher.dispatch_once().await {
                    Ok(Some(r)) => {
                        tracing::info!(
                            "[kanban_host] dispatched task={} run={} profile={}",
                            r.task_id, r.run_id, r.profile_name
                        );
                        // emit KanbanTaskAssigned SSE
                        let _ = me.sse_tx.send(KanbanSseEvent::KanbanTaskAssigned {
                            task_id: r.task_id.clone(),
                            run_id: r.run_id.clone(),
                            profile_name: r.profile_name.clone(),
                            title: String::new(),
                        });
                        // 真 spawn session (2026-06-04 阶段 4)
                        // 简化版: 调 create_session + 持久化, 跑 LLM 留 v2
                        let db = me.db.clone();
                        let agent_host2 = agent_host.clone();
                        let task_id = r.task_id.clone();
                        let run_id = r.run_id.clone();
                        let _ = db
                            .run_blocking(move |c: &rusqlite::Connection| -> Result<_, qianxun_core::kanban::KanbanError> {
                                // 查 task + run
                                let task: Option<qianxun_core::kanban::types::Task> = c
                                    .query_row(
                                        "SELECT id, board_id, parent_id, title, body, assignee_role, status, priority, deadline, metadata, created_at, t_started_at, t_completed_at, last_heartbeat_at \
                                         FROM kanban_tasks WHERE id = ?1",
                                        rusqlite::params![task_id],
                                        |row| {
                                            use qianxun_core::kanban::db::row_to_task;
                                            row_to_task(row)
                                        },
                                    )
                                    .ok();
                                let run: Option<qianxun_core::kanban::types::AgentRun> = c
                                    .query_row(
                                        "SELECT id, task_id, profile_id, status, claim_id, r_heartbeat_at, started_at, ended_at, outcome, summary, error, token_input, token_output \
                                         FROM kanban_runs WHERE id = ?1",
                                        rusqlite::params![run_id],
                                        |row| {
                                            use qianxun_core::kanban::db::row_to_run;
                                            row_to_run(row)
                                        },
                                    )
                                    .ok();
                                if let (Some(task), Some(run)) = (task, run) {
                                    match agent_host2.create_session_for_kanban_task(&task, &run) {
                                        Ok(_rt) => {}
                                        Err(e) => tracing::error!("[kanban_host] spawn failed: {e}"),
                                    }
                                }
                                Ok(())
                            })
                            .await;
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("[kanban_host] dispatch error: {e}"),
                }
            }
        });
    }

    /// 订阅 SSE 事件 (前端用)
    pub fn subscribe(&self) -> broadcast::Receiver<KanbanSseEvent> {
        self.sse_tx.subscribe()
    }

    /// 触发 dispatch_once (外部手动调, 用于 HTTP 路由 "/dispatch/<text>")
    pub async fn dispatch_once(&self) -> Result<Option<DispatchedRun>, qianxun_core::kanban::KanbanError> {
        let r = self.dispatcher.dispatch_once().await?;
        if let Some(ref dispatched) = r {
            // emit KanbanTaskAssigned SSE
            let task = self.db.get_task(&dispatched.task_id).await?;
            let title = task.map(|t| t.title).unwrap_or_default();
            let _ = self.sse_tx.send(KanbanSseEvent::KanbanTaskAssigned {
                task_id: dispatched.task_id.clone(),
                run_id: dispatched.run_id.clone(),
                profile_name: dispatched.profile_name.clone(),
                title,
            });
        }
        Ok(r)
    }

    /// Emit helper (供 worker 工具调, v6 §4 模式 3 + §8.4)
    pub fn emit(&self, event: KanbanSseEvent) {
        let _ = self.sse_tx.send(event);
    }
}

/// Build KanbanHost from daemon 启动上下文 (MVP-3 plan 0 入口).
pub fn build_host(
    db: KanbanDb,
    team_registry: TeamRegistry,
) -> Arc<KanbanHost> {
    Arc::new(KanbanHost::new(db, team_registry))
}

/// 把 KanbanSseEvent 转成跟现有 SseEvent 兼容的 Value (供 sse.rs 转发).
pub fn kanban_event_to_sse_value(event: &KanbanSseEvent) -> Value {
    serde_json::to_value(event).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kanban_sse_event_type_names() {
        let e = KanbanSseEvent::KanbanTaskAssigned {
            task_id: "t".into(),
            run_id: "r".into(),
            profile_name: "coder".into(),
            title: "x".into(),
        };
        assert_eq!(e.type_name(), "kanban_task_assigned");

        let e = KanbanSseEvent::KanbanTaskProgress {
            task_id: "t".into(),
            run_id: "r".into(),
            event_kind: "comment".into(),
            preview: "p".into(),
        };
        assert_eq!(e.type_name(), "kanban_task_progress");
    }

    #[test]
    fn test_kanban_sse_event_serde_round_trip() {
        let e = KanbanSseEvent::KanbanTaskCompleted {
            task_id: "t".into(),
            run_id: "r".into(),
            outcome: "success".into(),
            summary: "all good".into(),
            token_input: 100,
            token_output: 200,
            elapsed_ms: 5000,
        };
        let json = e.to_sse_data();
        assert!(json.contains("\"type\":\"kanban_task_completed\""));
        assert!(json.contains("\"outcome\":\"success\""));
        assert!(json.contains("\"token_input\":100"));
    }

    #[tokio::test]
    async fn test_kanban_host_subscribe_receives_event() {
        let db = KanbanDb::in_memory().expect("in_memory");
        let reg = TeamRegistry::load_default();
        let host = KanbanHost::new(db, reg);
        let mut rx = host.subscribe();
        // emit 一个事件
        host.emit(KanbanSseEvent::KanbanTaskSpawned {
            parent_task_id: Some("p".into()),
            child_task_id: "c".into(),
            title: "do x".into(),
            assignee_role: "coder".into(),
        });
        // 接收
        let event = rx.recv().await.expect("recv event");
        assert_eq!(event.type_name(), "kanban_task_spawned");
    }
}

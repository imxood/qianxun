//! KanbanError — Kanban 子系统错误类型 (v6 §7.1 / §10)
//!
//! 覆盖 8 大错误场景: 状态机非法转换 / 实体未找到 / scope 护栏 / 角色未注册 /
//! sqlite / json / 持久化 / generic. 全部走 thiserror, 支持 #[from] 转换.

use std::path::PathBuf;

/// Kanban 子系统所有错误的统一类型.
#[derive(thiserror::Error, Debug)]
pub enum KanbanError {
    /// 任务状态转换非法 (v6 §7.1: Triage -> Done 等不允许的转换)
    #[error("invalid state transition: {0} -> {1}")]
    InvalidStateTransition(String, String),

    /// Task 不存在
    #[error("task not found: {0}")]
    TaskNotFound(String),

    /// Board 不存在
    #[error("board not found: {0}")]
    BoardNotFound(String),

    /// Project 不存在
    #[error("project not found: {0}")]
    ProjectNotFound(String),

    /// Run 不存在
    #[error("agent run not found: {0}")]
    RunNotFound(String),

    /// Role 不存在 / 未注册
    #[error("role not registered: {0}")]
    UnknownRole(String),

    /// Profile 不存在
    #[error("profile not registered: {0}")]
    UnknownProfile(String),

    /// 工具不允许在当前 scope 调用 (v6 §4 模式 3 护栏)
    /// Worker 调 Orchestrator-only 工具, 或反之
    #[error("tool '{tool}' not allowed in current scope (need {required}, got {current})")]
    ScopeViolation {
        tool: String,
        required: String,
        current: String,
    },

    /// 父任务不存在 (创建子任务时)
    #[error("parent task not found: {0}")]
    ParentNotFound(String),

    /// 任务依赖图有环 (DAG cycle)
    #[error("dependency cycle detected involving task {0}")]
    DependencyCycle(String),

    /// 黑板写入被并发拒绝 (last writer wins 失败)
    #[error("blackboard write conflict on task={task_id} key={key}")]
    BlackboardConflict { task_id: String, key: String },

    /// 心跳限频: 距上次 heartbeat < 60s, skip
    #[error("heartbeat throttled (last {last_secs}s ago, threshold 60s)")]
    HeartbeatThrottled { last_secs: u64 },

    /// 派生深度超过 TeamConfig.max_spawn_depth
    #[error("spawn depth {current} exceeds max {max}")]
    SpawnDepthExceeded { current: u8, max: u8 },

    /// 并发子任务数超过 max_concurrent_children (排队, 不立即拒绝)
    #[error("concurrent children {current} >= max {max}, queued")]
    ConcurrentChildrenFull { current: u16, max: u16 },

    /// orchestrator_enabled 关闭
    #[error("orchestrator disabled (TeamConfig.orchestrator_enabled = false)")]
    OrchestratorDisabled,

    /// Dispatcher 还在跑 (重复 spawn)
    #[error("dispatcher already running")]
    DispatcherAlreadyRunning,

    /// SQLite 错误 (rusqlite)
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// JSON 错误 (serde_json)
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// IO 错误 (spawn_blocking join / 文件读)
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// spawn_blocking join 错误
    #[error("blocking task panicked: {0}")]
    BlockingJoin(String),

    /// 路径错误 (PathBuf 转换)
    #[error("invalid path: {0:?}")]
    InvalidPath(PathBuf),

    /// generic 错误 (catch-all)
    #[error("kanban error: {0}")]
    Other(String),
}

impl KanbanError {
    /// 简化为 String (用于 log / SSE event)
    pub fn to_log_string(&self) -> String {
        self.to_string()
    }

    /// 是否可恢复 (true = 重试可能成功, false = 永久失败)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            KanbanError::ConcurrentChildrenFull { .. }
                | KanbanError::HeartbeatThrottled { .. }
                | KanbanError::BlackboardConflict { .. }
                | KanbanError::BlockingJoin(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_state_transition_display() {
        let err = KanbanError::InvalidStateTransition("Triage".into(), "Done".into());
        assert_eq!(
            err.to_string(),
            "invalid state transition: Triage -> Done"
        );
    }

    #[test]
    fn test_scope_violation_display() {
        let err = KanbanError::ScopeViolation {
            tool: "kanban_create".into(),
            required: "Orchestrator".into(),
            current: "Worker".into(),
        };
        assert!(err.to_string().contains("kanban_create"));
        assert!(err.to_string().contains("Orchestrator"));
        assert!(err.to_string().contains("Worker"));
    }

    #[test]
    fn test_is_recoverable() {
        assert!(KanbanError::ConcurrentChildrenFull {
            current: 5,
            max: 5,
        }
        .is_recoverable());
        assert!(!KanbanError::UnknownRole("foo".into()).is_recoverable());
        assert!(!KanbanError::DependencyCycle("task_x".into()).is_recoverable());
        assert!(!KanbanError::OrchestratorDisabled.is_recoverable());
    }

    #[test]
    fn test_from_sqlite_error() {
        let sqlite_err = rusqlite::Error::QueryReturnedNoRows;
        let kb_err: KanbanError = sqlite_err.into();
        assert!(kb_err.to_string().contains("sqlite"));
    }

    #[test]
    fn test_from_json_error() {
        let json_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("{invalid}").unwrap_err();
        let kb_err: KanbanError = json_err.into();
        assert!(kb_err.to_string().contains("json"));
    }
}

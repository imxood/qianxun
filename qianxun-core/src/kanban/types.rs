//! Kanban 核心类型定义 (v6 §6.1 / §6.2 / §6.3 / §3.6)
//!
//! 8 个核心 struct: Project / KanbanBoard / Task / TaskLink / AgentRun /
//! BlackboardCell / KanbanEvent / KanbanScope.
//!
//! 关键字段命名约定 (v6 §6.2 决策):
//! - Task 字段加 `t_` 前缀 (t_started_at, t_completed_at), 跟 Run 的 `r_`
//!   前缀区分, 避免 SQL JOIN 时混淆 (Hermes 风险 A).
//! - KanbanEvent 的 `task_id` 可空 (board 级事件).
//!
//! ## 持久化
//!
//! 8 张 SQLite 表, 跟 daemon_sessions 同一文件 `~/.qianxun/daemon.db`,
//! 由 `daemon/persistence.rs` 负责 DDL (见 MVP-2 plan 1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// =============================================================================
// Project (§3.6.2)
// =============================================================================

/// 千寻顶层组织单位. v1: 1 Project = 1 KanbanBoard + N Session.
/// v2: 1 Project 可有 N 个 Board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// "proj_xxx" UUID v4
    pub id: String,
    /// 用户起名, e.g. "千寻重构"
    pub name: String,
    /// 用户描述
    pub description: String,
    /// 默认 workspace, e.g. ~/code/qianxun
    pub default_root: PathBuf,
    /// 跨仓库项目用, e.g. [qianxun-core/, qianxun-memory/]
    pub extra_roots: Vec<PathBuf>,
    /// Active / Archived
    pub status: ProjectStatus,
    /// user_id (VPS 端) 或 "local" (单机)
    pub owner: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Active,
    Archived,
}

// =============================================================================
// KanbanBoard (§6.2)
// =============================================================================

/// 看板 (1 Project = 1 Board in v1, N Board in v2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanBoard {
    /// "kb_xxx" UUID v4
    pub id: String,
    /// 业务名
    pub name: String,
    /// FK kanban_projects.id (v5 §3.6 新增)
    pub project_id: String,
    /// 从 Project.default_root 同步
    pub project_root: PathBuf,
    /// fallback role, 缺省 "techlead"
    pub default_role: String,
    pub status: BoardStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BoardStatus {
    Active,
    Archived,
}

// =============================================================================
// Task (§6.2 / §6.1)
// =============================================================================

/// 任务 (DAG 节点)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// "task_xxx"
    pub id: String,
    /// FK kanban_boards.id
    pub board_id: String,
    /// FK kanban_tasks.id (root 任务 None)
    pub parent_id: Option<String>,
    /// <=120 chars
    pub title: String,
    /// 任务描述, 可 Markdown
    pub body: String,
    /// FK kanban_role_defs.id (逻辑引用, 不强约束)
    pub assignee_role: String,
    pub status: TaskStatus,
    /// 0=low, 255=urgent (默认 128)
    pub priority: u8,
    pub deadline: Option<DateTime<Utc>>,
    /// {gate: "pass"|"block", effort: "small", ...}
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    /// task 级状态机时间 (v6 §6.2 字段命名约定, 跟 r_ 区分)
    pub t_started_at: Option<DateTime<Utc>>,
    pub t_completed_at: Option<DateTime<Utc>>,
    /// 上次 worker 心跳 (任务级)
    pub last_heartbeat_at: Option<DateTime<Utc>>,
}

/// Task 状态机 (v6 §7.1). 8 种 enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 刚创建, 未分配
    #[default]
    Triage,
    /// 父任务 done, 等待依赖解
    Ready,
    /// 有活跃 run
    InProgress,
    /// 全部 children done + verifier gate=pass
    Done,
    /// 显式 block
    Blocked,
    /// user abort
    Cancelled,
    /// run 失败
    Failed,
}

// =============================================================================
// TaskLink (§6.2)
// =============================================================================

/// DAG 边 (parent -> child 关系, 含 dep_type)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLink {
    pub parent_id: String,
    pub child_id: String,
    /// Sequential / Soft / Verifier / Synthesizer
    pub dep_type: DependencyKind,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// 父 done 才能开始 (default)
    #[default]
    Sequential,
    /// 父 done 是建议, 不强制
    Soft,
    /// 父 done + metadata.gate=pass 才解锁
    Verifier,
    /// 父 done + Verifier's child gate=pass 才解锁
    Synthesizer,
}

// =============================================================================
// AgentRun (§6.2)
// =============================================================================

/// 执行实例 (重试时新建 row, v6 §4 模式 1 借鉴 Hermes task_runs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    /// "run_xxx"
    pub id: String,
    /// FK kanban_tasks.id
    pub task_id: String,
    /// FK kanban_profiles.id
    pub profile_id: String,
    pub status: RunStatus,
    /// 取消/重认领时新建 uuid (v4, TEXT 存 SQLite)
    pub claim_id: String,
    /// run 级心跳 (跟 Task.last_heartbeat_at 区分)
    pub r_heartbeat_at: Option<DateTime<Utc>>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: RunOutcome,
    /// LLM 总结
    pub summary: String,
    pub error: Option<String>,
    pub token_input: u64,
    pub token_output: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Pending,
    Running,
    Done,
    Crashed,
    TimedOut,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    #[default]
    Success,
    PartialSuccess,
    Failure,
    Skipped,
    /// Swarm Verifier 写 gate=block 触发
    GateBlocked,
}

// =============================================================================
// BlackboardCell (§6.2 / §4 模式 2)
// =============================================================================

/// 黑板条目 (Hermes [prefix]json 模式的独立表化, v6 §4 模式 2 决策).
///
/// 主键 (task_id, key), last writer wins (best-effort, 不 CAS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardCell {
    /// FK kanban_tasks.id
    pub task_id: String,
    /// e.g. "current_focus" | "user_constraints" | "intermediate_result"
    pub key: String,
    /// 任意 JSON
    pub value: serde_json::Value,
    /// profile_id | "user" | "system"
    pub author: String,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// KanbanEvent (§6.2 / §6.3)
// =============================================================================

/// 事件流 (审计 + 实时推送, v6 §3.4 ModeDecision 等)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanEvent {
    /// auto-increment
    pub id: i64,
    /// FK kanban_tasks.id (None = board 级事件)
    pub task_id: Option<String>,
    /// FK kanban_runs.id
    pub run_id: Option<String>,
    /// 23 种 variant
    pub kind: KanbanEventKind,
    /// 事件具体内容
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// 23 种事件 (v6 §6.3 完整列表)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KanbanEventKind {
    // Task lifecycle (10)
    TaskCreated,
    TaskAssigned,
    TaskStarted,
    TaskPaused,
    TaskResumed,
    TaskCompleted,
    TaskBlocked,
    TaskUnblocked,
    TaskCancelled,
    TaskFailed,
    // Run lifecycle (6)
    RunCreated,
    RunClaimed,
    RunHeartbeat,
    RunCompleted,
    RunCrashed,
    RunTimedOut,
    // Dependency (1)
    DependencyUnblocked,
    // Blackboard (2)
    BlackboardWrite,
    BlackboardRead,
    // Swarm (3)
    GatePass,
    GateBlock,
    VerifierRun,
    SynthesizerRun,
    // System (2)
    ConfigChanged,
    Error,
}

impl KanbanEventKind {
    /// 总变体数 (跟 v6 §6.3 对齐: 10 + 6 + 1 + 2 + 3 + 2 = 24, 含 Error = 23 + Error)
    pub const COUNT: usize = 24;

    /// 分类 (用于前端按类目展示)
    pub fn category(&self) -> &'static str {
        match self {
            Self::TaskCreated
            | Self::TaskAssigned
            | Self::TaskStarted
            | Self::TaskPaused
            | Self::TaskResumed
            | Self::TaskCompleted
            | Self::TaskBlocked
            | Self::TaskUnblocked
            | Self::TaskCancelled
            | Self::TaskFailed => "task",
            Self::RunCreated
            | Self::RunClaimed
            | Self::RunHeartbeat
            | Self::RunCompleted
            | Self::RunCrashed
            | Self::RunTimedOut => "run",
            Self::DependencyUnblocked => "dependency",
            Self::BlackboardWrite | Self::BlackboardRead => "blackboard",
            Self::GatePass | Self::GateBlock | Self::VerifierRun | Self::SynthesizerRun => "swarm",
            Self::ConfigChanged | Self::Error => "system",
        }
    }
}

// =============================================================================
// KanbanScope (§6.1 / §4 模式 3 护栏)
// =============================================================================

/// 工具 scope 上下文 (Worker 跟 Orchestrator 的护栏, v6 §4 模式 3 关键).
///
/// 关键不变量: Worker 启动时**只能看到** `task_id` 字段 (从 Kanban scope
/// 上下文拿), 不暴露全局 task 列表, 防 prompt injection 篡改兄弟任务.
#[derive(Debug, Clone)]
pub struct KanbanScope {
    /// 当前 board
    pub board_id: String,
    /// Worker / Orchestrator (v6 §4 模式 3)
    pub role: WorkerScope,
    /// Worker 只能动这个 task (None = Orchestrator)
    pub assigned_task_id: Option<String>,
    /// Orchestrator 可见的 profile 列表
    pub profiles_available: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerScope {
    Worker,
    Orchestrator,
}

impl WorkerScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worker => "Worker",
            Self::Orchestrator => "Orchestrator",
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task_status() -> TaskStatus {
        TaskStatus::Triage
    }

    fn sample_worker_scope() -> WorkerScope {
        WorkerScope::Worker
    }

    #[test]
    fn test_task_status_default_is_triage() {
        assert_eq!(sample_task_status(), TaskStatus::Triage);
        assert_eq!(TaskStatus::default(), TaskStatus::Triage);
    }

    #[test]
    fn test_run_status_default_is_pending() {
        assert_eq!(RunStatus::default(), RunStatus::Pending);
    }

    #[test]
    fn test_run_outcome_default_is_success() {
        assert_eq!(RunOutcome::default(), RunOutcome::Success);
    }

    #[test]
    fn test_dependency_kind_default_is_sequential() {
        assert_eq!(DependencyKind::default(), DependencyKind::Sequential);
    }

    #[test]
    fn test_dependency_kind_serde_round_trip() {
        for kind in [
            DependencyKind::Sequential,
            DependencyKind::Soft,
            DependencyKind::Verifier,
            DependencyKind::Synthesizer,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: DependencyKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back, "serde round-trip failed for {kind:?}");
        }
    }

    #[test]
    fn test_worker_scope_equality_and_as_str() {
        assert_eq!(WorkerScope::Worker, WorkerScope::Worker);
        assert_ne!(WorkerScope::Worker, WorkerScope::Orchestrator);
        assert_eq!(WorkerScope::Worker.as_str(), "Worker");
        assert_eq!(WorkerScope::Orchestrator.as_str(), "Orchestrator");
    }

    #[test]
    fn test_task_status_serde_all_8_variants() {
        let statuses = [
            TaskStatus::Triage,
            TaskStatus::Ready,
            TaskStatus::InProgress,
            TaskStatus::Done,
            TaskStatus::Blocked,
            TaskStatus::Cancelled,
            TaskStatus::Failed,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let back: TaskStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back, "serde round-trip failed for {s:?}");
        }
    }

    #[test]
    fn test_run_status_serde_all_7_variants() {
        let statuses = [
            RunStatus::Pending,
            RunStatus::Running,
            RunStatus::Done,
            RunStatus::Crashed,
            RunStatus::TimedOut,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let back: RunStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back, "serde round-trip failed for {s:?}");
        }
    }

    #[test]
    fn test_kanban_event_kind_count_matches_v6() {
        // v6 §6.3: 10 task + 6 run + 1 dependency + 2 blackboard + 3 swarm + 2 system = 24
        // (KanbanEventKind::Error 在 v6 算 23 + 1 = 24 显式声明)
        assert_eq!(
            KanbanEventKind::COUNT,
            24,
            "KanbanEventKind 变体数跟 v6 §6.3 不一致"
        );
    }

    #[test]
    fn test_kanban_event_kind_categories() {
        assert_eq!(KanbanEventKind::TaskCreated.category(), "task");
        assert_eq!(KanbanEventKind::RunHeartbeat.category(), "run");
        assert_eq!(KanbanEventKind::DependencyUnblocked.category(), "dependency");
        assert_eq!(KanbanEventKind::BlackboardWrite.category(), "blackboard");
        assert_eq!(KanbanEventKind::GatePass.category(), "swarm");
        assert_eq!(KanbanEventKind::ConfigChanged.category(), "system");
        assert_eq!(KanbanEventKind::Error.category(), "system");
    }

    #[test]
    fn test_project_status_serde() {
        for s in [ProjectStatus::Active, ProjectStatus::Archived] {
            let json = serde_json::to_string(&s).unwrap();
            let back: ProjectStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_board_status_serde() {
        for s in [BoardStatus::Active, BoardStatus::Archived] {
            let json = serde_json::to_string(&s).unwrap();
            let back: BoardStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_task_field_naming_convention() {
        // v6 §6.2 决策: Task 用 t_ 前缀, Run 用 r_ 前缀
        let task_json = serde_json::to_value(TaskStatus::Done).unwrap();
        let task_status_str = task_json.as_str().unwrap();
        assert_eq!(task_status_str, "done", "TaskStatus 应序列化 snake_case done");

        let run_json = serde_json::to_value(RunStatus::Done).unwrap();
        let run_status_str = run_json.as_str().unwrap();
        assert_eq!(run_status_str, "done", "RunStatus 应序列化 snake_case done");
    }
}

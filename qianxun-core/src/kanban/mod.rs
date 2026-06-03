//! 千寻 Kanban 子系统 (v6 §14.1 MVP-2 第一步)
//!
//! 8 张 SQLite 表 + 12 个 kanban_* 工具 + 4 个 pattern dispatcher + 黑板 +
//! 任务/运行解耦 (Hermes 借鉴, 见 `04-kanban-design.md` v6 §4 M1).
//!
//! ## 模块结构
//!
//! ```text
//! kanban/
//!   mod.rs            # 本文件: 模块入口 + 公共导出
//!   types.rs          # 8 个核心 struct (Project / Task / AgentRun / BlackboardCell / ...)
//!   error.rs          # KanbanError (thiserror)
//!   state_machine.rs  # 7 状态 + check_transition + recompute_parent (MVP-2 plan 3)
//!   dispatcher.rs     # KanbanDispatcher (MVP-2 plan 4)
//!   db.rs             # KanbanDb CRUD 28+ 方法 (MVP-2 plan 2)
//! ```
//!
//! ## 关联
//!
//! - `tools/builtin/kanban.rs` — 12 个 kanban_* 工具 (MVP-2 plan 5)
//! - `agent/team.rs` — Profile / Role / TeamConfig / TeamRegistry (MVP-2 plan 4)
//! - `agent/pattern.rs` — 4 个 pattern dispatcher (MVP-2 plan 6)
//! - `blackboard/` — BlackboardCell 封装 (MVP-2 plan 4)
//! - `daemon/persistence.rs` — 8 张表 DDL + ALTER TABLE (MVP-2 plan 1)
//! - `daemon/kanban_host.rs` — daemon 端 Kanban host (MVP-3)

pub mod db;
pub mod error;
pub mod state_machine;
pub mod types;

pub use db::KanbanDb;
pub use error::KanbanError;
pub use types::{
    AgentRun, BlackboardCell, BoardStatus, DependencyKind, KanbanBoard, KanbanEvent,
    KanbanEventKind, KanbanScope, Project, ProjectStatus, RunOutcome, RunStatus, Task,
    TaskLink, TaskStatus, WorkerScope,
};

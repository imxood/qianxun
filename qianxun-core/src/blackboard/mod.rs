//! 千寻黑板模块 (v6 §4 模式 2 + §6.2 BlackboardCell)
//!
//! 黑板是 worker / orchestrator 之间共享状态的轻量机制. Hermes 用
//! `[prefix] json` task_comments 实现, 千寻 v6 决策用独立表
//! (kanban_blackboard, MVP-2 plan 1 已建).
//!
//! 本模块:
//! - `BlackboardCell` 已在 `kanban::types` 定义 (跟 SQLite 行对应)
//! - `KanbanDb::read_blackboard / write_blackboard` (MVP-2 plan 2)
//! - 本文件作为黑板模块的入口, 后续可加 Blackboard 锁 / 观察者 / 广播
//!
//! MVP-2 阶段: 黑板 CRUD 由 KanbanDb 负责, 不在本模块加抽象.

pub use crate::kanban::types::BlackboardCell;

//! Session 持久化主入口 (从 persistence.rs 抽, 2026-06-04 Commit 11)
//!
//! SessionStore struct + 4 DAO impl 文件 (session_dao / snapshot_dao /
//! event_log_dao / kanban_schema) + error / types / ddl.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

pub use self::error::SessionStoreError;
pub use self::types::{EventEntry, SessionMeta};

mod ddl;
mod error;
mod event_log_dao;
mod kanban_schema;
mod session_dao;
mod snapshot_dao;
mod types;

#[cfg(test)]
mod tests;

pub use kanban_schema::{ensure_default_project, init_kanban_schema};

/// SessionStore — 3 张表 CRUD 封装.
///
/// 设计为 `Arc<SessionStore>`, 多个 task (prompt_handler / 恢复 / 管理 API)
/// 共享同一份连接. 内部 `db: Mutex<Connection>` 串行化写, 配合
/// `spawn_blocking` 使用.
pub struct SessionStore {
    db: Arc<Mutex<Connection>>,
}

impl SessionStore {
    /// 暴露共享 connection (给 MVP-3 KanbanDb 复用 daemon.db, v6 §7.3 决策).
    pub fn db_arc(&self) -> std::sync::Arc<std::sync::Mutex<rusqlite::Connection>> {
        self.db.clone()
    }

    /// 打开 / 创建 SQLite 数据库, 初始化 3 张表.
    pub fn new(path: &Path) -> Result<Self, SessionStoreError> {
        // 确保父目录存在
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(path)?;
        // 启用外键约束 (CASCADE 删除依赖)
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        // WAL 模式支持并发读
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        ddl::create_tables(&conn)?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开内存数据库 (用于测试).
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, SessionStoreError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        ddl::create_tables(&conn)?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Stage 10b: 优雅关闭时强制 checkpoint WAL.
    pub fn flush(&self) -> Result<(), SessionStoreError> {
        let conn = self.db.lock()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

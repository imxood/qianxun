//! Session 持久化错误类型 (从 persistence.rs 抽, 2026-06-04 Commit 11)

use thiserror::Error;

/// Session 持久化错误.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("conversation format error: {0}")]
    ConversationFormat(#[from] qianxun_core::agent::conversation::ConversationFormatError),

    #[error("connection lock poisoned")]
    LockPoisoned,
}

impl From<std::sync::PoisonError<std::sync::MutexGuard<'_, rusqlite::Connection>>> for SessionStoreError {
    fn from(_: std::sync::PoisonError<std::sync::MutexGuard<'_, rusqlite::Connection>>) -> Self {
        SessionStoreError::LockPoisoned
    }
}

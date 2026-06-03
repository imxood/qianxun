//! SessionStore 共享类型 (从 persistence.rs 抽, 2026-06-04 Commit 11)

/// Session 元数据 (用于 `list_active`).
#[derive(Debug, Clone)]
#[allow(dead_code)] // 部分字段 Stage 4 才被 `restore_from_disk` 完整使用
pub struct SessionMeta {
    pub id: String,
    pub project_root: Option<String>,
    pub status: String,
    pub created_at: String,
    pub last_active_at: String,
    pub message_count: u32,
}

/// 事件日志条目 (用于 `load_events`).
#[derive(Debug, Clone)]
#[allow(dead_code)] // 字段在 Stage 4 恢复路径才被读
pub struct EventEntry {
    pub seq: u32,
    pub event_type: String,
    pub event_json: String,
}

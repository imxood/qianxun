use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 会话 ID 类型。
pub type SessionId = String;

/// 观测 ID 类型。
pub type ObsId = String;

/// 记忆 ID 类型。
pub type MemoryId = String;

/// 会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub project: String,
    pub cwd: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: SessionStatus,
    pub observation_count: u32,
    pub model: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SessionStatus {
    Active,
    Ended,
}

/// Hook 类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HookType {
    PreToolUse,
    PostToolUse,
}

/// 观测类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ObservationType {
    FileRead,
    FileWrite,
    FileEdit,
    CommandRun,
    Error,
    Search,
    Think,
    Other,
}

/// 压缩后的观测。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: ObsId,
    pub session_id: SessionId,
    pub timestamp: DateTime<Utc>,
    pub obs_type: ObservationType,
    pub title: String,
    pub subtitle: Option<String>,
    pub facts: Vec<String>,
    pub narrative: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub importance: u8,
    pub confidence: Option<f64>,
}

/// 跨会话持久记忆。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub mem_type: MemoryType,
    pub title: String,
    pub content: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub strength: u8,
    pub version: u32,
    pub parent_id: Option<MemoryId>,
    pub is_latest: bool,
    pub forget_after: Option<DateTime<Utc>>,
    pub project: Option<String>,
    pub access_count: u64,
    pub last_accessed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryType {
    Architecture,
    Pattern,
    Preference,
    Bug,
    Workflow,
    Fact,
}

/// 工作记忆插槽。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySlot {
    pub label: String,
    pub content: String,
    pub size_limit: usize,
    pub description: String,
    pub pinned: bool,
    pub scope: SlotScope,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SlotScope {
    Project,
    Global,
}

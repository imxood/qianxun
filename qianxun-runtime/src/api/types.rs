// qianxun-runtime/src/api/types.rs
// RuntimeApi 5 个方法共用的 request/response 类型.
//
// 设计:
//   - 全 derive(Serialize, Deserialize), 给 HTTP JSON / Tauri IPC 共用
//   - 字段命名跟 Svelte frontend entity/Plan.ts / entity/Session.ts 1:1 (sub-task #4 接入零摩擦)
//   - 时间字段用 String (ISO 8601), 跟 SessionMeta.list_active 现有结构一致, 不引 chrono serde
//   - enum 区分状态, 序列化用 snake_case 跟 TypeScript union 兼容

use serde::{Deserialize, Serialize};

/// Session 状态过滤 (list_sessions 的入参).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionFilter {
    /// 只列内存中活跃的 (running agent loop)
    Active,
    /// 只列已暂停的
    Paused,
    /// 只列持久化但内存中已驱逐的 (重启后可 restore_from_disk 加载)
    Stored,
    /// 不过滤, 全列 (default)
    #[default]
    All,
}

/// Session 状态 (list_sessions 返回的 status 字段).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Paused,
    Stored,
}

/// 单个 session 摘要 (list_sessions 数组元素).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub model: String,
    pub status: SessionStatus,
    pub created_at: String,
    pub last_active_at: String,
    pub message_count: u32,
}

/// list_sessions 返回的容器 (含总数 + 内存中活跃/暂停计数 + filter 回显).
///
/// 跟原 daemon list_sessions 1:1 字段 (含 `filter` 回显给前端验证用).
/// 改用 snake_case 跟 Svelte frontend 1:1 兼容.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionInfo>,
    pub total: usize,
    /// 回显请求的 filter ("active" / "paused" / "stored" / "all")
    pub filter: String,
    pub active_in_memory: usize,
    pub paused_in_memory: usize,
}

/// send_message 请求体.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequest {
    /// 用户消息列表 (按时间顺序). Stage 2 简化: 只支持 user role 纯文本.
    #[serde(default)]
    pub messages: Vec<SendMessage>,
    /// 可选: 覆盖 session 默认 model (Stage 2 暂忽略, 留 config 切换用).
    #[serde(default)]
    pub model: Option<String>,
}

/// send_message 单条消息.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessage {
    /// "user" / "assistant" / "system" — Stage 2 简化: 实际只处理 user.
    pub role: String,
    /// 文本内容.
    pub content: String,
}

/// send_message 立即返回的响应 (SSE 事件流走 Receiver<SseEvent>, 跟 HTTP chunked 同源).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResponse {
    /// 已接受请求, 流开始发送. 客户端拿 session_id + status 校验.
    pub session_id: String,
    pub status: &'static str, // 永远是 "streaming"
}

/// Plan 状态 (简单三态, 跟 Svelte 端 Plan['status'] 1:1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    Running,
    Done,
    Aborted,
}

/// create_plan 入参 (sub-task #3 简化: 只收 session_id + name + description).
/// 后续 sub-task 接完整 contract (tasks / assigned_to / verify_prompt 等).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanInput {
    pub session_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 可选: 超时 ms (默认 30 min). sub-task #3 不真用, 留 PlanInfo 透传.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// create_plan / list_plans 返回的 plan 摘要.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanInfo {
    pub id: String,
    pub session_id: String,
    pub name: String,
    pub status: PlanStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
}

/// load_session 返回的完整 session 状态 (含 conversation snapshot).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub exists_in_memory: bool,
    pub status: SessionStatus,
    /// Conversation 序列化为 JSON 字符串 (Stage 4 简化: 用现有 snapshot 格式).
    /// Optional 因为某些 session 可能只有 metadata 没 snapshot.
    pub conversation_json: Option<String>,
    pub message_count: u32,
}

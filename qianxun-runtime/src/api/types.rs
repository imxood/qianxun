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
    /// 2026-06-09 加: 工作目录根 (跟 create_session 的 project_root 透传).
    /// 前端 projectStore 用此字段去重 derive project 列表.
    /// None 表示会话未绑定项目 (顶层 "Chat" 入口).
    #[serde(default)]
    pub project_root: Option<String>,
}

/// create_session 入参 (前端 invoke 透传).
///
/// `model` 暂未使用 — SessionRuntime 内部从 `SharedState.resolved` 拿 active provider,
/// 跟 SendRequest.model 一致 (Stage 2 简化, 留 config 切换用).
/// `project_root` 透传给 AgentLoopHost::create_session (工作目录关联).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub project_root: Option<String>,
}

/// update_active_provider 入参 (前端 Provider 设置 UI 调).
///
/// 行为 (2026-06-09 加):
/// 1. 后端把 active_provider 字段写到 ~/.qianxun/config.json (原子写)
/// 2. **不**热替换 runtime.provider (避免破坏 send_message in-flight task)
/// 3. 前端收到 success 后**提示用户重启 desktop** (改动需重启生效)
///
/// 理由: 简化实现, 跟当前 "RuntimeState 启动时构造, 之后不变" 一致.
/// 后续 P1 可加 "热替换 provider" (ArcSwap + 重置 mpsc 通道).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProviderRequest {
    /// 新激活的 provider 名称 (e.g. "deepseek" / "MiniMax" / 自定义)
    pub active_provider: String,
    /// 可选: 同时更新该 provider 的配置 (api_key / model / base_url).
    /// None 表示只切 active, 不动 provider 配置.
    #[serde(default)]
    pub provider_config: Option<qianxun_core::config::ProviderConfig>,
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

/// Plan 状态 (Phase D 收尾: 加 Pending/Failed 跟 Svelte 端 PlanStatus 5 态 1:1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    /// 调度中, 还没开始第一个 task.
    Pending,
    Running,
    Done,
    /// 任一 task 失败 → 整个 plan 失败.
    Failed,
    Aborted,
}

/// Plan 任务 (Phase D 收尾: 跟 Svelte 端 PlanTaskSpec 字段对齐).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTaskSpec {
    pub id: String,
    pub title: String,
    pub prompt: String,
    /// 角色 ("coder" / "tester" / "researcher" 等), 后续按角色配 LLM 行为.
    #[serde(default)]
    pub assigned_to: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// 单 task 超时 ms (0 = 不超时).
    #[serde(default)]
    pub timeout_ms: u64,
}

/// Plan contract (跟 Svelte 端 PlanContract 1:1 字段名).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlanContract {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 任务列表 (Phase D 必填, 之前是 0 任务时业务空跑).
    pub tasks: Vec<PlanTaskSpec>,
    #[serde(default)]
    pub timeout_ms: u64,
}

/// 单个 task 的执行结果 (Phase D 收尾: 给前端展示).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTaskResult {
    pub id: String,
    pub status: PlanStatus,
    /// LLM 最后一条 assistant 消息内容 (简版; 后续可换多消息).
    pub output: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
}

/// create_plan 入参 (Phase D 收尾: 必填 contract.tasks).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanInput {
    pub session_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 可选: 整个 plan 超时 ms. 0 = 不超时.
    #[serde(default)]
    pub timeout_ms: u64,
    /// Phase D: 任务列表. 之前是 0 任务时业务空跑, 现在每个 task 走 LLM 真实执行.
    #[serde(default)]
    pub tasks: Vec<PlanTaskSpec>,
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
    /// Phase D 收尾: 任务的执行结果 (按 tasks 顺序).
    #[serde(default)]
    pub task_results: Vec<PlanTaskResult>,
    /// Phase D 收尾: contract 透传 (前端拿到后能 join plan 跟 task 关系).
    #[serde(default)]
    pub contract: PlanContract,
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

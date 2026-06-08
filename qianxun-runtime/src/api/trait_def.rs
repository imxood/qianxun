// qianxun-runtime/src/api/trait_def.rs
// RuntimeApi trait 定义 — 5 个方法 + 关联类型.
//
// Trait 在 api/trait_def.rs, impl 在 core.rs (impl block 必须单文件, Rust 语法约束).
// 所有方法 async, 内部走 async_trait (跟 qianxun-core 一致).
//
// 关键设计:
//   - list_sessions / cancel / load 同步返回 Result<T>, 业务 < 100ms 完成
//   - send 异步返回 Result<mpsc::Receiver<SseEvent>>, 业务起后台 task 后立即返
//   - plans create/list 也是同步返回, in-memory HashMap 锁保护

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::api::error::RuntimeApiResult;
use crate::api::types::{
    ListSessionsResponse, PlanInfo, PlanInput, SendRequest, SendResponse, SessionFilter,
    SessionState,
};
use crate::sse::SseEvent;

/// 千寻运行时核心业务接口 (跨 daemon HTTP router + Tauri command).
///
/// 5 个方法语义:
/// - `list_sessions`:  列所有 session (store metadata + 内存 runtime 状态合并)
/// - `send_message`:   推 user 消息 + 起后台 agent loop + 返回 SSE event 通道
/// - `create_plan`:    在指定 session 上建一个 plan (Running 状态)
/// - `cancel_session`: 软取消 session (agent_host cancel_session, 跟现有 daemon 一致)
/// - `load_session`:   从 store 加载 session 完整状态 (含 conversation snapshot)
///
/// 设计原则:
/// - 不绑 HTTP / Tauri 任何协议: 返回 mpsc::Receiver 给业务流, 让 caller 包装
/// - 不绑具体错误类型: 5 个方法共用 RuntimeApiError, HTTP layer map 到 StatusCode
/// - 不绑 SessionStore / AgentLoopHost: trait 是接口, 业务实现在 core.rs
#[async_trait]
pub trait RuntimeApi: Send + Sync {
    /// 列 session (可选 status 过滤).
    ///
    /// 业务: 合并 store.list_active 元数据 + agent_host.get_session 内存态
    /// (跟 daemon router list_sessions 1:1 行为). filter = "active" / "paused" / "stored" / "all".
    async fn list_sessions(
        &self,
        filter: SessionFilter,
    ) -> RuntimeApiResult<ListSessionsResponse>;

    /// 推 user 消息 + 起 agent loop, 返回 SSE 事件流.
    ///
    /// 业务 (跟 daemon prompt_handler 1:1):
    /// 1. 验证 session 存在 (404 if not)
    /// 2. 推 user 消息到 conversation
    /// 3. 构造 AgentLoop + DaemonOutputSink
    /// 4. spawn tokio task 跑 processing_loop::handle_user_message
    /// 5. 返 mpsc::Receiver<SseEvent>, HTTP 包装 Sse, Tauri 包装 emit event
    async fn send_message(
        &self,
        session_id: &str,
        req: SendRequest,
    ) -> RuntimeApiResult<(SendResponse, mpsc::Receiver<SseEvent>)>;

    /// 在指定 session 上建一个 plan, 立即返 Pending 状态.
    ///
    /// Phase D 收尾: 真实执行 — spawn 后台 task 顺序跑每个 task (LLM + tools),
    /// plan status 从 Pending → Running → Done/Failed, task_results 累积.
    ///
    /// 业务 (sub-task #3 简化): in-memory HashMap, 锁保护.
    async fn create_plan(&self, input: PlanInput) -> RuntimeApiResult<PlanInfo>;

    /// 列所有 plan (Tauri Settings 面板用, daemon 暂不接).
    async fn list_plans(&self) -> RuntimeApiResult<Vec<PlanInfo>>;

    /// 取消正在跑的 plan (Phase D 收尾加). 把 status 置 Aborted, 写 ended_at.
    async fn cancel_plan(&self, plan_id: &str) -> RuntimeApiResult<()>;

    /// 取消正在跑的 session (软取消, agent_host 设置 paused flag).
    async fn cancel_session(&self, session_id: &str) -> RuntimeApiResult<()>;

    /// 加载 session 完整状态 (含 conversation snapshot).
    async fn load_session(&self, session_id: &str) -> RuntimeApiResult<SessionState>;
}

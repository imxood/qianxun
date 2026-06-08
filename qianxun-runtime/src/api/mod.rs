// qianxun-runtime/src/api/mod.rs
// RuntimeApi — daemon HTTP router 跟 Tauri command 共用的核心业务接口.
//
// 5 个方法 (Stage 4a sub-task #3 范围):
//   1. sessions  — 列 session (支持 status filter: active/paused/stored/all)
//   2. send      — 推 user 消息 → 异步返回 SSE 事件流 (channel receiver)
//   3. plans     — Plan CRUD (create_plan 跟 list_plans, 简单 in-memory)
//   4. cancel    — 取消正在跑的 session (调 agent_host.cancel_session)
//   5. load      — 从 store 加载 session 完整状态 (含 conversation snapshot)
//
// 设计:
//   - 异步 trait (async_trait), 方法可 await agent_host / store / LLM
//   - 返回类型都用 thiserror 错误 + Result<T, RuntimeApiError>
//   - 流式方法 (send) 返回 mpsc::Receiver<SseEvent>, HTTP 包装成 Sse, Tauri 包装成 emit event
//   - 类型分文件: types.rs 收口 request/response 结构体, 5 个方法文件各 1 个
//   - impl 块在 core.rs, 业务逻辑 1:1 搬 daemon router (sub-task #3 不改业务, 只搬位置)

pub mod cancel;
pub mod error;
pub mod load;
pub mod plans;
pub mod send;
pub mod sessions;
pub mod trait_def;
pub mod types;

pub use error::{RuntimeApiError, RuntimeApiResult};
pub use trait_def::RuntimeApi;
pub use types::{
    PlanInfo, PlanInput, PlanStatus, SendRequest, SendResponse, SessionFilter, SessionInfo,
    SessionState, SessionStatus,
};

// 重导出 core 里的 trait impl, 让 caller 直接 `use qianxun_runtime::api::RuntimeApiExt;`
pub use crate::core::RuntimeApiExt;

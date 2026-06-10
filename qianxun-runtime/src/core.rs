// qianxun-runtime/src/core.rs
// RuntimeApi impl — 在 Arc<RuntimeState> 上实现 RuntimeApi trait.
//
// 设计:
//   - trait 5 个方法都用 `pub async fn xxx_impl(state, ...)` 在 api/ 各个文件定义业务
//   - 本文件 impl block 只是薄委托 (一行调对应 _impl), 不做业务
//   - impl block 必须在单文件 (Rust 语法约束), 所以本文件 ~50 行可接受
//   - 通过 RuntimeApiExt extension trait 让 Arc<RuntimeState> 直接调 .list_sessions() 等
//
// 调用模式:
//   use qianxun_runtime::api::RuntimeApiExt;
//   let sessions = state.list_sessions(SessionFilter::All).await?;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::api::background_task::{
    cancel_background_task_impl, get_background_task_impl, list_background_tasks_impl,
    resume_background_task_impl, start_background_task_impl,
};
use crate::api::cancel::cancel_session_impl;
use crate::api::error::RuntimeApiResult;
use crate::api::load::load_session_impl;
use crate::api::plans::{cancel_plan_impl, create_plan_impl, list_plans_impl};
use crate::api::send::send_message_impl;
use crate::api::sessions::{
    create_session_impl, delete_session_impl, list_sessions_impl, pause_session_impl,
    resume_session_impl, update_active_provider_impl,
};
use crate::api::trait_def::RuntimeApi;
use crate::api::types::{
    CreateSessionRequest, ListSessionsResponse, PlanInfo, PlanInput, SendRequest, SendResponse,
    SessionFilter, SessionInfo, SessionState, UpdateProviderRequest,
};
use crate::background_task::{TaskInfo, TaskStatus};
use crate::sse::SseEvent;
use crate::state::RuntimeState;

/// RuntimeApi extension — 在 Arc<RuntimeState> 上自动获得 .list_sessions() 等方法.
///
/// 通过 blanket impl 让 Arc<RuntimeState>: RuntimeApi 自动获得 .xxx() 调用,
/// caller 不用先 `state.runtime.list_sessions()` 而是 `state.list_sessions()`.
///   1. `state: Arc<RuntimeState>` 直接当 RuntimeApi 用
///   2. `&Arc<RuntimeState>` 也自动 deref 到 Arc<RuntimeState>
///   3. `&RuntimeState` 也自动转 Arc<RuntimeState> 调 (有成本: clone Arc)
#[async_trait]
impl RuntimeApi for Arc<RuntimeState> {
    async fn list_sessions(
        &self,
        filter: SessionFilter,
    ) -> RuntimeApiResult<ListSessionsResponse> {
        list_sessions_impl(self.clone(), filter).await
    }

    async fn create_session(
        &self,
        req: CreateSessionRequest,
    ) -> RuntimeApiResult<SessionInfo> {
        create_session_impl(self.clone(), req).await
    }

    async fn send_message(
        &self,
        session_id: &str,
        req: SendRequest,
    ) -> RuntimeApiResult<(SendResponse, mpsc::Receiver<SseEvent>)> {
        send_message_impl(self.clone(), session_id, req).await
    }

    async fn send_message_to_sub_session(
        &self,
        sub_session_id: &str,
        req: SendRequest,
    ) -> RuntimeApiResult<(SendResponse, mpsc::Receiver<SseEvent>)> {
        // 4a-2 P0-2: 务实版 — sub_session_id 当前等同于 session_id (前端解析 parent 后传).
        // P1 阶段 (sub_session 持久化缺口) 改造为查 sub_session store 拿 parent_session_id.
        send_message_impl(self.clone(), sub_session_id, req).await
    }

    async fn create_plan(&self, input: PlanInput) -> RuntimeApiResult<PlanInfo> {
        create_plan_impl(self.clone(), input).await
    }

    async fn list_plans(&self) -> RuntimeApiResult<Vec<PlanInfo>> {
        list_plans_impl(self.clone()).await
    }

    async fn cancel_plan(&self, plan_id: &str) -> RuntimeApiResult<()> {
        cancel_plan_impl(self.clone(), plan_id).await
    }

    async fn cancel_session(&self, session_id: &str) -> RuntimeApiResult<()> {
        cancel_session_impl(self.clone(), session_id).await
    }

    async fn load_session(&self, session_id: &str) -> RuntimeApiResult<SessionState> {
        load_session_impl(self.clone(), session_id).await
    }

    async fn delete_session(&self, session_id: &str) -> RuntimeApiResult<()> {
        delete_session_impl(self.clone(), session_id).await
    }

    async fn pause_session(&self, session_id: &str) -> RuntimeApiResult<()> {
        pause_session_impl(self.clone(), session_id).await
    }

    async fn resume_session(&self, session_id: &str) -> RuntimeApiResult<()> {
        resume_session_impl(self.clone(), session_id).await
    }

    async fn update_active_provider(
        &self,
        req: UpdateProviderRequest,
    ) -> RuntimeApiResult<()> {
        update_active_provider_impl(self.clone(), req).await
    }

    // ─── 缺口 05: 后台异步任务方法 (Stage 5 新增) ───

    async fn start_background_task(
        &self,
        task_kind: String,
        opts: serde_json::Value,
    ) -> RuntimeApiResult<TaskInfo> {
        start_background_task_impl(self.clone(), task_kind, opts).await
    }

    async fn get_background_task(
        &self,
        task_id: &str,
    ) -> RuntimeApiResult<TaskInfo> {
        get_background_task_impl(self.clone(), task_id).await
    }

    async fn cancel_background_task(
        &self,
        task_id: &str,
        reason: String,
    ) -> RuntimeApiResult<()> {
        cancel_background_task_impl(self.clone(), task_id, reason).await
    }

    async fn resume_background_task(
        &self,
        task_id: &str,
    ) -> RuntimeApiResult<()> {
        resume_background_task_impl(self.clone(), task_id).await
    }

    async fn list_background_tasks(
        &self,
        filter: Option<TaskStatus>,
    ) -> RuntimeApiResult<Vec<TaskInfo>> {
        list_background_tasks_impl(self.clone(), filter).await
    }
}

/// RuntimeApiExt — 给 caller 写起来更直观的 re-export.
/// 实际上 RuntimeApi trait 已经给 Arc<RuntimeState> 加了方法, 这里只做 alias.
pub trait RuntimeApiExt: RuntimeApi {}
impl<T: RuntimeApi + ?Sized> RuntimeApiExt for T {}

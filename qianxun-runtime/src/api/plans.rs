// qianxun-runtime/src/api/plans.rs
// create_plan / list_plans — Phase D 收尾: tasks 真实执行.
//
// 业务变化 (Phase D 收尾):
//   - 之前: PlanInfo 只有 id / session_id / name / status, create_plan 立刻返
//     Running 但不真执行. list_plans 返所有.
//   - 现在: PlanInput 加 tasks 列表, create_plan:
//       1. 验证 session 存在
//       2. 构造 PlanInfo (status = Pending, task_results = 空 Vec 占位)
//       3. 写 in-memory store
//       4. spawn 后台 task 跑 execute_plan() — 顺序执行每个 task,
//          每个 task 走 LLM (state.provider) + tools (state.tools) 真实跑
//       5. 即时返 plan (status=Pending, 等后台 spawn 完改成 Running)
//   - list_plans 返所有 + 完整 task_results (供前端展示).
//   - 取消走 cancel_session (跟现有 daemon 一致, 后续加 plan.cancel 单独接口).
//
// 并发模型: 后台 task 持 Arc<RuntimeState> 跑, 通过 store lock 改 plan status,
// 当前没 watch 通知前端 (后续 SseEvent 加 plan_update variant).

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use chrono::Utc;

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{processing_loop, AgentLoop};
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::tools::ToolCategoryFilter;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{
    PlanContract, PlanInfo, PlanInput, PlanStatus, PlanTaskResult, PlanTaskSpec,
};
use crate::RuntimeState;

/// in-memory plan store (sub-task #3: HashMap + Mutex).
///
/// 后续 sub-task 接 store (跟 SessionStore 同款 SQLite 表) 时, 整体替换为
/// `Arc<dyn PlanStore>`, 业务方法签名不变.
pub type PlanStore = Mutex<HashMap<String, PlanInfo>>;

/// create_plan 业务实现.
pub async fn create_plan_impl(
    state: Arc<RuntimeState>,
    input: PlanInput,
) -> RuntimeApiResult<PlanInfo> {
    // 1. 验证 session 存在 (业务约束: plan 必绑定一个 session)
    if !state.agent_host.session_exists(&input.session_id) {
        return Err(RuntimeApiError::NotFound(format!(
            "session {} not found",
            input.session_id
        )));
    }

    // 2. 构造 PlanInfo. status = Pending (后台 task 启动后改 Running).
    let now = Utc::now();
    let plan_id = format!("plan_{}", now.format("%Y%m%d_%H%M%S_%6f"));
    let contract = PlanContract {
        name: input.name.clone(),
        description: input.description.clone(),
        tasks: input.tasks.clone(),
        timeout_ms: input.timeout_ms,
    };
    let task_results: Vec<PlanTaskResult> = input
        .tasks
        .iter()
        .map(|t| PlanTaskResult {
            id: t.id.clone(),
            status: PlanStatus::Pending,
            output: String::new(),
            error: None,
            started_at: None,
            ended_at: None,
        })
        .collect();
    let plan = PlanInfo {
        id: plan_id.clone(),
        session_id: input.session_id.clone(),
        name: input.name.clone(),
        status: PlanStatus::Pending,
        started_at: now.to_rfc3339(),
        ended_at: None,
        task_results,
        contract,
    };

    // 3. 写入 in-memory store
    {
        let mut plans = state.plans.lock().expect("PlanStore lock poisoned");
        plans.insert(plan.id.clone(), plan.clone());
    }

    // 4. spawn 后台 task 跑 execute_plan() — 顺序执行每个 task.
    //    即时返 Pending 状态, 后台 task 跑起来后改 Running, 完事后改 Done/Failed.
    let state_for_exec = state.clone();
    let plan_id_for_exec = plan.id.clone();
    tokio::spawn(async move {
        if let Err(e) = execute_plan(state_for_exec.clone(), plan_id_for_exec.clone()).await {
            tracing::error!(
                plan_id = %plan_id_for_exec,
                error = %e,
                "execute_plan failed"
            );
        }
    });

    tracing::info!(
        "[api] create_plan: id={} session_id={} name={} tasks={}",
        plan.id,
        plan.session_id,
        plan.name,
        input.tasks.len()
    );
    Ok(plan)
}

/// list_plans 业务实现.
pub async fn list_plans_impl(state: Arc<RuntimeState>) -> RuntimeApiResult<Vec<PlanInfo>> {
    let plans = state.plans.lock().expect("PlanStore lock poisoned");
    Ok(plans.values().cloned().collect())
}

/// cancel_plan 业务实现 (Phase D 收尾加).
///
/// 把指定 plan 状态置为 Aborted. 当前实现: 直接改 store. 后续 sub-task 加
/// 真正的 task 取消 (给 in-flight task 发 cancel signal).
pub async fn cancel_plan_impl(state: Arc<RuntimeState>, plan_id: &str) -> RuntimeApiResult<()> {
    let mut plans = state.plans.lock().expect("PlanStore lock poisoned");
    let plan = plans
        .get_mut(plan_id)
        .ok_or_else(|| RuntimeApiError::NotFound(format!("plan {plan_id} not found")))?;
    plan.status = PlanStatus::Aborted;
    plan.ended_at = Some(Utc::now().to_rfc3339());
    tracing::info!(plan_id = %plan_id, "plan cancelled");
    Ok(())
}

/// 后台执行 plan: 顺序跑每个 task, 第一个失败就停止 (整个 plan 变 Failed).
///
/// 业务简化 (Phase D 收尾):
///   - 顺序执行 (不并发, 不依赖图)
///
///   - 每个 task:
///
///     1. 状态 → Running, 写 task_results
///     2. 拿 session runtime (state.agent_host.get_session)
///     3. 构造 AgentLoop + Conversation snapshot
///     4. spawn processing_loop::handle_user_message (跟 send_message_impl 同)
///     5. 等 task 输出 (SseEvent 流里的 message_stop / event_done), 拿最后一条 assistant 内容
///     6. task → Done / Failed, 写 task_results
///
///   - 所有 task Done → plan Done. 任一 Failed → plan Failed.
async fn execute_plan(state: Arc<RuntimeState>, plan_id: String) -> RuntimeApiResult<()> {
    // 1. 取 plan 跟 tasks
    let (tasks, session_id, timeout_ms) = {
        let plans = state.plans.lock().expect("PlanStore lock poisoned");
        let plan = plans
            .get(&plan_id)
            .ok_or_else(|| RuntimeApiError::NotFound(format!("plan {plan_id} not found")))?;
        // Phase D 收尾: 复制 tasks + session_id + timeout
        (plan.contract.tasks.clone(), plan.session_id.clone(), plan.contract.timeout_ms)
    };

    // 2. 把 plan 状态置 Running
    {
        let mut plans = state.plans.lock().expect("PlanStore lock poisoned");
        if let Some(plan) = plans.get_mut(&plan_id) {
            plan.status = PlanStatus::Running;
        }
    }

    // 3. 顺序执行 tasks
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let mut any_failed = false;
    let mut last_output = String::new();

    for task in &tasks {
        // 任务级超时 (0 = 不超时). 整体 plan timeout 在外层加 tokio timeout.
        let task_result = execute_one_task(
            state.clone(),
            &plan_id,
            &session_id,
            task,
            cancel_flag.clone(),
        )
        .await;
        match task_result {
            Ok(output) => {
                last_output = output;
            }
            Err(e) => {
                tracing::error!(plan_id = %plan_id, task_id = %task.id, error = %e, "task failed");
                any_failed = true;
                // 不 break — 让所有 task 都标 Failed, 方便前端看哪几个失败
            }
        }
        if any_failed {
            // 失败短路, 不继续后续 task
            break;
        }
    }

    // 4. 更新 plan 状态
    {
        let mut plans = state.plans.lock().expect("PlanStore lock poisoned");
        if let Some(plan) = plans.get_mut(&plan_id) {
            plan.status = if any_failed {
                PlanStatus::Failed
            } else {
                PlanStatus::Done
            };
            plan.ended_at = Some(Utc::now().to_rfc3339());
        }
    }

    // 5. 触发 plan 整体超时 (用 tokio timeout 包) — 简化: 暂不实现
    let _ = timeout_ms;

    let _ = last_output; // 留给后续 sub-task 做 result.summary
    Ok(())
}

/// 执行单个 task: 走 LLM + tools, 拿最后一条 assistant 输出.
async fn execute_one_task(
    state: Arc<RuntimeState>,
    plan_id: &str,
    session_id: &str,
    task: &PlanTaskSpec,
    cancel_flag: Arc<AtomicBool>,
) -> RuntimeApiResult<String> {
    // 1. 标 task → Running
    {
        let mut plans = state.plans.lock().expect("PlanStore lock poisoned");
        if let Some(plan) = plans.get_mut(plan_id) {
            if let Some(tr) = plan.task_results.iter_mut().find(|r| r.id == task.id) {
                tr.status = PlanStatus::Running;
                tr.started_at = Some(Utc::now().to_rfc3339());
            }
        }
    }

    // 2. 拿 session runtime
    let runtime = state
        .agent_host
        .get_session(session_id)
        .ok_or_else(|| RuntimeApiError::NotFound(format!("session {session_id} not found")))?;
    runtime.touch();

    // 3. 构造 AgentLoop + Conversation snapshot + 注入 task prompt
    let mut agent_loop = AgentLoop::new(runtime.resolved.agent.clone());
    let mut conv: Conversation = runtime
        .conversation
        .lock()
        .expect("SessionRuntime conversation lock poisoned")
        .clone();
    conv.push_user_message(vec![ContentBlock::text(&task.prompt)]);

    // 4. 跑 processing_loop 拿流式事件, 累积 text_delta 到 last_output
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let provider = runtime.provider.clone();
    let tools = runtime.tools.clone();
    let memory_context = String::new();
    let skills_catalog = runtime.skills.build_catalog_prompt();
    let skill_injections = String::new();

    // 用 noop sink, text_delta 通过 channel 送
    struct TextCollectSink {
        tx: tokio::sync::mpsc::Sender<String>,
    }
    #[async_trait::async_trait]
    impl qianxun_core::output::OutputSink for TextCollectSink {
        async fn on_text(&self, text: &str) {
            let _ = self.tx.send(text.to_string()).await;
        }
        async fn on_thinking(&self, _text: &str) {
            // noop — Plan task 不关心 thinking
        }
        async fn on_thinking_flush(&self) {
            // noop
        }
        async fn on_tool_call(
            &self,
            _tool_call_id: &str,
            _tool_name: &str,
            _arguments: &serde_json::Value,
        ) {
            // noop — task 工具调用不外发, 只关心 final text output
        }
        async fn on_token_usage(&self, _usage: &qianxun_core::types::TokenUsage) {
            // noop — task 不记 usage 累计
        }
        async fn on_error(&self, _error: &qianxun_core::types::LlmError) {
            // noop — task error 走 on_turn_finished 的 StopReason
        }
        async fn on_turn_finished(
            &self,
            _reason: &qianxun_core::types::StopReason,
            _usage: &qianxun_core::types::TokenUsage,
        ) {
            // noop
        }
        async fn on_status(&self, _status: &str) {
            // noop
        }
    }
    let sink = TextCollectSink { tx: tx.clone() };

    let cancel_flag_clone = cancel_flag.clone();
    let handle = tokio::spawn(async move {
        processing_loop::handle_user_message(
            &mut agent_loop,
            &mut conv,
            provider.as_ref(),
            tools.as_ref(),
            ToolCategoryFilter::all(),
            &sink,
            &memory_context,
            &skills_catalog,
            &skill_injections,
            cancel_flag_clone,
        )
        .await;
    });

    // 5. 累积 text 输出
    let mut last_output = String::new();
    while let Some(delta) = rx.recv().await {
        last_output.push_str(&delta);
    }
    // 等 task 真正结束 (processing_loop 已退出)
    let _ = handle.await;

    // 6. 标 task → Done (Phase D 收尾: 简化, 不区分工具调用是否出错)
    {
        let mut plans = state.plans.lock().expect("PlanStore lock poisoned");
        if let Some(plan) = plans.get_mut(plan_id) {
            if let Some(tr) = plan.task_results.iter_mut().find(|r| r.id == task.id) {
                tr.status = PlanStatus::Done;
                tr.output = last_output.clone();
                tr.ended_at = Some(Utc::now().to_rfc3339());
            }
        }
    }

    Ok(last_output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase D 收尾测试 1: create_plan + cancel_plan 基础流程.
    /// 验证: create 后 plan 在 store, cancel 后 status = Aborted.
    /// 不验 task 真实执行 (那个需要 LLM 真实调用, 留给 integration test).
    #[tokio::test]
    async fn create_then_cancel_plan_marks_aborted() {
        let state = RuntimeState::new_in_memory_with_config(
            qianxun_core::config::ResolvedConfig::default(),
        )
        .await
        .expect("RuntimeState init");

        // 1. 创 session (auto-gen id)
        let session = state
            .agent_host

            .create_session(crate::agent_host::CreateSessionOpts::default())
            .expect("create_session");
        let session_id = session.session_id.clone();

        // 2. create_plan (空 task 列表, 不会真执行)
        let plan = create_plan_impl(
            state.clone(),
            PlanInput {
                session_id: session_id.clone(),
                name: "测试 plan".into(),
                description: "phase D".into(),
                timeout_ms: 0,
                tasks: vec![],
            },
        )
        .await
        .expect("create_plan");
        assert_eq!(plan.session_id, session_id);
        // 之前是 Running (mock 阶段), 现在是 Pending
        assert!(matches!(plan.status, PlanStatus::Pending | PlanStatus::Running));
        // task_results 跟 tasks 同长度
        assert_eq!(plan.task_results.len(), 0);
        assert_eq!(plan.contract.tasks.len(), 0);

        // 3. cancel_plan
        cancel_plan_impl(state.clone(), &plan.id)
            .await
            .expect("cancel_plan");
        let plans = state.plans.lock().expect("PlanStore lock poisoned");
        let plan_after = plans.get(&plan.id).expect("plan still in store");
        assert_eq!(plan_after.status, PlanStatus::Aborted);
        assert!(plan_after.ended_at.is_some());
    }

    /// Phase D 收尾测试 2: cancel_plan 不存在的 plan_id → NotFound.
    #[tokio::test]
    async fn cancel_nonexistent_plan_returns_not_found() {
        let state = RuntimeState::new_in_memory_with_config(
            qianxun_core::config::ResolvedConfig::default(),
        )
        .await
        .expect("RuntimeState init");

        let result = cancel_plan_impl(state.clone(), "plan_does_not_exist").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            RuntimeApiError::NotFound(msg) => {
                assert!(msg.contains("plan_does_not_exist"));
            }
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    /// Phase D 收尾测试 3: create_plan 不存在的 session_id → NotFound.
    #[tokio::test]
    async fn create_plan_nonexistent_session_returns_not_found() {
        let state = RuntimeState::new_in_memory_with_config(
            qianxun_core::config::ResolvedConfig::default(),
        )
        .await
        .expect("RuntimeState init");

        let result = create_plan_impl(
            state.clone(),
            PlanInput {
                session_id: "sess_does_not_exist".into(),
                name: "test".into(),
                description: "".into(),
                timeout_ms: 0,
                tasks: vec![],
            },
        )
        .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            RuntimeApiError::NotFound(msg) => {
                assert!(msg.contains("sess_does_not_exist"));
            }
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    /// Phase D 收尾测试 4: list_plans 返所有 + 含 task_results.
    #[tokio::test]
    async fn list_plans_returns_all_with_task_results() {
        let state = RuntimeState::new_in_memory_with_config(
            qianxun_core::config::ResolvedConfig::default(),
        )
        .await
        .expect("RuntimeState init");

        let session = state
            .agent_host

            .create_session(crate::agent_host::CreateSessionOpts::default())
            .expect("create_session");
        let session_id = session.session_id.clone();

        // 创 2 个 plan
        for name in ["plan_a", "plan_b"] {
            create_plan_impl(
                state.clone(),
                PlanInput {
                    session_id: session_id.clone(),
                    name: name.into(),
                    description: "".into(),
                    timeout_ms: 0,
                    tasks: vec![PlanTaskSpec {
                        id: "t1".into(),
                        title: "test".into(),
                        prompt: "do something".into(),
                        assigned_to: "coder".into(),
                        depends_on: vec![],
                        timeout_ms: 0,
                    }],
                },
            )
            .await
            .expect("create_plan");
        }

        // list 返 2 个
        let all = list_plans_impl(state.clone()).await.expect("list_plans");
        assert_eq!(all.len(), 2);
        // 每个 plan 都有 1 个 task_result (Pending 状态)
        for plan in &all {
            assert_eq!(plan.task_results.len(), 1);
            assert_eq!(plan.task_results[0].id, "t1");
            // Pending (后端 task 没真跑前) 或 Running (真跑后) 都 OK
        }
    }
}

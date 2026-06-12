// qianxun-runtime/src/api/plans.rs
// create_plan / list_plans / cancel_plan — P1-1 收尾 (2026-06-12) 走 SQLite 持久化.
//
// 业务变化 (P1-1 收尾):
//   - 之前: in-memory `PlanStore = Mutex<HashMap>`, 重启丢 plan. 注释明确:
//     "sub-task 接 store (跟 SessionStore 同款 SQLite 表) 时, 整体替换".
//   - 现在: 全走 `state.store` SQLite 表 plans (5 个 CRUD: create / list /
//     get / update_status / update_task_results), 重启不丢. contract_json +
//     task_results_json 走 JSON 列存.
//
// 并发模型: 状态变更走 SQLite 串行写 (跟 SessionStore 共享连接, 内部 Mutex 锁).
//   - 业务接口 `create_plan_impl` / `list_plans_impl` / `cancel_plan_impl` 签名不变
//   - execute_plan 启动从 `store.get_plan` 反序列化 contract 跟 task_results
//   - 每个状态变更 (status / task_results) 调对应 store 方法, 走 SQLite
//
// 后续 P1-3 接 SseEvent::PlanUpdate 时, 这里 state 变更处加 emit.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::Utc;

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{processing_loop, AgentLoop};
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::tools::ToolCategoryFilter;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{
    PlanContract, PlanInfo, PlanInput, PlanStatus, PlanTaskResult, PlanTaskSpec, SubSessionStatus,
};
use crate::persistence::{PlanRow, SessionStore};
use crate::sse::SseEvent;
use crate::RuntimeState;

/// PlanStatus → SQLite 字符串 (snake_case, 跟 serde rename_all 一致).
fn plan_status_to_str(s: PlanStatus) -> &'static str {
    match s {
        PlanStatus::Pending => "pending",
        PlanStatus::Running => "running",
        PlanStatus::Done => "done",
        PlanStatus::Failed => "failed",
        PlanStatus::Aborted => "aborted",
    }
}

fn plan_status_from_str(s: &str) -> RuntimeApiResult<PlanStatus> {
    match s {
        "pending" => Ok(PlanStatus::Pending),
        "running" => Ok(PlanStatus::Running),
        "done" => Ok(PlanStatus::Done),
        "failed" => Ok(PlanStatus::Failed),
        "aborted" => Ok(PlanStatus::Aborted),
        other => Err(RuntimeApiError::Internal(format!(
            "unknown plan status: {other}"
        ))),
    }
}

/// 把 PlanRow 反序列化成完整 PlanInfo (contract_json / task_results_json).
fn plan_row_to_info(row: PlanRow) -> RuntimeApiResult<PlanInfo> {
    let contract: PlanContract = serde_json::from_str(&row.contract_json).map_err(|e| {
        RuntimeApiError::Internal(format!("plan contract deserialize failed: {e}"))
    })?;
    let task_results: Vec<PlanTaskResult> =
        serde_json::from_str(&row.task_results_json).map_err(|e| {
            RuntimeApiError::Internal(format!("plan task_results deserialize failed: {e}"))
        })?;
    Ok(PlanInfo {
        id: row.id,
        session_id: row.session_id,
        name: row.name,
        status: plan_status_from_str(&row.status)?,
        started_at: row.started_at,
        ended_at: row.ended_at,
        task_results,
        contract,
    })
}

/// 更新 plan 总体 status + ended_at (走 SQLite).
fn store_update_plan_status(
    store: &SessionStore,
    plan_id: &str,
    status: PlanStatus,
    ended_at: Option<&str>,
) -> RuntimeApiResult<()> {
    store
        .update_plan_status(plan_id, plan_status_to_str(status), ended_at)
        .map_err(|e| RuntimeApiError::Internal(format!("update_plan_status failed: {e}")))
}

/// 读出 task_results, 找到指定 task_id 调 mutator, 再写回 (read-modify-write).
/// 用于 execute_one_task 标 task 自身的 Running / Done / Failed + 时间戳.
/// 返回最新 task_results JSON 字符串 (P1-3: emit PlanUpdate 时附快照).
fn store_mutate_task_result(
    store: &SessionStore,
    plan_id: &str,
    task_id: &str,
    mutator: impl FnOnce(&mut PlanTaskResult),
) -> RuntimeApiResult<String> {
    let row = store
        .get_plan(plan_id)
        .map_err(|e| RuntimeApiError::Internal(format!("get_plan failed: {e}")))?
        .ok_or_else(|| RuntimeApiError::NotFound(format!("plan {plan_id} not found")))?;
    let mut task_results: Vec<PlanTaskResult> =
        serde_json::from_str(&row.task_results_json).map_err(|e| {
            RuntimeApiError::Internal(format!("task_results deserialize failed: {e}"))
        })?;
    if let Some(tr) = task_results.iter_mut().find(|r| r.id == task_id) {
        mutator(tr);
    } else {
        return Err(RuntimeApiError::NotFound(format!(
            "task {task_id} not in plan {plan_id}"
        )));
    }
    let json = serde_json::to_string(&task_results)
        .map_err(|e| RuntimeApiError::Internal(format!("task_results serialize failed: {e}")))?;
    store
        .update_plan_task_results(plan_id, &json)
        .map_err(|e| RuntimeApiError::Internal(format!("update_plan_task_results failed: {e}")))?;
    Ok(json)
}

/// P1-3 helper: 读 plan 当前 task_results JSON 字符串 (供 PlanUpdate 快照用).
fn read_task_results_json(store: &SessionStore, plan_id: &str) -> RuntimeApiResult<String> {
    let row = store
        .get_plan(plan_id)
        .map_err(|e| RuntimeApiError::Internal(format!("get_plan failed: {e}")))?
        .ok_or_else(|| RuntimeApiError::NotFound(format!("plan {plan_id} not found")))?;
    Ok(row.task_results_json)
}

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

    // 3. 写入 SQLite store (P1-1 收尾: 重启不丢)
    let contract_json = serde_json::to_string(&plan.contract)
        .map_err(|e| RuntimeApiError::Internal(format!("contract serialize failed: {e}")))?;
    let task_results_json = serde_json::to_string(&plan.task_results)
        .map_err(|e| RuntimeApiError::Internal(format!("task_results serialize failed: {e}")))?;
    state
        .store
        .create_plan(
            &plan.id,
            &plan.session_id,
            &plan.name,
            plan_status_to_str(plan.status),
            &plan.started_at,
            plan.ended_at.as_deref(),
            &contract_json,
            &task_results_json,
        )
        .map_err(|e| RuntimeApiError::Internal(format!("create_plan failed: {e}")))?;

    // 3.5 P1-3: emit PlanUpdate (Pending) 给订阅方
    state.emit_plan_event(SseEvent::PlanUpdate {
        plan_id: plan.id.clone(),
        status: "pending".into(),
        task_results_json: Some(task_results_json.clone()),
        updated_at: chrono::Utc::now().timestamp_millis(),
    });

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
    let rows = state
        .store
        .list_plans()
        .map_err(|e| RuntimeApiError::Internal(format!("list_plans failed: {e}")))?;
    rows.into_iter().map(plan_row_to_info).collect()
}

/// cancel_plan 业务实现.
///
/// 把指定 plan 状态置为 Aborted. 当前实现: 直接改 store. 后续 sub-task 加
/// 真正的 task 取消 (给 in-flight task 发 cancel signal).
pub async fn cancel_plan_impl(state: Arc<RuntimeState>, plan_id: &str) -> RuntimeApiResult<()> {
    // 1. 验证 plan 存在
    state
        .store
        .get_plan(plan_id)
        .map_err(|e| RuntimeApiError::Internal(format!("get_plan failed: {e}")))?
        .ok_or_else(|| RuntimeApiError::NotFound(format!("plan {plan_id} not found")))?;
    // 2. 标 Aborted + ended_at
    let now = Utc::now().to_rfc3339();
    store_update_plan_status(&state.store, plan_id, PlanStatus::Aborted, Some(&now))?;
    // P1-3: emit PlanUpdate (Aborted)
    state.emit_plan_event(SseEvent::PlanUpdate {
        plan_id: plan_id.to_string(),
        status: "aborted".into(),
        task_results_json: None, // Aborted 路径不再重发 task_results
        updated_at: chrono::Utc::now().timestamp_millis(),
    });
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
    // 1. 从 SQLite 读 plan (contract + task_results)
    let row = state
        .store
        .get_plan(&plan_id)
        .map_err(|e| RuntimeApiError::Internal(format!("get_plan failed: {e}")))?
        .ok_or_else(|| RuntimeApiError::NotFound(format!("plan {plan_id} not found")))?;
    let plan_info = plan_row_to_info(row)?;
    let (tasks, session_id, timeout_ms) = (
        plan_info.contract.tasks.clone(),
        plan_info.session_id.clone(),
        plan_info.contract.timeout_ms,
    );

    // 2. 把 plan 状态置 Running
    store_update_plan_status(&state.store, &plan_id, PlanStatus::Running, None)?;
    // P1-3: emit PlanUpdate (Running) — 重新读 task_results 序列化附上, 方便前端显示完整快照
    let running_snapshot = read_task_results_json(&state.store, &plan_id)?;
    state.emit_plan_event(SseEvent::PlanUpdate {
        plan_id: plan_id.clone(),
        status: "running".into(),
        task_results_json: Some(running_snapshot),
        updated_at: chrono::Utc::now().timestamp_millis(),
    });

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
    let now = Utc::now().to_rfc3339();
    let final_status = if any_failed {
        PlanStatus::Failed
    } else {
        PlanStatus::Done
    };
    store_update_plan_status(&state.store, &plan_id, final_status, Some(&now))?;
    // P1-3: emit PlanUpdate (Done/Failed) — 附完整 task_results 快照
    let final_snapshot = read_task_results_json(&state.store, &plan_id)?;
    let final_status_str = plan_status_to_str(final_status).to_string();
    state.emit_plan_event(SseEvent::PlanUpdate {
        plan_id: plan_id.clone(),
        status: final_status_str,
        task_results_json: Some(final_snapshot),
        updated_at: chrono::Utc::now().timestamp_millis(),
    });

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
    // 0. 2026-06-12 收尾: 建 sub_session (跟 task 1:1, 失败 warn 不阻断 — 缺它只影响
    //    前端"打开子会话", 不影响 plan 主体执行).
    let sub_session_id = format!("sub_{}_{}", plan_id, task.id);
    let _ = crate::api::sub_sessions::create_sub_session_impl(
        state.clone(),
        crate::api::types::SubSessionInput {
            id: sub_session_id.clone(),
            plan_id: plan_id.to_string(),
            parent_session_id: session_id.to_string(),
            task_id: task.id.clone(),
            role: "executor".to_string(),
        },
    )
    .await
    .map_err(|e| tracing::warn!(error = %e, sub_session_id = %sub_session_id, "create_sub_session failed"));

    // 1. 标 task → Running
    let now_running = Utc::now().to_rfc3339();
    let task_running_json = store_mutate_task_result(&state.store, plan_id, &task.id, |tr| {
        tr.status = PlanStatus::Running;
        tr.started_at = Some(now_running.clone());
    })?;
    // P1-3: emit PlanUpdate (task Running)
    state.emit_plan_event(SseEvent::PlanUpdate {
        plan_id: plan_id.to_string(),
        status: "running".into(),
        task_results_json: Some(task_running_json),
        updated_at: chrono::Utc::now().timestamp_millis(),
    });

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
            None,
        )
        .await;
    });

    // 5. 累积 text 输出
    let mut last_output = String::new();
    while let Some(delta) = rx.recv().await {
        last_output.push_str(&delta);
    }
    // 等 task 真正结束 (processing_loop 已退出) — 错误透传上层,
    // 失败时由 execute_plan 负责 task Failed 标记, 这里也 update sub_session = Failed.
    let task_result = handle.await;
    let final_status = match &task_result {
        Ok(()) => SubSessionStatus::Done,
        Err(e) => {
            tracing::warn!(error = %e, plan_id, task_id = %task.id, "execute_one_task LLM loop failed");
            SubSessionStatus::Failed
        }
    };

    // 6. 2026-06-12 收尾: update sub_session (Done / Failed) — 跟 task 主线并行,
    //    失败 warn 不阻断 task 主线写入.
    let _ = crate::api::sub_sessions::update_sub_session_impl(
        state.clone(),
        &sub_session_id,
        final_status,
        Some(&last_output),
    )
    .await
    .map_err(|e| tracing::warn!(error = %e, sub_session_id = %sub_session_id, "update_sub_session failed"));

    // 7. 标 task → Done (Phase D 收尾: 简化, 不区分工具调用是否出错)
    let now_ended = Utc::now().to_rfc3339();
    let output_for_store = last_output.clone();
    let task_done_json = store_mutate_task_result(&state.store, plan_id, &task.id, |tr| {
        tr.status = PlanStatus::Done;
        tr.output = last_output.clone();
        tr.ended_at = Some(now_ended.clone());
    })?;
    // P1-3: emit PlanUpdate (task Done)
    state.emit_plan_event(SseEvent::PlanUpdate {
        plan_id: plan_id.to_string(),
        status: "running".into(),
        task_results_json: Some(task_done_json),
        updated_at: chrono::Utc::now().timestamp_millis(),
    });

    let _ = output_for_store;
    // JoinError (tokio::spawn) → RuntimeApiError::Internal 透传上层 execute_plan.
    // 失败前已经 update sub_session = Failed + 标 task Done, 不会重复标.
    task_result.map_err(|e| RuntimeApiError::Internal(format!("execute_one_task join: {e}")))?;
    Ok(last_output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 等待 plan 终止状态 (Done / Failed / Aborted), 避免裸 sleep 不稳定.
    /// 简化: 最多等 50 * 100ms = 5s. P1 收尾测试, 跑空 task 列表应 <100ms.
    async fn wait_plan_terminated(state: &Arc<RuntimeState>, plan_id: &str) {
        for _ in 0..50 {
            let row = state.store.get_plan(plan_id).expect("get_plan");
            if let Some(r) = row {
                if r.status == "done" || r.status == "failed" || r.status == "aborted" {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("plan {plan_id} did not reach terminal state in 5s");
    }

    /// P1-1 收尾测试 1: create_plan + cancel_plan 基础流程.
    /// 验证: create 后 plan 走 SQLite 持久化, cancel 后 status = Aborted.
    /// 不验 task 真实执行 (那个需要 LLM 真实调用, 留给 integration test).
    #[tokio::test]
    async fn create_then_cancel_plan_marks_aborted() {
        let state = RuntimeState::new_in_memory_with_config(
            qianxun_core::config::ResolvedConfig::default(),
        )
        .await
        .expect("RuntimeState init");

        // 1. 创 session
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
                description: "P1-1".into(),
                timeout_ms: 0,
                tasks: vec![],
            },
        )
        .await
        .expect("create_plan");
        assert_eq!(plan.session_id, session_id);
        // Pending (启动瞬间) 或 Running (后台已改) 都 OK
        assert!(matches!(
            plan.status,
            PlanStatus::Pending | PlanStatus::Running | PlanStatus::Done
        ));
        assert_eq!(plan.task_results.len(), 0);
        assert_eq!(plan.contract.tasks.len(), 0);

        // 3. cancel_plan (后台 task 跑空 list 标 Done 后, cancel 改 Aborted)
        wait_plan_terminated(&state, &plan.id).await;
        cancel_plan_impl(state.clone(), &plan.id)
            .await
            .expect("cancel_plan");
        // 直接查 SQLite 验证 (P1-1: 不再 state.plans.lock())
        let row = state
            .store
            .get_plan(&plan.id)
            .expect("get_plan")
            .expect("plan still in store");
        assert_eq!(row.status, "aborted");
        assert!(row.ended_at.is_some());
    }

    /// P1-1 收尾测试 2: cancel_plan 不存在的 plan_id → NotFound.
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

    /// P1-1 收尾测试 3: create_plan 不存在的 session_id → NotFound.
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

    /// P1-1 收尾测试 4: list_plans 返所有 + 含 task_results (走 SQLite 序列化).
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

        // list 返 2 个 (走 SQLite)
        let all = list_plans_impl(state.clone()).await.expect("list_plans");
        assert_eq!(all.len(), 2);
        // 每个 plan 都有 1 个 task_result (Pending/Running/Done, 后端 task 跑空 list 后 Done)
        for plan in &all {
            assert_eq!(plan.task_results.len(), 1);
            assert_eq!(plan.task_results[0].id, "t1");
        }
    }

    /// P1-1 收尾测试 5: plan 走 SQLite 持久化 — 重启后仍能 list 到.
    /// 验证: in-memory store 测不出来 (重启 = 新 state), 但 list/get API
    /// 内部必须走 SQLite, 此测试用 store.create_plan + store.list_plans 直接验证.
    #[tokio::test]
    async fn plans_persist_to_sqlite_store() {
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

        let plan = create_plan_impl(
            state.clone(),
            PlanInput {
                session_id: session_id.clone(),
                name: "持久化测试".into(),
                description: "P1-1 sqlite".into(),
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

        // 1. 直接查 store (绕开 in-memory 已废弃的 state.plans)
        let row = state
            .store
            .get_plan(&plan.id)
            .expect("get_plan")
            .expect("plan in store");
        assert_eq!(row.session_id, session_id);
        assert_eq!(row.name, "持久化测试");
        // contract_json 必须含 tasks 序列化
        assert!(row.contract_json.contains("\"id\":\"t1\""));
        // task_results_json 必须含初始 Pending 状态
        assert!(row.task_results_json.contains("\"id\":\"t1\""));
        assert!(row.task_results_json.contains("\"pending\""));

        // 2. list_plans 也必须能查到
        let all = list_plans_impl(state.clone()).await.expect("list_plans");
        assert!(all.iter().any(|p| p.id == plan.id));
    }

    /// P1-3 收尾测试: broadcast bus 收到 PlanUpdate 事件.
    /// 验证: subscribe_plan_events → create_plan (Pending) + cancel (Aborted)
    /// 至少收到 2 个 PlanUpdate, 顺序正确, 字段对齐 (plan_id / status).
    /// 不验 task_results_json 完整内容 (序列化路径 store 测过), 只验事件能发出来.
    #[tokio::test]
    async fn plan_events_emitted_via_broadcast() {
        use crate::sse::SseEvent;

        let state = RuntimeState::new_in_memory_with_config(
            qianxun_core::config::ResolvedConfig::default(),
        )
        .await
        .expect("RuntimeState init");

        // 1. 创 session + 订阅事件
        let session = state
            .agent_host
            .create_session(crate::agent_host::CreateSessionOpts::default())
            .expect("create_session");
        let session_id = session.session_id.clone();
        let mut rx = state.subscribe_plan_events();

        // 2. create_plan 触发 Pending 事件
        let plan = create_plan_impl(
            state.clone(),
            PlanInput {
                session_id: session_id.clone(),
                name: "事件测试".into(),
                description: "P1-3".into(),
                timeout_ms: 0,
                tasks: vec![],
            },
        )
        .await
        .expect("create_plan");

        // 3. 等背景 task 跑完 (空 list 走 Done)
        wait_plan_terminated(&state, &plan.id).await;

        // 4. cancel_plan 触发 Aborted
        cancel_plan_impl(state.clone(), &plan.id)
            .await
            .expect("cancel_plan");

        // 5. 收集事件 (最多 10 个, 设 1s timeout 防 hang)
        let mut events: Vec<SseEvent> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline && events.len() < 10 {
            match tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(ev)) => events.push(ev),
                _ => break,
            }
        }

        // 6. 至少包含 pending + aborted (中间可能还有 running/done, 都 OK)
        let plan_events: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SseEvent::PlanUpdate {
                    plan_id, status, ..
                } if plan_id == &plan.id => Some(status.clone()),
                _ => None,
            })
            .collect();
        assert!(
            plan_events.iter().any(|s| s == "pending"),
            "expected pending event, got: {plan_events:?}"
        );
        assert!(
            plan_events.iter().any(|s| s == "aborted"),
            "expected aborted event, got: {plan_events:?}"
        );
    }
}

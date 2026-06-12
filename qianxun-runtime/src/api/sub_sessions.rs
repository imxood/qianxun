// qianxun-runtime/src/api/sub_sessions.rs
// sub_session 4 个 RuntimeApi 方法 — 2026-06-12 收尾 (跟 plans 1:1 对齐, 但更薄).
//
// 业务背景: E2E Round 1 反馈 "主会话中没有任何交互提示 如 等待子Agent" +
// "plan 列表点击打开子会话无法打开" — 根因是 execute_one_task 没建 sub_session 实体,
// 前端 subSessionStore.byPlan() 永远 [].
// 本次补: execute_one_task 启动时调 create_sub_session, 完成 / 失败时调
// update_sub_session, 每次变更 emit SubSessionUpdate. 前端 init 调
// list_sub_sessions(None) 拉全量, 后续 onSubSessionEvent 增量.
//
// 跟 plans 的差异: 不暴露 contract / task_results, 只暴露 status / output —
// 任务上下文从前端 plan 侧拿, sub_session 只负责 "谁在跑 / 跑到哪 / 出了什么".

use std::sync::Arc;

use chrono::Utc;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{SubSessionInfo, SubSessionInput, SubSessionStatus};
use crate::persistence::SubSessionRow;
use crate::sse::SseEvent;
use crate::RuntimeState;

/// SubSessionRow → 公开 SubSessionInfo. 1:1 平铺, 字段命名跟 store 列对齐.
fn row_to_info(row: SubSessionRow) -> SubSessionInfo {
    SubSessionInfo {
        id: row.id,
        plan_id: row.plan_id,
        parent_session_id: row.parent_session_id,
        task_id: row.task_id,
        role: row.role,
        status: row.status,
        started_at: row.started_at,
        ended_at: row.ended_at,
        output: row.output,
    }
}

/// 公开 list 接口 — 跟 store.list_sub_sessions 1:1 转发.
pub async fn list_sub_sessions_impl(
    state: Arc<RuntimeState>,
    plan_id: Option<&str>,
) -> RuntimeApiResult<Vec<SubSessionInfo>> {
    let rows = state
        .store
        .list_sub_sessions(plan_id)
        .map_err(|e| RuntimeApiError::Internal(format!("list_sub_sessions: {e}")))?;
    Ok(rows.into_iter().map(row_to_info).collect())
}

/// 公开 get 接口.
pub async fn get_sub_session_impl(
    state: Arc<RuntimeState>,
    sub_session_id: &str,
) -> RuntimeApiResult<SubSessionInfo> {
    let row = state
        .store
        .get_sub_session(sub_session_id)
        .map_err(|e| RuntimeApiError::Internal(format!("get_sub_session: {e}")))?
        .ok_or_else(|| RuntimeApiError::NotFound(format!("sub_session {sub_session_id} not found")))?;
    Ok(row_to_info(row))
}

/// 内部 create — execute_one_task 启动时调. 立即 emit SubSessionUpdate (Active).
pub async fn create_sub_session_impl(
    state: Arc<RuntimeState>,
    input: SubSessionInput,
) -> RuntimeApiResult<()> {
    let started_at = Utc::now().to_rfc3339();
    state
        .store
        .create_sub_session(
            &input.id,
            &input.plan_id,
            &input.parent_session_id,
            &input.task_id,
            &input.role,
            &started_at,
        )
        .map_err(|e| RuntimeApiError::Internal(format!("create_sub_session: {e}")))?;
    emit_sub_session_update(&state, &input, SubSessionStatus::Active, None, &started_at);
    Ok(())
}

/// 内部 update — execute_one_task 完成 / 失败时调. emit SubSessionUpdate (Done/Failed/Aborted).
pub async fn update_sub_session_impl(
    state: Arc<RuntimeState>,
    sub_session_id: &str,
    status: SubSessionStatus,
    output: Option<&str>,
) -> RuntimeApiResult<()> {
    let ended_at = Utc::now().to_rfc3339();
    state
        .store
        .update_sub_session(sub_session_id, status.as_str(), Some(&ended_at), output)
        .map_err(|e| RuntimeApiError::Internal(format!("update_sub_session: {e}")))?;

    // 拿最新 row 用于 emit payload (前端 onSubSessionEvent 拿到完整实体).
    let row = state
        .store
        .get_sub_session(sub_session_id)
        .map_err(|e| RuntimeApiError::Internal(format!("update_sub_session re-read: {e}")))?
        .ok_or_else(|| {
            RuntimeApiError::NotFound(format!("sub_session {sub_session_id} disappeared after update"))
        })?;
    let info = row_to_info(row);
    state.emit_sub_session_event(SseEvent::SubSessionUpdate {
        sub_session_id: info.id.clone(),
        plan_id: info.plan_id.clone(),
        task_id: info.task_id.clone(),
        status: info.status.clone(),
        sub_session_json: serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string()),
        updated_at: Utc::now().timestamp_millis(),
    });
    Ok(())
}

/// create 时的 emit 辅助 — 没有完整 row (刚 insert, store 返回 ()), 用 input 跟 started_at
/// 拼一份最小 SubSessionInfo 反序列化进 payload, 保持前端 onSubSessionEvent 拿到的是完整 JSON.
fn emit_sub_session_update(
    state: &Arc<RuntimeState>,
    input: &SubSessionInput,
    status: SubSessionStatus,
    output: Option<&str>,
    started_at: &str,
) {
    let info = SubSessionInfo {
        id: input.id.clone(),
        plan_id: input.plan_id.clone(),
        parent_session_id: input.parent_session_id.clone(),
        task_id: input.task_id.clone(),
        role: input.role.clone(),
        status: status.as_str().to_string(),
        started_at: started_at.to_string(),
        ended_at: None,
        output: output.map(|s| s.to_string()),
    };
    state.emit_sub_session_event(SseEvent::SubSessionUpdate {
        sub_session_id: info.id.clone(),
        plan_id: info.plan_id.clone(),
        task_id: info.task_id.clone(),
        status: info.status.clone(),
        sub_session_json: serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string()),
        updated_at: Utc::now().timestamp_millis(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use qianxun_core::config::ResolvedConfig;

    /// 1. create + get + list 三件套: 单条 round-trip 正确.
    #[tokio::test]
    async fn create_get_list_sub_session_roundtrip() {
        let state = RuntimeState::new_in_memory_with_config(ResolvedConfig::default())
            .await
            .expect("new state");
        let input = SubSessionInput {
            id: "sub_p1_t1".into(),
            plan_id: "p1".into(),
            parent_session_id: "s1".into(),
            task_id: "t1".into(),
            role: "executor".into(),
        };
        create_sub_session_impl(state.clone(), input).await.expect("create");
        let got = get_sub_session_impl(state.clone(), "sub_p1_t1").await.expect("get");
        assert_eq!(got.id, "sub_p1_t1");
        assert_eq!(got.status, "active");
        let all = list_sub_sessions_impl(state.clone(), None).await.expect("list all");
        assert_eq!(all.len(), 1);
        let by_plan = list_sub_sessions_impl(state.clone(), Some("p1")).await.expect("by plan");
        assert_eq!(by_plan.len(), 1);
        let by_other = list_sub_sessions_impl(state.clone(), Some("p2")).await.expect("by other");
        assert_eq!(by_other.len(), 0);
    }

    /// 2. update 走 SQLite, status 跟 output 持久化, get 拿回最新值.
    #[tokio::test]
    async fn update_sub_session_persists_status_and_output() {
        let state = RuntimeState::new_in_memory_with_config(ResolvedConfig::default())
            .await
            .expect("new state");
        create_sub_session_impl(
            state.clone(),
            SubSessionInput {
                id: "sub_p1_t1".into(),
                plan_id: "p1".into(),
                parent_session_id: "s1".into(),
                task_id: "t1".into(),
                role: "executor".into(),
            },
        )
        .await
        .expect("create");
        update_sub_session_impl(
            state.clone(),
            "sub_p1_t1",
            SubSessionStatus::Done,
            Some("hello from sub session"),
        )
        .await
        .expect("update");
        let got = get_sub_session_impl(state.clone(), "sub_p1_t1").await.expect("get");
        assert_eq!(got.status, "done");
        assert_eq!(got.output.as_deref(), Some("hello from sub session"));
        assert!(got.ended_at.is_some(), "ended_at should be set after update");
    }

    /// 3. get 不存在 → NotFound (前端点击"打开子会话"拿到空 id 的兜底).
    #[tokio::test]
    async fn get_sub_session_missing_returns_not_found() {
        let state = RuntimeState::new_in_memory_with_config(ResolvedConfig::default())
            .await
            .expect("new state");
        let err = get_sub_session_impl(state.clone(), "nonexistent").await.unwrap_err();
        assert!(matches!(err, RuntimeApiError::NotFound(_)), "got {err:?}");
    }

    /// 4. update 不存在 → Store error (避免静默 0 affected).
    #[tokio::test]
    async fn update_sub_session_missing_returns_store_error() {
        let state = RuntimeState::new_in_memory_with_config(ResolvedConfig::default())
            .await
            .expect("new state");
        let err = update_sub_session_impl(state.clone(), "nonexistent", SubSessionStatus::Done, None)
            .await
            .unwrap_err();
        assert!(matches!(err, RuntimeApiError::Internal(_)), "got {err:?}");
    }
}

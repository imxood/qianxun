// qianxun-runtime/src/api/plans.rs
// create_plan / list_plans — 简单 in-memory HashMap 存储.
//
// 业务简化 (sub-task #3 范围):
//   - Plan 持久化不在 sub-task #3 范围 (后续 sub-task 接 store + 完整 contract)
//   - in-memory HashMap<PlanId, PlanInfo> + Mutex, RuntimeState drop 时一起丢
//   - 字段只存 PlanInfo (id / session_id / name / status / timestamps)
//   - contract (tasks / assigned_to / verify_prompt) 留给后续 sub-task
//
// 业务上 create_plan 一定返 Running 状态, list_plans 返所有 (不过滤).
// 取消由 cancel_session 统一处理 (跟现有 daemon 一致, 后续加 plan.cancel 单独接口).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::api::error::{RuntimeApiError, RuntimeApiResult};
use crate::api::types::{PlanInfo, PlanInput, PlanStatus};
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

    // 2. 构造 PlanInfo
    let now = Utc::now();
    let plan = PlanInfo {
        id: format!(
            "plan_{}",
            now.format("%Y%m%d_%H%M%S_%6f"),
        ),
        session_id: input.session_id.clone(),
        name: input.name.clone(),
        status: PlanStatus::Running,
        started_at: now.to_rfc3339(),
        ended_at: None,
    };

    // 3. 写入 in-memory store
    let mut plans = state
        .plans
        .lock()
        .expect("PlanStore lock poisoned");
    plans.insert(plan.id.clone(), plan.clone());

    tracing::info!(
        "[api] create_plan: id={} session_id={} name={}",
        plan.id,
        plan.session_id,
        plan.name
    );
    Ok(plan)
}

/// list_plans 业务实现.
pub async fn list_plans_impl(state: Arc<RuntimeState>) -> RuntimeApiResult<Vec<PlanInfo>> {
    let plans = state.plans.lock().expect("PlanStore lock poisoned");
    Ok(plans.values().cloned().collect())
}

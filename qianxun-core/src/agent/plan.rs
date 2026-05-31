use crate::tools::ToolCategoryFilter;
use serde::Serialize;

/// 生成的执行计划。
#[derive(Debug, Clone, Serialize)]
pub struct PlanResult {
    pub rationale: String,
    pub steps: Vec<PlanStep>,
    pub risks: Vec<String>,
}

/// 计划中的一个步骤。
#[derive(Debug, Clone, Serialize)]
pub struct PlanStep {
    pub id: u32,
    pub description: String,
    pub files: Vec<String>,
    pub depends_on: Vec<u32>,
    pub estimated_effort: Effort,
}

#[derive(Debug, Clone, Serialize)]
pub enum Effort {
    Small,
    Medium,
    Large,
}

/// Plan-and-Execute 状态机状态。
#[derive(Debug, Clone, PartialEq)]
pub enum PlanState {
    Planning,
    WaitingApproval,
    Executing,
    Completed,
}

/// 用户决策。
#[derive(Debug, Clone, PartialEq)]
pub enum UserDecision {
    Execute,
    Cancel,
    Edit,
}

/// 获取计划阶段（Phase 1）的工具过滤器。
/// 只允许读、搜索和思考工具。
pub fn plan_phase_filter() -> ToolCategoryFilter {
    ToolCategoryFilter::read_only()
}

/// 获取执行阶段（Phase 2）的工具过滤器。
/// 全部工具可用。
pub fn execute_phase_filter() -> ToolCategoryFilter {
    ToolCategoryFilter::all()
}

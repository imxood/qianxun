use std::collections::HashMap;

/// 工作流模板 ID。
pub type WorkflowTemplateId = String;

/// 工作流模板。
#[derive(Debug, Clone)]
pub struct WorkflowTemplate {
    pub id: WorkflowTemplateId,
    pub name: String,
    pub description: String,
    pub stages: Vec<WorkflowStage>,
}

/// 工作流阶段。
#[derive(Debug, Clone)]
pub struct WorkflowStage {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub allowed_tools: crate::tools::ToolCategoryFilter,
    pub exit_marker: String,
}

/// 阶段执行结果。
#[derive(Debug, Clone)]
pub struct StageResult {
    pub stage_name: String,
    pub summary: String,
    pub tool_calls: Vec<String>,
    pub llm_rounds: u32,
}

/// Workflow 状态。
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowState {
    Running,
    StageCompleted,
    Completed,
}

/// Workflow 管理器。
pub struct WorkflowManager {
    builtin: HashMap<WorkflowTemplateId, WorkflowTemplate>,
}

impl WorkflowManager {
    pub fn new() -> Self {
        let mut builtin = HashMap::new();
        builtin.insert("code-review".into(), Self::builtin_code_review());
        builtin.insert("bug-fix".into(), Self::builtin_bug_fix());
        builtin.insert("release".into(), Self::builtin_release());
        builtin.insert("refactor".into(), Self::builtin_refactor());
        Self { builtin }
    }

    /// 获取模板。
    pub fn get(&self, id: &str) -> Option<&WorkflowTemplate> {
        self.builtin.get(id)
    }

    /// 列出所有模板。
    pub fn list(&self) -> Vec<&WorkflowTemplate> {
        let mut templates: Vec<_> = self.builtin.values().collect();
        templates.sort_by_key(|t| &t.id);
        templates
    }

    /// 代码审查模板（只读）。
    fn builtin_code_review() -> WorkflowTemplate {
        WorkflowTemplate {
            id: "code-review".into(),
            name: "代码审查".into(),
            description: "对当前变更进行逐文件代码审查".into(),
            stages: vec![
                WorkflowStage {
                    name: "analyze".into(),
                    description: "分析变更范围和目的".into(),
                    instructions: "阅读所有变更文件，理解改动的范围和目的。列出每个文件的变更要点。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::read_only(),
                    exit_marker: "## Stage Complete: analyze".into(),
                },
                WorkflowStage {
                    name: "review".into(),
                    description: "逐行审查代码".into(),
                    instructions: "对每个变更文件进行逐行审查，检查逻辑正确性、边界情况和代码风格。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::read_only(),
                    exit_marker: "## Stage Complete: review".into(),
                },
                WorkflowStage {
                    name: "summarize".into(),
                    description: "汇总审查结果".into(),
                    instructions: "汇总发现的问题，给出总体评估和修改建议。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::read_only(),
                    exit_marker: "## Stage Complete: summarize".into(),
                },
            ],
        }
    }

    /// Bug 修复模板。
    fn builtin_bug_fix() -> WorkflowTemplate {
        WorkflowTemplate {
            id: "bug-fix".into(),
            name: "Bug 修复".into(),
            description: "定位并修复 Bug".into(),
            stages: vec![
                WorkflowStage {
                    name: "reproduce".into(),
                    description: "复现问题".into(),
                    instructions: "理解 Bug 描述，通过阅读代码和日志复现问题。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::read_only(),
                    exit_marker: "## Stage Complete: reproduce".into(),
                },
                WorkflowStage {
                    name: "diagnose".into(),
                    description: "根因分析".into(),
                    instructions: "定位根因，分析影响范围。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::read_only(),
                    exit_marker: "## Stage Complete: diagnose".into(),
                },
                WorkflowStage {
                    name: "fix".into(),
                    description: "实施修复".into(),
                    instructions: "根据分析结果实施修复。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: fix".into(),
                },
                WorkflowStage {
                    name: "verify".into(),
                    description: "验证修复".into(),
                    instructions: "编译并运行测试验证修复正确。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: verify".into(),
                },
            ],
        }
    }

    /// 发布模板。
    fn builtin_release() -> WorkflowTemplate {
        WorkflowTemplate {
            id: "release".into(),
            name: "发布".into(),
            description: "执行发布流程".into(),
            stages: vec![
                WorkflowStage {
                    name: "check".into(),
                    description: "发布前检查".into(),
                    instructions: "检查当前状态：未提交变更、测试状态、版本号。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: check".into(),
                },
                WorkflowStage {
                    name: "prepare".into(),
                    description: "发布准备".into(),
                    instructions: "更新版本号、生成 changelog。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: prepare".into(),
                },
                WorkflowStage {
                    name: "build".into(),
                    description: "构建".into(),
                    instructions: "编译并运行测试。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: build".into(),
                },
                WorkflowStage {
                    name: "finalize".into(),
                    description: "完成发布".into(),
                    instructions: "git tag、commit。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: finalize".into(),
                },
            ],
        }
    }

    /// 重构模板。
    fn builtin_refactor() -> WorkflowTemplate {
        WorkflowTemplate {
            id: "refactor".into(),
            name: "重构".into(),
            description: "代码重构".into(),
            stages: vec![
                WorkflowStage {
                    name: "analyze".into(),
                    description: "分析依赖关系".into(),
                    instructions: "分析依赖关系，确定重构范围和方案。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::read_only(),
                    exit_marker: "## Stage Complete: analyze".into(),
                },
                WorkflowStage {
                    name: "plan".into(),
                    description: "制定重构计划".into(),
                    instructions: "制定具体步骤（文件拆分/合并/重命名）。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::read_only(),
                    exit_marker: "## Stage Complete: plan".into(),
                },
                WorkflowStage {
                    name: "execute".into(),
                    description: "执行重构".into(),
                    instructions: "逐文件实施重构。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: execute".into(),
                },
                WorkflowStage {
                    name: "verify".into(),
                    description: "验证重构".into(),
                    instructions: "编译验证、运行测试。".into(),
                    allowed_tools: crate::tools::ToolCategoryFilter::all(),
                    exit_marker: "## Stage Complete: verify".into(),
                },
            ],
        }
    }
}

impl Default for WorkflowManager {
    fn default() -> Self {
        Self::new()
    }
}

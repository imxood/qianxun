/// Reflective 模式状态机状态。
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectState {
    /// Phase 1: 标准 React 循环
    Reacting,
    /// Phase 2: 自检
    Reviewing,
    /// Phase 3: 修正中
    Revising,
    /// 完成
    Completed,
}

/// 自检结果。
#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub issues: Vec<String>,
    pub confidence: u8,
}

impl ReviewResult {
    /// 是否通过了自检（无 issue 或 confidence 足够高）。
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty() || self.confidence >= 8
    }
}

/// 最大自检轮次。
pub const MAX_REVIEW_ROUNDS: u32 = 2;

/// 判断本轮是否需要自检。
///
/// 仅当有 write_file/edit_file/execute_command 等修改操作时才触发。
pub fn should_self_review(tool_names: &[&str]) -> bool {
    tool_names.iter().any(|t| {
        matches!(
            *t,
            "write_file" | "edit_file" | "execute_command" | "delete_file"
        )
    })
}

/// 构建审查 prompt。
pub fn build_review_prompt(
    original_request: &str,
    tool_calls: &[(&str, &str)], // (tool_name, summary)
) -> String {
    let tool_summary: String = tool_calls
        .iter()
        .map(|(name, summary)| format!("- {name}: {summary}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut p = String::new();
    p.push_str("## Self-Review\n\n");
    p.push_str("请审查你刚刚完成的修改。\n\n");
    p.push_str("### 用户的需求\n");
    p.push_str(original_request);
    p.push_str("\n\n### 你执行的工具调用\n");
    p.push_str(&tool_summary);
    p.push_str("\n\n### 审查清单\n");
    p.push_str("1. 是否完整实现了用户需求？有没有遗漏的部分？\n");
    p.push_str("2. 引入的变更是否可能破坏现有功能？\n");
    p.push_str("3. 代码风格是否与项目现有风格一致？\n");
    p.push_str("4. 有没有明显的错误（语法、类型、逻辑）？\n");
    p.push_str("5. 是否处理了边界情况和错误？\n\n");
    p.push_str("### 输出格式\n");
    p.push_str("如果一切正常，输出 \"[Review: OK]\"。\n");
    p.push_str("如果发现问题，输出 \"[Review: Issues]\" 并列出具体问题。\n\n");
    p.push_str("## 审查结果\n");
    p
}

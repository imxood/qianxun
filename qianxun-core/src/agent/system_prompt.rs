pub const BASE_PROMPT: &str = r#"你是千寻（Qianxun），一个高效的 AI 编程助手。

你通过一系列工具来完成用户的任务。每次工具调用后，等待结果再决定下一步。

## 核心原则
- 结论优先，然后展开依据
- 代码优先，解释为辅
- 每次只做一件事，不要并行执行无关任务
"#;

pub fn build_system_prompt(
    memory_context: &str,
    skills_catalog: &str,
    custom_instructions: Option<&str>,
) -> String {
    let mut parts = vec![BASE_PROMPT.to_string()];

    if let Some(instructions) = custom_instructions {
        parts.push(format!("\n## 用户指令\n{instructions}\n"));
    }

    if !memory_context.is_empty() {
        parts.push(format!("\n## 上下文记忆\n{memory_context}\n"));
    }

    if !skills_catalog.is_empty() {
        parts.push(format!("\n## 可用技能\n{skills_catalog}\n"));
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_with_all_parts() {
        let prompt = build_system_prompt("记忆内容", "技能目录", Some("自定义指令"));
        assert!(prompt.contains("千寻"));
        assert!(prompt.contains("记忆内容"));
        assert!(prompt.contains("技能目录"));
        assert!(prompt.contains("自定义指令"));
    }

    #[test]
    fn test_build_system_prompt_empty_context() {
        let prompt = build_system_prompt("", "", None);
        assert!(prompt.contains("千寻"));
        assert_eq!(prompt, BASE_PROMPT);
    }
}

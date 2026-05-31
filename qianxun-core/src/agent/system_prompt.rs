pub const BASE_PROMPT: &str = r#"你是千寻（Qianxun），一个高效的 AI 编程助手。

按 分析 → 规划 → 执行 → 总结 四阶段工作。简单任务可快速过，复杂任务自然展开。

### 1. 分析（Analyze）
理解需求 → 读相关文件（read_text_file）→ 搜索关键模式（grep/search）→ 查历史记忆（memory_recall）

### 2. 规划（Plan）
出计划格式：
```
## 计划
目标: ...
步骤:
  1. 做什么 — 原因 / 涉及文件
  2. ...
风险: ...
```
**等待用户确认后再执行**

### 3. 执行（Execute）
- 独立操作可并行（同时读多个文件），依赖操作串行
- 每完成一步 ✅ 标记
- 遇到意外暂停分析
- 修改文件用工具，不在对话中输出代码正文

### 4. 总结（Summarize）
```
## 总结
完成: ...  变更: ...  建议: ...
```

## 原则
- **最小改动** — 不改相邻未故障代码
- **结论优先** — 先给结果，再简要解释
- **仅改计划内的** — 不超范围实现
"#;

pub fn build_system_prompt(
    workspace_context: &str,
    custom_instructions: Option<&str>,
) -> String {
    let mut parts = vec![BASE_PROMPT.to_string()];

    if let Some(instructions) = custom_instructions {
        parts.push(format!("\n## 用户指令\n{instructions}\n"));
    }

    if !workspace_context.is_empty() {
        parts.push(format!("\n{workspace_context}\n"));
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_with_all_parts() {
        let prompt = build_system_prompt("工作区上下文", Some("自定义指令"));
        assert!(prompt.contains("千寻"));
        assert!(prompt.contains("工作区上下文"));
        assert!(prompt.contains("自定义指令"));
    }

    #[test]
    fn test_build_system_prompt_empty_context() {
        let prompt = build_system_prompt("", None);
        assert!(prompt.contains("千寻"));
        assert_eq!(prompt, BASE_PROMPT);
    }
}

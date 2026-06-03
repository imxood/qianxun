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

/// 构建系统提示词。
///
/// - `mode` — 当前模式：`"plan"` 时 LLM 不应调用写工具，`"auto"` 时全部可用
pub fn build_system_prompt(
    workspace_context: &str,
    custom_instructions: Option<&str>,
    mode: &str,
) -> String {
    let mut parts = vec![BASE_PROMPT.to_string()];

    // 注入当前模式指令
    match mode {
        "plan" => parts.push(
            concat!(
                "\n## 当前模式：计划模式\n",
                "当前模式为 **计划模式**，只允许读取、搜索和思考操作。\n",
                "**不要调用 write_file、edit_file、execute_command、delete_file 等写操作工具**。\n",
                "你的任务是分析需求和代码结构，制定执行计划并等待用户确认。\n",
            )
            .to_string(),
        ),
        _ => parts.push("\n## 当前模式：自动模式\n所有工具可用。\n".to_string()),
    }

    if let Some(instructions) = custom_instructions {
        parts.push(format!("\n## 用户指令\n{instructions}\n"));
    }

    if !workspace_context.is_empty() {
        parts.push(format!("\n{workspace_context}\n"));
    }

    parts.join("\n")
}

/// 注入 Kanban 上下文 (v6 §4 模式 3 Worker scope 护栏).
///
/// 加 `[CURRENT_TASK_ID]` 占位符让 LLM 知道当前 task, 防 prompt injection
/// 篡改兄弟任务 (Worker 只能动 assigned task).
pub fn inject_kanban_scope(prompt: &str, task_id: Option<&str>, role_suffix: Option<&str>) -> String {
    let mut out = prompt.to_string();
    if let Some(tid) = task_id {
        out.push_str(&format!("\n\n[CURRENT_TASK_ID]\n{tid}\n"));
    }
    if let Some(role) = role_suffix {
        out.push_str(&format!("\n[ROLE]\n{role}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_with_all_parts() {
        let prompt = build_system_prompt("工作区上下文", Some("自定义指令"), "auto");
        assert!(prompt.contains("千寻"));
        assert!(prompt.contains("工作区上下文"));
        assert!(prompt.contains("自定义指令"));
    }

    #[test]
    fn test_build_system_prompt_empty_context() {
        let prompt = build_system_prompt("", None, "auto");
        assert!(prompt.contains("千寻"));
        assert!(prompt.contains("自动模式"));
    }

    #[test]
    fn test_inject_kanban_scope_with_task_and_role() {
        let base = "base prompt";
        let out = inject_kanban_scope(base, Some("task_abc"), Some("Worker"));
        assert!(out.contains("[CURRENT_TASK_ID]"));
        assert!(out.contains("task_abc"));
        assert!(out.contains("[ROLE]"));
        assert!(out.contains("Worker"));
        assert!(out.contains("base prompt"));
    }

    #[test]
    fn test_inject_kanban_scope_with_neither() {
        let base = "base prompt";
        let out = inject_kanban_scope(base, None, None);
        assert_eq!(out, base, "no scope -> unchanged");
    }

    #[test]
    fn test_inject_kanban_scope_with_task_only() {
        let out = inject_kanban_scope("base", Some("task_x"), None);
        assert!(out.contains("[CURRENT_TASK_ID]"));
        assert!(!out.contains("[ROLE]"));
    }
}

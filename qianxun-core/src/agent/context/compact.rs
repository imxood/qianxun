use crate::agent::message::{ContentBlock, Message};
use crate::agent::context::window::AutoCompactWindow;
use crate::config::ResolvedCompactionConfig;
use crate::provider::types::CompletionRequest;
use crate::provider::LlmProvider;
use crate::types::{LlmError, ThinkingConfig, ToolChoice};

/// L1: Snip — 将旧 tool_result 文本内容替换为 [已清除] 标记。
/// 保留最近 `keep_fresh_turns` 个 assistant 消息的 tool_result 不变。
/// 不改变消息结构（数量、角色），只清空文本。
///
/// 算法：从最新消息反向遍历，统计 assistant 消息数。
/// 超过 keep_fresh_turns 的消息中的 tool_result 块被清除。
pub fn snip_tool_results(messages: &mut [Message], keep_fresh_turns: usize) {
    let mut asst_count = 0usize;
    for msg in messages.iter_mut().rev() {
        if matches!(msg, Message::Assistant { .. }) {
            asst_count += 1;
        }
        if asst_count > keep_fresh_turns {
            for block in msg.content_mut().iter_mut() {
                if block.r#type == "tool_result" {
                    block.text = Some("[已清除]".into());
                    block.is_error = Some(false);
                }
            }
        }
    }
}

/// L2: MicroCompact — 超 TTL 时只保留最近 `keep_count` 个 tool_result。
///
/// 算法：反向遍历，按 tool_result 块计数。前 keep_count 个保留，其余标记清除。
pub fn micro_compact(messages: &mut [Message], keep_count: usize) {
    let mut result_count = 0usize;
    for msg in messages.iter_mut().rev() {
        for block in msg.content_mut().iter_mut() {
            if block.r#type == "tool_result" {
                result_count += 1;
                if result_count > keep_count {
                    block.text = Some("[已清除]".into());
                    block.is_error = Some(false);
                }
            }
        }
    }
}

// ─── LLM 单次调用助手 ─────────────────────────────────────────

/// 发送一次性 LLM 请求并收集完整文本响应（无工具、无 thinking）。
pub async fn llm_complete(
    provider: &dyn LlmProvider,
    system_prompt: &str,
    user_message: &str,
    max_tokens: u64,
) -> Result<String, LlmError> {
    let request = CompletionRequest {
        system: Some(system_prompt.to_string()),
        messages: vec![Message::user(vec![ContentBlock::text(user_message)])],
        tools: vec![],
        tool_choice: ToolChoice::Auto,
        max_tokens: Some(max_tokens),
        temperature: None,
        thinking: ThinkingConfig::Disabled,
        stop_sequences: vec![],
    };

    let mut stream = provider.stream_completion(request).await?;
    let mut text = String::new();
    use futures::StreamExt;
    while let Some(event) = stream.next().await {
        if let crate::provider::types::LlmStreamEvent::Text(t) = event? {
            text.push_str(&t);
        }
    }
    Ok(text)
}

// ─── 旧消息格式化 ─────────────────────────────────────────────

/// 将消息序列化为可读的文本格式（供 LLM 压缩使用）。
fn format_messages_for_prompt(messages: &[Message]) -> String {
    let mut s = String::new();
    for msg in messages {
        let role = msg.role();
        s.push_str(&format!("# {role}\n"));
        for block in msg.content() {
            match block.r#type.as_str() {
                "text" => {
                    if let Some(ref t) = block.text {
                        s.push_str(t);
                        s.push('\n');
                    }
                }
                "tool_use" => {
                    if let Some(ref name) = block.tool_name {
                        s.push_str(&format!("[工具调用: {name}]\n"));
                        if let Some(ref input) = block.input {
                            s.push_str(&format!("{input}\n"));
                        }
                    }
                }
                "tool_result" => {
                    let text = block.text.as_deref().unwrap_or("[已清除]");
                    let preview = if text.len() > 500 {
                        format!("{}...(+{}字符)", &text[..500], text.len() - 500)
                    } else {
                        text.to_string()
                    };
                    let err = if block.is_error.unwrap_or(false) { " [错误]" } else { "" };
                    s.push_str(&format!("[工具结果{err}: {preview}]\n"));
                }
                _ => {}
            }
        }
        s.push('\n');
    }
    s
}

/// 计算替换点：将最早的部分消息压缩为摘要。
/// compress_ratio 控制压缩比例（0.0~1.0）。
/// 始终保留最后一条 user 消息。
fn find_split_point(messages: &[Message], compress_ratio: f64) -> usize {
    if messages.is_empty() {
        return 0;
    }
    let last_user_idx = messages.iter().rposition(|m| matches!(m, Message::User { .. }));
    let by_count = (messages.len() as f64 * compress_ratio) as usize;
    match last_user_idx {
        Some(idx) => by_count.min(idx),
        None => by_count.min(messages.len().saturating_sub(1)),
    }
}

/// 用摘要文本替换 messages[0..split_point) 的内容。
/// 返回被替换的消息数。
fn replace_with_summary(messages: &mut Vec<Message>, split_point: usize, summary: &str) -> usize {
    if split_point == 0 {
        return 0;
    }
    let summary_text = format!(
        "以下是对之前轮次的对话摘要：\n\n{}\n\n---\n对话继续",
        summary.trim()
    );
    messages.drain(..split_point);
    messages.insert(0, Message::user(vec![ContentBlock::text(summary_text)]));
    split_point
}

// ─── L3: Collapse ────────────────────────────────────────────

const COLLAPSE_SYSTEM_PROMPT: &str = "你是一个对话压缩专家。用中文输出精简摘要。";

const COLLAPSE_USER_TEMPLATE: &str = r#"压缩以下 AI 编程助手对话历史。保留关键信息：用户需求、决策理由、文件路径和主要内容。
移除重复内容、冗余思考和调试细节。

只输出压缩后的对话文本，不需要解释。

对话历史：
{old_messages}"#;

/// L3: Collapse — 用 LLM 精简最旧的部分轮次（约 30%）。
pub async fn collapse_messages(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    config: &ResolvedCompactionConfig,
) -> usize {
    let split = find_split_point(messages, 0.3);
    if split < 2 {
        return 0; // Not enough messages to compress
    }

    let old_msgs = &messages[..split];
    let formatted = format_messages_for_prompt(old_msgs);
    let user_msg = COLLAPSE_USER_TEMPLATE.replace("{old_messages}", &formatted);

    match llm_complete(provider, COLLAPSE_SYSTEM_PROMPT, &user_msg, config.max_output_tokens * 2).await {
        Ok(summary) if summary.len() >= 50 => {
            let n = replace_with_summary(messages, split, &summary);
            tracing::info!("[Compact L3] collapsed {n} messages");
            n
        }
        Ok(_) => {
            tracing::warn!("[Compact L3] summary too short, skipped");
            0
        }
        Err(e) => {
            tracing::warn!("[Compact L3] LLM call failed: {e}");
            0
        }
    }
}

// ─── L4: AutoCompact ─────────────────────────────────────────

const AUTOCOMPACT_SYSTEM_PROMPT: &str = "你是一个对话压缩专家。按指令格式输出结构化中文摘要。";

const AUTOCOMPACT_USER_TEMPLATE: &str = r#"分析下面的 AI 编程助手对话历史，输出结构化摘要。

<analysis>
先思考：这份对话中的关键信息是什么？用户的核心需求是什么？
哪些信息是重复的、过时的、或者不重要的？
如何组织摘要才能让 AI 在继续对话时能理解上下文？
</analysis>

<summary>
根据以上分析，输出结构化的中文对话摘要，包括：
1. 项目目标
2. 已完成的工作
3. 关键决策和理由
4. 当前状态和待办事项
5. 文件修改记录（路径 + 变更说明）

以下是需要压缩的对话：
{old_messages}
</summary>"#;

/// L4: AutoCompact — 双段提示词完整摘要，替换旧轮次（约 60%）。
pub async fn auto_compact_messages(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    config: &ResolvedCompactionConfig,
) -> usize {
    let split = find_split_point(messages, 0.6);
    if split < 2 {
        return 0;
    }

    let old_msgs = &messages[..split];
    let formatted = format_messages_for_prompt(old_msgs);
    let user_msg = AUTOCOMPACT_USER_TEMPLATE.replace("{old_messages}", &formatted);

    let response = match llm_complete(provider, AUTOCOMPACT_SYSTEM_PROMPT, &user_msg, config.max_output_tokens * 3).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[Compact L4] LLM call failed: {e}");
            return 0;
        }
    };

    // 解析：优先提取 <summary> 标签中的内容
    let summary = if let Some(start) = response.find("<summary>") {
        let content_start = start + "<summary>".len();
        if let Some(end) = response[content_start..].find("</summary>") {
            &response[content_start..content_start + end]
        } else {
            &response[content_start..]
        }
    } else if response.find("<analysis>").is_some() {
        // If only analysis block found, skip everything after it
        // This typically means the LLM didn't produce a summary
        tracing::warn!("[Compact L4] no <summary> tag found");
        return 0;
    } else {
        response.as_str()
    };

    let summary = summary.trim();
    if summary.len() < 50 {
        tracing::warn!("[Compact L4] summary too short ({} chars), skipped", summary.len());
        return 0;
    }

    let n = replace_with_summary(messages, split, summary);
    tracing::info!("[Compact L4] compressed {n} messages, summary={}chars", summary.len());
    n
}

// ─── 编排函数 ────────────────────────────────────────────────

/// 根据当前水位决定执行 L3 Collapse 或 L4 AutoCompact。
/// 返回被压缩的消息数，0 表示失败。
///
/// 决策链：
/// - ratio >= 95% (Blocked) → 直接 AutoCompact
/// - ratio >= 90% (Danger) → 先 Collapse，不够再 AutoCompact
/// - ratio >= 85% → AutoCompact
pub async fn attempt_compression(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    config: &ResolvedCompactionConfig,
    window: &AutoCompactWindow,
) -> usize {
    let ratio = window.usage_ratio(config.scope);
    let block_ratio = config.block_ratio;
    let collapse_ratio = config.collapse_ratio;

    if ratio >= block_ratio {
        auto_compact_messages(messages, provider, config).await
    } else if ratio >= collapse_ratio {
        let n = collapse_messages(messages, provider, config).await;
        if n > 0 {
            n
        } else {
            auto_compact_messages(messages, provider, config).await
        }
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::message::ContentBlock;

    fn make_tool_result_msg(tool_use_id: &str) -> Message {
        let blocks = vec![ContentBlock::tool_result(tool_use_id.into(), "some result content", false)];
        Message::user(blocks)
    }

    fn make_asst_with_tool_use(tool_use_id: &str) -> Message {
        let blocks = vec![
            ContentBlock::text("using tool..."),
            ContentBlock::tool_use(tool_use_id.into(), "test_tool", serde_json::json!({})),
        ];
        Message::assistant(blocks)
    }

    fn make_text_msg(role: &str) -> Message {
        match role {
            "user" => Message::user(vec![ContentBlock::text("hello")]),
            _ => Message::assistant(vec![ContentBlock::text("ok")]),
        }
    }

    fn count_snipped(msgs: &[Message]) -> usize {
        msgs.iter()
            .flat_map(|m| m.content())
            .filter(|b| b.r#type == "tool_result" && b.text.as_deref() == Some("[已清除]"))
            .count()
    }

    fn count_preserved(msgs: &[Message]) -> usize {
        msgs.iter()
            .flat_map(|m| m.content())
            .filter(|b| b.r#type == "tool_result" && b.text.as_deref() != Some("[已清除]"))
            .count()
    }

    #[test]
    fn test_snip_clears_old_tool_results() {
        let mut msgs = vec![
            make_asst_with_tool_use("a"),
            make_tool_result_msg("a"),
            make_asst_with_tool_use("b"),
            make_tool_result_msg("b"),
            make_asst_with_tool_use("c"),
            make_tool_result_msg("c"),
            make_text_msg("assistant"),
        ];
        // keep_fresh_turns = 2 → 保留最近 2 个 assistant 的 tool_result
        snip_tool_results(&mut msgs, 2);
        assert_eq!(count_snipped(&msgs), 1, "oldest tool_result should be snipped");
        assert_eq!(count_preserved(&msgs), 2, "two most recent tool_results preserved");
    }

    #[test]
    fn test_snip_preserves_message_count() {
        let mut msgs = vec![
            make_asst_with_tool_use("a"),
            make_tool_result_msg("a"),
            make_asst_with_tool_use("b"),
            make_tool_result_msg("b"),
        ];
        let count = msgs.len();
        snip_tool_results(&mut msgs, 1);
        assert_eq!(msgs.len(), count, "message count should not change");
    }

    #[test]
    fn test_snip_empty_or_short() {
        let mut empty: Vec<Message> = vec![];
        snip_tool_results(&mut empty, 3);
        assert!(empty.is_empty());

        let mut single = vec![make_text_msg("user")];
        snip_tool_results(&mut single, 3);
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn test_micro_compact_keeps_n_recent() {
        let mut msgs = Vec::new();
        for i in 0..30 {
            msgs.push(make_asst_with_tool_use(&format!("id_{i}")));
            msgs.push(make_tool_result_msg(&format!("id_{i}")));
        }
        micro_compact(&mut msgs, 10);
        assert_eq!(count_snipped(&msgs), 20, "20 oldest tool_results should be snipped");
        assert_eq!(count_preserved(&msgs), 10, "10 most recent preserved");
    }

    #[test]
    fn test_micro_compact_all_preserved_when_under_limit() {
        let mut msgs = vec![
            make_asst_with_tool_use("a"),
            make_tool_result_msg("a"),
            make_asst_with_tool_use("b"),
            make_tool_result_msg("b"),
        ];
        micro_compact(&mut msgs, 10);
        assert_eq!(count_snipped(&msgs), 0);
        assert_eq!(count_preserved(&msgs), 2);
    }
}

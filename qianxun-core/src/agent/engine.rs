use crate::agent::context::window::CompactZone;
use crate::agent::context::{AutoCompactWindow, compact, normalize};
use crate::agent::conversation::Conversation;
use crate::agent::message::{ContentBlock, Message};
use crate::config::ResolvedCompactionConfig;
use crate::output::OutputSink;
use crate::provider::LlmProvider;
use crate::provider::types::LlmStreamEvent;
use crate::tools::{ToolCategoryFilter, ToolRegistry};
use crate::types::{AgentConfig, StopReason, TokenUsage};
use futures::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Truncate a string for debug logging at char boundary, appending count of truncated chars.
fn trunc(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…(+{}chars)", &s[..end], s.len() - max)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    WaitingLlm,
    ToolExecuting,
    Stopping,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct AgentLoop {
    pub state: AgentState,
    pub turn_count: u32,
    pub retry_count: u32,
    pub config: AgentConfig,
    pub accumulated_usage: TokenUsage,
    pub compact_window: Option<AutoCompactWindow>,
    pub compact_config: Option<ResolvedCompactionConfig>,
}

impl AgentLoop {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            state: AgentState::Idle,
            turn_count: 0,
            retry_count: 0,
            config,
            accumulated_usage: TokenUsage::default(),
            compact_window: None,
            compact_config: None,
        }
    }

    pub fn can_continue(&self) -> bool {
        self.turn_count < self.config.max_turns
    }

    pub fn reset(&mut self) {
        self.state = AgentState::Idle;
        self.turn_count = 0;
        self.retry_count = 0;
        self.accumulated_usage = TokenUsage::default();
        // 保留 compact_window — reset 只清运行时状态，不丢压缩窗口配置
    }
}

// ─── processing_loop ────────────────────────────────────────

pub mod processing_loop {
    use super::*;

    /// 处理用户消息：stream → tool_loop → 输出
    /// - `skills_catalog`: Layer 1 技能目录（注入 system prompt）
    /// - `skill_injections`: Layer 2 技能完整内容（自动/手动匹配后注入）
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_user_message(
        agent: &mut AgentLoop,
        conversation: &mut Conversation,
        provider: &dyn LlmProvider,
        tools: &ToolRegistry,
        tool_filter: ToolCategoryFilter,
        sink: &dyn OutputSink,
        memory_context: &str,
        skills_catalog: &str,
        skill_injections: &str,
        cancel_flag: Arc<AtomicBool>,
    ) {
        agent.state = AgentState::WaitingLlm;
        agent.turn_count += 1;
        tracing::info!("[turn {}] processing user message", agent.turn_count);

        loop {
            if cancel_flag.load(Ordering::SeqCst) {
                sink.on_status("用户已取消").await;
                agent.state = AgentState::Idle;
                return;
            }

            if !agent.can_continue() {
                agent.state = AgentState::Stopping;
                sink.on_turn_finished(&StopReason::MaxTokens, &agent.accumulated_usage)
                    .await;
                return;
            }

            // ── Normalize (always — fixes tool_use/tool_result pairing) ──
            normalize::normalize_messages(conversation.messages_mut());

            // ── Context compression pipeline ──
            if let (Some(ref mut cw), Some(cc)) =
                (agent.compact_window.as_mut(), agent.compact_config.as_ref())
            {
                // Update token tracking
                cw.update(&agent.accumulated_usage);

                // L1: Always snip old tool_results
                compact::snip_tool_results(conversation.messages_mut(), cc.snip_fresh_turns);

                // L2: MicroCompact if time threshold exceeded
                if cw.should_micro_compact() {
                    compact::micro_compact(conversation.messages_mut(), cc.micro_compact_keep);
                }

                // L3/L4: Check zone and attempt compression
                let zone = cw.compute_zone(cc.scope);
                cw.zone = zone;
                match zone {
                    CompactZone::Blocked | CompactZone::Danger => {
                        if cw.is_circuit_broken() {
                            let old_len = conversation.messages().len();
                            conversation.enforce_budget(&[]).await;
                            let removed = old_len - conversation.messages().len();
                            if removed > 0 {
                                sink.on_status(&format!("熔断降级：已截断 {removed} 条最旧消息"))
                                    .await;
                            }
                        } else {
                            sink.on_status("上下文使用率较高，正在压缩...").await;
                            let n = compact::attempt_compression(
                                conversation.messages_mut(),
                                provider,
                                cc,
                                cw,
                            )
                            .await;
                            if n > 0 {
                                cw.record_compaction();
                                sink.on_status(&format!("压缩完成，释放 {n} 条消息")).await;
                            } else {
                                let broken = cw.record_failure();
                                if broken {
                                    sink.on_status("压缩连续失败，已熔断").await;
                                }
                            }
                        }
                    }
                    CompactZone::Warning => {
                        let pct = cw.usage_ratio(cc.scope) * 100.0;
                        tracing::info!("上下文使用率 {pct:.1}%");
                    }
                    CompactZone::Safe => {}
                }
            }

            // ── build → stream ──
            let request = {
                let defs = tools.definitions();
                conversation.build_request(
                    &defs,
                    memory_context,
                    skills_catalog,
                    skill_injections,
                    &agent.config,
                )
            };

            tracing::info!(
                "发送 LLM 请求: {} 条消息, {} 个工具定义",
                request.messages.len(),
                request.tools.len(),
            );
            tracing::debug!(
                "LLM 请求详情: system_len={}, messages=[{}], tools=[{}]",
                request.system.as_ref().map_or(0, |s| s.len()),
                request
                    .messages
                    .iter()
                    .map(|m| format!(
                        "{}:{}",
                        m.role(),
                        m.content()
                            .iter()
                            .map(|b| b.r#type.as_str())
                            .collect::<Vec<_>>()
                            .join(","),
                    ))
                    .collect::<Vec<_>>()
                    .join(" | "),
                request
                    .tools
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            if let Some(sys) = &request.system {
                tracing::debug!("=== System Prompt ===\n{}", trunc(sys, 2000));
            }

            // ── stream_completion with rate-limit retry ──
            let mut stream = loop {
                match provider.stream_completion(request.clone()).await {
                    Ok(s) => break s,
                    Err(e) => {
                        if let crate::types::LlmError::RateLimitExceeded { retry_after, .. } = &e {
                            if agent.retry_count < agent.config.max_retries {
                                agent.retry_count += 1;
                                let wait = retry_after.unwrap_or(std::time::Duration::from_secs(5));
                                sink.on_status(&format!(
                                    "速率受限，{}s 后重试 ({}/{})",
                                    wait.as_secs(),
                                    agent.retry_count,
                                    agent.config.max_retries
                                ))
                                .await;
                                tokio::time::sleep(wait).await;
                                continue;
                            }
                        }
                        tracing::error!("LLM stream start failed: {e}");
                        sink.on_error(&e).await;
                        agent.state = AgentState::Error(e.to_string());
                        return;
                    }
                }
            };

            tracing::debug!("LLM stream started, consuming...");

            // ── consume the stream ──
            let mut response_text = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut thinking_blocks: Vec<(String, Option<String>)> = Vec::new();
            #[allow(unused_assignments)]
            let mut current_thinking_text = String::new();
            let mut thinking_logged_len = 0;
            #[allow(unused_assignments)]
            let mut current_thinking_sig: Option<String> = None;
            let mut received_stop = false;

            while let Some(event) = stream.next().await {
                if cancel_flag.load(Ordering::SeqCst) {
                    break;
                }
                match event {
                    Ok(LlmStreamEvent::Text(text)) => {
                        response_text.push_str(&text);
                        sink.on_text(&text).await;
                    }
                    Ok(LlmStreamEvent::ToolCall {
                        id,
                        tool_name,
                        arguments,
                    }) => {
                        sink.on_tool_call(&id, &tool_name, &arguments).await;
                        tool_calls.push((id, tool_name, arguments));
                    }
                    Ok(LlmStreamEvent::UsageUpdate(usage)) => {
                        // UsageUpdate from DeepSeek is cumulative (message_start has
                        // input=500, message_delta has input=2000).  Replace — don't add.
                        agent.accumulated_usage = usage.clone();
                        if let Some(ref mut cw) = agent.compact_window {
                            cw.update(&agent.accumulated_usage);
                        }
                        sink.on_token_usage(&agent.accumulated_usage).await;
                    }
                    Ok(LlmStreamEvent::Stop(reason)) => {
                        received_stop = true;
                        if reason == StopReason::ToolUse && !tool_calls.is_empty() {
                            build_turn(conversation, &response_text, &tool_calls, &thinking_blocks);
                            // Record assistant time for L2 TTL
                            if let Some(ref mut cw) = agent.compact_window {
                                cw.set_last_assistant_time(std::time::Instant::now());
                            }

                            // execute tools
                            sink.on_thinking_flush().await;
                            agent.state = AgentState::ToolExecuting;
                            let tool_names: Vec<&str> =
                                tool_calls.iter().map(|(_, n, _)| n.as_str()).collect();
                            tracing::info!(
                                "LLM 返回工具调用: {} 个 — {:?}",
                                tool_calls.len(),
                                tool_names,
                            );
                            if !response_text.is_empty() {
                                tracing::debug!(
                                    "=== LLM 回复文本 (tool_use 前) ===\n{}",
                                    trunc(&response_text, 5000)
                                );
                            }
                            for (_, name, args) in &tool_calls {
                                tracing::debug!(
                                    "  工具 {name}: args={}",
                                    trunc(&serde_json::to_string(args).unwrap_or_default(), 2000),
                                );
                            }
                            let mut results = Vec::new();
                            for (id, name, args) in &tool_calls {
                                sink.on_status(&format!("执行工具: {name}")).await;
                                tracing::info!(
                                    "[tool] execute: {name} ({id}) args={}",
                                    serde_json::to_string(args).unwrap_or_default(),
                                );
                                match tools
                                    .execute_async_with_filter(name, args.clone(), &tool_filter)
                                    .await
                                {
                                    Ok(output) => {
                                        tracing::info!(
                                            "[tool] result: {name} ({id}) is_error={} len={}",
                                            output.is_error,
                                            output.content.len(),
                                        );
                                        tracing::debug!(
                                            "[tool] result content: {name} ({id})\n{}",
                                            trunc(&output.content, 5000),
                                        );
                                        results.push((id.clone(), output.content, output.is_error));
                                    }
                                    Err(e) => {
                                        tracing::error!("[tool] error: {name} ({id}): {e}");
                                        sink.on_status(&format!("工具执行失败: {name} — {e}"))
                                            .await;
                                        results.push((id.clone(), format!("Error: {e}"), true));
                                    }
                                }
                            }

                            sink.on_status("工具执行完成，继续请求 LLM...").await;

                            // Push tool results and loop back to LLM
                            let result_count = results.len();
                            let result_blocks: Vec<ContentBlock> = results
                                .into_iter()
                                .map(|(id, content, is_error)| {
                                    ContentBlock::tool_result(id, content, is_error)
                                })
                                .collect();
                            conversation.push_message(Message::user(result_blocks));
                            // turn_count is incremented at the top of this function once per
                            // user→assistant exchange.  Tool round-trips within one exchange
                            // should NOT consume additional turns — the loop naturally
                            // terminates when the LLM stops requesting tools.
                            tracing::info!(
                                "工具执行完成，回送 {} 个结果到 LLM (turn {})",
                                result_count,
                                agent.turn_count,
                            );
                            break; // exit stream loop → outer loop continues
                        } else {
                            // End of turn
                            let turn_usage = agent.accumulated_usage.clone();
                            build_turn(conversation, &response_text, &tool_calls, &thinking_blocks);
                            // Record assistant time for L2 TTL
                            if let Some(ref mut cw) = agent.compact_window {
                                cw.set_last_assistant_time(std::time::Instant::now());
                            }

                            agent.state = AgentState::Idle;
                            tracing::info!(
                                "LLM 回复完成: reason={reason:?}, text={}chars, tool_calls={}, thinking={}blocks",
                                response_text.len(),
                                tool_calls.len(),
                                thinking_blocks.len(),
                            );
                            if !response_text.is_empty() {
                                tracing::debug!(
                                    "=== LLM 回复文本 ===\n{}",
                                    trunc(&response_text, 10000)
                                );
                            }
                            for (i, (t, _)) in thinking_blocks.iter().enumerate() {
                                tracing::debug!("=== thinking block {i} ===\n{}", trunc(t, 5000));
                            }
                            sink.on_turn_finished(&reason, &turn_usage).await;
                            return;
                        }
                    }
                    Ok(LlmStreamEvent::Thinking { text, signature }) => {
                        if !text.is_empty() {
                            current_thinking_text.push_str(&text);
                            sink.on_thinking(&text).await;

                            // 积累约 1000 字符后输出一条 [thinking] 日志
                            if current_thinking_text.len() - thinking_logged_len >= 1000 {
                                let chunk = &current_thinking_text[thinking_logged_len..];
                                tracing::debug!("[thinking] {}", trunc(chunk, 2000));
                                thinking_logged_len = current_thinking_text.len();
                            }
                        }
                        if let Some(sig) = signature {
                            // 输出块内剩余未日志的 thinking 内容
                            if thinking_logged_len < current_thinking_text.len() {
                                let chunk = &current_thinking_text[thinking_logged_len..];
                                tracing::debug!("[thinking] {}", trunc(chunk, 2000));
                            }
                            tracing::debug!(
                                "[thinking block complete] {} chars",
                                current_thinking_text.len()
                            );
                            sink.on_thinking_flush().await;
                            current_thinking_sig = Some(sig);
                            thinking_blocks.push((
                                std::mem::take(&mut current_thinking_text),
                                current_thinking_sig.take(),
                            ));
                            thinking_logged_len = 0;
                        }
                    }
                    Err(e) => {
                        sink.on_error(&e).await;
                        agent.state = AgentState::Error(e.to_string());
                        return;
                    }
                }
            }
            // ── end while stream ──

            // ── check cancellation after stream ──
            if cancel_flag.load(Ordering::SeqCst) {
                if !response_text.is_empty() || !tool_calls.is_empty() {
                    build_turn(conversation, &response_text, &tool_calls, &thinking_blocks);
                }
                sink.on_turn_finished(&StopReason::Cancelled, &agent.accumulated_usage)
                    .await;
                agent.state = AgentState::Idle;
                return;
            }

            if !received_stop {
                // Stream ended without a Stop event (e.g. network cut, non-SSE response).
                // End the turn to avoid busy-looping.
                tracing::warn!("LLM stream ended without Stop event — forcing end_turn");
                if !response_text.is_empty() || !tool_calls.is_empty() {
                    build_turn(conversation, &response_text, &tool_calls, &thinking_blocks);
                }
                sink.on_turn_finished(&StopReason::Error, &agent.accumulated_usage)
                    .await;
                agent.state = AgentState::Error("no stop signal".into());
                return;
            }
        }
        // ── end loop ──
    }

    /// Append assistant message to conversation, including tool_use blocks.
    fn build_turn(
        conversation: &mut Conversation,
        response_text: &str,
        tool_calls: &[(String, String, serde_json::Value)],
        thinking: &[(String, Option<String>)],
    ) {
        let mut blocks = Vec::new();
        for (text, sig) in thinking {
            blocks.push(ContentBlock::thinking(text, sig.clone()));
        }
        if !response_text.is_empty() {
            blocks.push(ContentBlock::text(response_text));
        }
        for (id, name, input) in tool_calls {
            blocks.push(ContentBlock::tool_use(
                id.clone(),
                name.clone(),
                input.clone(),
            ));
        }
        if !blocks.is_empty() {
            conversation.push_message(Message::assistant(blocks));
        }
    }
}

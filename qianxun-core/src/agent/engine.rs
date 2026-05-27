use crate::agent::conversation::Conversation;
use crate::agent::message::{ContentBlock, Message};
use crate::output::OutputSink;
use crate::provider::types::LlmStreamEvent;
use crate::provider::LlmProvider;
use crate::tools::ToolRegistry;
use crate::types::{AgentConfig, StopReason, TokenUsage};
use futures::StreamExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    WaitingLlm,
    ToolExecuting,
    Stopping,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTransition {
    ContinueLlm,
    EndTurn,
    Retry,
    Error(String),
}

#[derive(Debug)]
pub struct AgentLoop {
    pub state: AgentState,
    pub turn_count: u32,
    pub retry_count: u32,
    pub config: AgentConfig,
    pub accumulated_usage: TokenUsage,
}

impl AgentLoop {
    pub fn new(config: AgentConfig) -> Self {
        Self {
            state: AgentState::Idle,
            turn_count: 0,
            retry_count: 0,
            config,
            accumulated_usage: TokenUsage::default(),
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
    }
}

// ─── processing_loop ────────────────────────────────────────

pub mod processing_loop {
    use super::*;

    /// 处理用户消息：stream → tool_loop → 输出
    pub async fn handle_user_message(
        agent: &mut AgentLoop,
        conversation: &mut Conversation,
        provider: &dyn LlmProvider,
        tools: &ToolRegistry,
        sink: &dyn OutputSink,
        memory_context: &str,
        skills_catalog: &str,
    ) {
        agent.state = AgentState::WaitingLlm;
        agent.turn_count += 1;

        loop {
            if !agent.can_continue() {
                agent.state = AgentState::Stopping;
                sink.on_turn_finished(
                    &StopReason::MaxTokens,
                    &agent.accumulated_usage,
                )
                .await;
                return;
            }

            // ── budget → build → stream ──
            conversation.enforce_budget(&tools.definitions()).await;

            let request = {
                let defs = tools.definitions();
                conversation.build_request(&defs, memory_context, skills_catalog, &agent.config)
            };

            let mut stream = match provider.stream_completion(request).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("LLM stream start failed: {e}");
                    sink.on_error(&e).await;
                    agent.state = AgentState::Error(e.to_string());
                    return;
                }
            };

            tracing::debug!("LLM stream started, consuming...");

            // ── consume the stream ──
            let mut response_text = String::new();
            let mut text_buffer = String::new();
            let mut thinking_buffer = String::new();
            let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut thinking_blocks: Vec<(String, Option<String>)> = Vec::new();
            #[allow(unused_assignments)]
            let mut current_thinking_text = String::new();
            #[allow(unused_assignments)]
            let mut current_thinking_sig: Option<String> = None;
            let mut received_stop = false;

            while let Some(event) = stream.next().await {
                match event {
                    Ok(LlmStreamEvent::Text(text)) => {
                        response_text.push_str(&text);
                        text_buffer.push_str(&text);
                        if text_buffer.len() >= 50 {
                            sink.on_text(&text_buffer).await;
                            text_buffer.clear();
                        }
                    }
                    Ok(LlmStreamEvent::ToolCall {
                        id,
                        tool_name,
                        arguments,
                    }) => {
                        flush_text(sink, &mut text_buffer).await;
                        flush_thinking(sink, &mut thinking_buffer).await;
                        sink.on_tool_call(&id, &tool_name, &arguments).await;
                        tool_calls.push((id, tool_name, arguments));
                    }
                    Ok(LlmStreamEvent::UsageUpdate(usage)) => {
                        agent.accumulated_usage = agent
                            .accumulated_usage
                            .clone() + usage;
                        sink.on_token_usage(&agent.accumulated_usage).await;
                    }
                    Ok(LlmStreamEvent::Stop(reason)) => {
                        received_stop = true;
                        if reason == StopReason::ToolUse && !tool_calls.is_empty() {
                            build_turn(
                                conversation,
                                &response_text,
                                &tool_calls,
                                &thinking_blocks,
                            );

                            // execute tools
                            agent.state = AgentState::ToolExecuting;
                            let mut results = Vec::new();
                            for (id, name, args) in &tool_calls {
                                match tools.execute_async(name, args.clone()).await {
                                    Ok(output) => {
                                        results.push((
                                            id.clone(),
                                            output.content,
                                            output.is_error,
                                        ));
                                    }
                                    Err(e) => {
                                        sink.on_error(&crate::types::LlmError::ApiError {
                                            provider: "tool".into(),
                                            status: 0,
                                            message: e.to_string(),
                                        })
                                        .await;
                                        results.push((id.clone(), format!("Error: {e}"), true));
                                    }
                                }
                            }

                            flush_text(sink, &mut text_buffer).await;
                            flush_thinking(sink, &mut thinking_buffer).await;

                            // Push tool results and loop back to LLM
                            let result_blocks: Vec<ContentBlock> = results
                                .into_iter()
                                .map(|(id, content, is_error)| {
                                    ContentBlock::tool_result(id, content, is_error)
                                })
                                .collect();
                            conversation.push_message(Message::user(result_blocks));
                            agent.turn_count += 1;
                            break; // exit stream loop → outer loop continues
                        } else {
                            // End of turn
                            let turn_usage = agent.accumulated_usage.clone();
                            build_turn(
                                conversation,
                                &response_text,
                                &tool_calls,
                                &thinking_blocks,
                            );

                            agent.state = AgentState::Idle;
                            flush_text(sink, &mut text_buffer).await;
                            flush_thinking(sink, &mut thinking_buffer).await;
                            sink.on_turn_finished(&reason, &turn_usage).await;
                            return;
                        }
                    }
                    Ok(LlmStreamEvent::Thinking { text, signature }) => {
                        if !text.is_empty() {
                            current_thinking_text.push_str(&text);
                            thinking_buffer.push_str(&text);
                            if thinking_buffer.len() >= 50 {
                                sink.on_thinking(&thinking_buffer).await;
                                thinking_buffer.clear();
                            }
                        }
                        if let Some(sig) = signature {
                            flush_thinking(sink, &mut thinking_buffer).await;
                            current_thinking_sig = Some(sig);
                            thinking_blocks.push((
                                std::mem::take(&mut current_thinking_text),
                                current_thinking_sig.take(),
                            ));
                        }
                    }
                    Err(e) => {
                        flush_text(sink, &mut text_buffer).await;
                        flush_thinking(sink, &mut thinking_buffer).await;
                        sink.on_error(&e).await;
                        agent.state = AgentState::Error(e.to_string());
                        return;
                    }
                }
            }
            // ── end while stream ──

            if !received_stop {
                // Stream ended without a Stop event (e.g. network cut, non-SSE response).
                // End the turn to avoid busy-looping.
                tracing::warn!("LLM stream ended without Stop event — forcing end_turn");
                if !response_text.is_empty() || !tool_calls.is_empty() {
                    build_turn(conversation, &response_text, &tool_calls, &thinking_blocks);
                }
                flush_text(sink, &mut text_buffer).await;
                flush_thinking(sink, &mut thinking_buffer).await;
                sink.on_turn_finished(
                    &StopReason::Error,
                    &agent.accumulated_usage,
                )
                .await;
                agent.state = AgentState::Error("no stop signal".into());
                return;
            }
        }
        // ── end loop ──
    }

    /// Flush buffered text to the sink.
    async fn flush_text(sink: &dyn OutputSink, buf: &mut String) {
        if !buf.is_empty() {
            sink.on_text(buf).await;
            buf.clear();
        }
    }

    /// Flush buffered thinking to the sink.
    async fn flush_thinking(sink: &dyn OutputSink, buf: &mut String) {
        if !buf.is_empty() {
            sink.on_thinking(buf).await;
            buf.clear();
        }
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

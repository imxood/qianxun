use crate::agent::context::window::CompactZone;
use crate::agent::context::{AutoCompactWindow, compact, normalize};
use crate::agent::conversation::Conversation;
use crate::agent::message::{ContentBlock, Message};
use crate::config::ResolvedCompactionConfig;
use crate::hooks::{HookContext, HookEvent, HookRegistry, HookResult};
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
    /// 缺口 12 移除: retry 决策已迁到 ProviderStack (Layer 1 + Layer 2 失败转移).
    /// 旧 `retry_count: u32` 字段已删除, 业务不再持有 per-call 重试状态.
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
    /// - `hooks`: 可选 HookRegistry. None 时跳过所有 dispatch (旧调用方不破坏).
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
        hooks: Option<&HookRegistry>,
    ) {
        agent.state = AgentState::WaitingLlm;
        agent.turn_count += 1;
        tracing::info!("[turn {}] processing user message", agent.turn_count);

        // ── 缺口 01 集成: 入口 dispatch (BeforeLoopIter, 触发 Continuation tier).
        //     best-effort, hook 错误不阻塞主流程 (HookResult::Block 时记录后继续).
        if let Some(h) = hooks {
            let mut ctx = HookContext::default();
            let _ = h.dispatch(HookEvent::BeforeLoopIter, &mut ctx).await;
        }

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

            // ── stream_completion (Layer 1 + Layer 2 失败转移已迁到 ProviderStack) ──
            // 缺口 12: 旧版 RateLimit retry 循环 (24 行) 已删. 失败时 ProviderStack
            // 内部已尝试 retry 同 provider + 切 fallback, 仍失败返 LlmError,
            // 透传到 sink.on_error 让上层走错误路径.
            let mut stream = match provider.stream_completion(request).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("[engine] LLM stream start failed: {e}");
                    sink.on_error(&e).await;
                    agent.state = AgentState::Error(e.to_string());
                    return;
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
                                // ── 缺口 01 集成: 工具调用前 dispatch (BeforeToolCall, 触发 ToolGuard+Transform tier).
                                //     HookResult::Block 时记录并跳过该工具 (best-effort, 不阻塞主流程).
                                if let Some(h) = hooks {
                                    let mut hook_ctx = HookContext {
                                        tool_name: Some(name.clone()),
                                        tool_args: Some(args.clone()),
                                        ..Default::default()
                                    };
                                    let res = h
                                        .dispatch(HookEvent::BeforeToolCall, &mut hook_ctx)
                                        .await;
                                    if matches!(res, HookResult::Block { .. }) {
                                        tracing::warn!(
                                            tool = %name,
                                            "[hooks] BeforeToolCall blocked, skipping"
                                        );
                                        let blocked_msg = format!(
                                            "[blocked by hook] tool {name} not allowed"
                                        );
                                        sink.on_tool_result(
                                            id,
                                            &blocked_msg,
                                            true,
                                            0,
                                        )
                                        .await;
                                        results.push((id.clone(), blocked_msg, true));
                                        continue;
                                    }
                                }
                                let tool_start = std::time::Instant::now();
                                match tools
                                    .execute_async_with_filter(name, args.clone(), &tool_filter)
                                    .await
                                {
                                    Ok(output) => {
                                        let elapsed_ms =
                                            tool_start.elapsed().as_millis() as u64;
                                        tracing::info!(
                                            "[tool] result: {name} ({id}) is_error={} len={} elapsed_ms={elapsed_ms}",
                                            output.is_error,
                                            output.content.len(),
                                        );
                                        tracing::debug!(
                                            "[tool] result content: {name} ({id})\n{}",
                                            trunc(&output.content, 5000),
                                        );
                                        // 通知 sink 工具执行完成 (default no-op,
                                        // 旧 sink 不感知; daemon SSE 用来发 tool_result 事件)
                                        sink.on_tool_result(
                                            id,
                                            &output.content,
                                            output.is_error,
                                            elapsed_ms,
                                        )
                                        .await;
                                        // ── 缺口 01 集成: 工具调用后 dispatch (AfterToolCall, 触发 ToolGuard+Continuation tier).
                                        if let Some(h) = hooks {
                                            let mut hook_ctx = HookContext {
                                                tool_name: Some(name.clone()),
                                                tool_args: Some(args.clone()),
                                                ..Default::default()
                                            };
                                            let _ = h
                                                .dispatch(HookEvent::AfterToolCall, &mut hook_ctx)
                                                .await;
                                        }
                                        results.push((id.clone(), output.content, output.is_error));
                                    }
                                    Err(e) => {
                                        let elapsed_ms =
                                            tool_start.elapsed().as_millis() as u64;
                                        tracing::error!("[tool] error: {name} ({id}): {e}");
                                        sink.on_status(&format!("工具执行失败: {name} — {e}"))
                                            .await;
                                        let err_content = format!("Error: {e}");
                                        sink.on_tool_result(id, &err_content, true, elapsed_ms)
                                            .await;
                                        // AfterToolCall 同样 dispatch (错误结果也是结果).
                                        if let Some(h) = hooks {
                                            let mut hook_ctx = HookContext {
                                                tool_name: Some(name.clone()),
                                                tool_args: Some(args.clone()),
                                                ..Default::default()
                                            };
                                            let _ = h
                                                .dispatch(HookEvent::AfterToolCall, &mut hook_ctx)
                                                .await;
                                        }
                                        results.push((id.clone(), err_content, true));
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
                            // ── 缺口 01 集成: 出口 dispatch (AfterLoopIter, 触发 Continuation+Skill tier).
                            if let Some(h) = hooks {
                                let mut hook_ctx = HookContext::default();
                                let _ = h.dispatch(HookEvent::AfterLoopIter, &mut hook_ctx).await;
                            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookHandler, HookResult, HookTier};
    use crate::provider::types::{CompletionRequest, LlmStreamEvent};
    use crate::types::{ProviderCapabilities, StopReason};
    use async_trait::async_trait;
    use futures::stream;

    /// 计数 hook: 每次 handle 被调都把 (event_name) 推到共享 Vec.
    /// 用于断言 handle_user_message 触发了几次, 哪些 event.
    struct CountingHook {
        name: String,
        tier: HookTier,
        log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl HookHandler for CountingHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn tier(&self) -> HookTier {
            self.tier
        }
        async fn handle(
            &self,
            event: HookEvent,
            _ctx: &mut HookContext,
        ) -> HookResult {
            let label = match event {
                HookEvent::BeforeLoopIter => "BeforeLoopIter",
                HookEvent::AfterLoopIter => "AfterLoopIter",
                HookEvent::BeforeToolCall => "BeforeToolCall",
                HookEvent::AfterToolCall => "AfterToolCall",
                HookEvent::BeforePromptBuild => "BeforePromptBuild",
                HookEvent::AfterPromptBuild => "AfterPromptBuild",
            };
            self.log.lock().unwrap().push(label.to_string());
            HookResult::Ok
        }
    }

    /// 极简 mock provider: 1 轮返 1 个 Text event + 1 个 Stop, 之后返空流.
    /// 不实现完整 LlmProvider 行为, 只够触发 turn 结束路径.
    struct TextOnlyProvider;

    #[async_trait]
    impl LlmProvider for TextOnlyProvider {
        fn id(&self) -> &str {
            "test-text-only"
        }
        fn name(&self) -> &str {
            "TextOnlyProvider"
        }
        fn capabilities(&self) -> &ProviderCapabilities {
            use std::sync::OnceLock;
            static CAPS: OnceLock<ProviderCapabilities> = OnceLock::new();
            CAPS.get_or_init(|| ProviderCapabilities {
                streaming: true,
                thinking: false,
                tool_use: false,
                max_tokens: Some(1024),
                max_input_tokens: Some(8192),
                supports_system_prompt: true,
                supports_cache_control: false,
                supports_image_input: false,
            })
        }
        async fn stream_completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<LlmStreamEvent, crate::types::LlmError>>,
            crate::types::LlmError,
        > {
            let events: Vec<Result<LlmStreamEvent, crate::types::LlmError>> = vec![
                Ok(LlmStreamEvent::Text("hello".into())),
                Ok(LlmStreamEvent::Stop(StopReason::EndTurn)),
            ];
            Ok(stream::iter(events).boxed())
        }
    }

    /// 收集 sink: 啥也不存, 只标记 agent 是否进入 Idle.
    struct CollectSink {
        finished: std::sync::Arc<std::sync::Mutex<bool>>,
    }

    #[async_trait]
    impl OutputSink for CollectSink {
        async fn on_status(&self, _msg: &str) {}
        async fn on_text(&self, _text: &str) {}
        async fn on_thinking(&self, _text: &str) {}
        async fn on_tool_call(
            &self,
            _id: &str,
            _name: &str,
            _args: &serde_json::Value,
        ) {
        }
        async fn on_token_usage(&self, _usage: &TokenUsage) {}
        async fn on_tool_result(
            &self,
            _id: &str,
            _content: &str,
            _is_error: bool,
            _elapsed_ms: u64,
        ) {
        }
        async fn on_turn_finished(&self, _reason: &StopReason, _usage: &TokenUsage) {
            *self.finished.lock().unwrap() = true;
        }
        async fn on_error(&self, _err: &crate::types::LlmError) {}
    }

    /// 验证: 传 `Some(&HookRegistry)` 时, processing_loop 入口 + 出口各 dispatch 1 次.
    /// Text-only turn (无 tool call), 期望 BeforeLoopIter + AfterLoopIter 各 1.
    #[tokio::test]
    async fn test_handle_user_message_dispatches_entry_and_exit() {
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let registry = std::sync::Arc::new(HookRegistry::new());
        registry.register(std::sync::Arc::new(CountingHook {
            name: "counter-continuation".into(),
            tier: HookTier::Continuation,
            log: log.clone(),
        }));

        let mut agent = AgentLoop::new(AgentConfig {
            max_turns: 4,
            max_retries: 0,
            max_tokens: Some(1024),
            temperature: None,
            thinking: crate::types::ThinkingConfig::Disabled,
            pattern: Default::default(),
            plan_and_execute: Default::default(),
            reflective: Default::default(),
            workflow: Default::default(),
        });
        let mut conv = Conversation::new(None);
        conv.push_user_message(vec![ContentBlock::text("hi")]);
        let tools = ToolRegistry::new();
        let finished = std::sync::Arc::new(std::sync::Mutex::new(false));
        let sink = CollectSink { finished };

        processing_loop::handle_user_message(
            &mut agent,
            &mut conv,
            &TextOnlyProvider,
            &tools,
            ToolCategoryFilter::all(),
            &sink,
            "",
            "",
            "",
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Some(registry.as_ref()),
        )
        .await;

        let log_snapshot = log.lock().unwrap().clone();
        assert!(
            log_snapshot.iter().any(|e| e == "BeforeLoopIter"),
            "expected BeforeLoopIter in {log_snapshot:?}"
        );
        assert!(
            log_snapshot.iter().any(|e| e == "AfterLoopIter"),
            "expected AfterLoopIter in {log_snapshot:?}"
        );
    }

    /// 验证: 传 `None` 时, handle_user_message 正常运行, 不 panic, sink 收到 on_turn_finished.
    /// 这是 plan 9.1.2 的"旧调用方不破坏"契约.
    #[tokio::test]
    async fn test_handle_user_message_with_none_hooks_completes() {
        let mut agent = AgentLoop::new(AgentConfig {
            max_turns: 4,
            max_retries: 0,
            max_tokens: Some(1024),
            temperature: None,
            thinking: crate::types::ThinkingConfig::Disabled,
            pattern: Default::default(),
            plan_and_execute: Default::default(),
            reflective: Default::default(),
            workflow: Default::default(),
        });
        let mut conv = Conversation::new(None);
        conv.push_user_message(vec![ContentBlock::text("hi")]);
        let tools = ToolRegistry::new();
        let finished = std::sync::Arc::new(std::sync::Mutex::new(false));
        let sink = CollectSink {
            finished: finished.clone(),
        };

        processing_loop::handle_user_message(
            &mut agent,
            &mut conv,
            &TextOnlyProvider,
            &tools,
            ToolCategoryFilter::all(),
            &sink,
            "",
            "",
            "",
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
        )
        .await;

        assert!(
            *finished.lock().unwrap(),
            "sink should have received on_turn_finished even with hooks=None"
        );
    }
}

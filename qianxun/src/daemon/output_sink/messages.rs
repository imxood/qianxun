//! 9 个 pub async fn (begin_message/text_delta/.../finish_turn_str/save_snapshot) +
//! 2 个私有 fn (drive_builder + send_event). 从 output_sink.rs 抽, 2026-06-04 Commit 12.

use qianxun_core::provider::types::LlmStreamEvent;
use qianxun_core::types::{LlmError, StopReason, TokenUsage};
use serde_json::Value;

use crate::daemon::sse::{SseEvent, SseEventBuilder};

impl super::DaemonOutputSink {
    /// 发 `MessageStart` (幂等 — 第一次发, 后续 no-op).
    #[allow(dead_code)]
    pub async fn begin_message(&self) {
        let needs_emit = {
            let state = self.state.lock().expect("SinkState mutex poisoned");
            !state.started
        };
        if !needs_emit {
            return;
        }
        let ev = SseEvent::MessageStart {
            session_id: self.session_id.clone(),
            model: self.model.clone(),
            max_tokens: self.max_tokens,
        };
        self.send_event(ev).await;
        let mut state = self.state.lock().expect("SinkState mutex poisoned");
        state.started = true;
    }

    /// 发 `TextDelta` (块切换时自动插 `ContentBlockStart`/`Stop`).
    #[allow(dead_code)]
    pub async fn text_delta(&self, text: &str) {
        let events = self.drive_builder(&LlmStreamEvent::Text(text.to_string()));
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// 发 `ThinkingDelta`.
    #[allow(dead_code)]
    pub async fn thinking(&self, text: &str) {
        let events = self.drive_builder(&LlmStreamEvent::Thinking {
            text: text.to_string(),
            signature: None,
        });
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// 发 `ToolUseComplete` (批式).
    #[allow(dead_code)]
    pub async fn tool_use(&self, id: &str, name: &str, arguments: &Value) {
        let events = self.drive_builder(&LlmStreamEvent::ToolCall {
            id: id.to_string(),
            tool_name: name.to_string(),
            arguments: arguments.clone(),
        });
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// 发 `ToolResult` (工具执行结果).
    #[allow(dead_code)]
    pub async fn tool_result(
        &self,
        tool_use_id: &str,
        content: &str,
        is_error: bool,
        elapsed_ms: u64,
    ) {
        let ev = SseEvent::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: content.to_string(),
            is_error,
            elapsed_ms,
        };
        self.send_event(ev).await;
    }

    /// 发 `Usage` 事件.
    pub async fn usage(&self, u: &TokenUsage) {
        let ev = SseEvent::Usage {
            input_tokens: u.input,
            output_tokens: u.output,
            cache_creation_input_tokens: u.cache_creation_input.unwrap_or(0),
            cache_read_input_tokens: u.cache_read_input.unwrap_or(0),
        };
        self.send_event(ev).await;
    }

    /// 发 `Error` 事件.
    pub async fn error(&self, e: &LlmError) {
        let ev = SseEventBuilder::error_from_llm(e);
        self.send_event(ev).await;
    }

    /// 用 `StopReason` enum 收尾.
    pub async fn finish_turn(&self, reason: &StopReason) {
        let reason_str = SseEventBuilder::stop_reason_str(reason);
        self.finish_turn_str(reason_str).await;
    }

    /// 用字符串 stop_reason 收尾.
    pub async fn finish_turn_str(&self, reason_str: &str) {
        let events = {
            let mut state = self.state.lock().expect("SinkState mutex poisoned");
            state.builder.finalize(reason_str)
        };
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// Stage 3 简化: 流结束写一次占位 snapshot.
    #[allow(dead_code)]
    pub fn save_snapshot(&self, ordinal: u32, conversation_json: &str) {
        let _ = self
            .store
            .save_snapshot(&self.session_id, ordinal, conversation_json);
    }

    // ── 私有 ──

    /// 用 `LlmStreamEvent` 驱动 SseEventBuilder.
    fn drive_builder(&self, ev: &LlmStreamEvent) -> Vec<SseEvent> {
        let mut state = self.state.lock().expect("SinkState mutex poisoned");
        state.builder.from_llm_event(ev)
    }

    /// 内部: 序列化为 JSON → 落盘 → 推 mpsc. `tx.send()` 出错绝不 panic.
    async fn send_event(&self, ev: SseEvent) {
        let (type_name, seq) = {
            let mut state = self.state.lock().expect("SinkState mutex poisoned");
            let type_name = ev.type_name();
            let seq = state.event_seq;
            state.event_seq = state.event_seq.saturating_add(1);
            (type_name, seq)
        };
        if let Ok(json) = serde_json::to_string(&ev) {
            if let Err(e) = self.store.append_event(&self.session_id, seq, type_name, &json) {
                tracing::debug!(
                    error = ?e,
                    session = %self.session_id,
                    seq,
                    "[output_sink] append_event failed (continuing)"
                );
            }
        }
        if self.tx.send(ev).await.is_err() {
            tracing::debug!(
                session = %self.session_id,
                "[output_sink] tx send failed (client disconnected, event dropped)"
            );
        }
    }
}

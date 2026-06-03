//! impl OutputSink for DaemonOutputSink (从 output_sink.rs 抽, 2026-06-04 Commit 12)
//!
//! 8 个 on_* 方法全部路由到 messages.rs 的直接方法.

use async_trait::async_trait;
use qianxun_core::output::OutputSink;
use qianxun_core::types::{LlmError, StopReason, TokenUsage};
use serde_json::Value;

#[async_trait]
impl OutputSink for super::DaemonOutputSink {
    async fn on_text(&self, text: &str) {
        self.text_delta(text).await;
    }

    async fn on_thinking(&self, text: &str) {
        self.thinking(text).await;
    }

    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &Value) {
        self.tool_use(tool_call_id, tool_name, arguments).await;
    }

    async fn on_tool_result(
        &self,
        tool_use_id: &str,
        content: &str,
        is_error: bool,
        elapsed_ms: u64,
    ) {
        self.tool_result(tool_use_id, content, is_error, elapsed_ms).await;
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        self.usage(usage).await;
    }

    async fn on_error(&self, error: &LlmError) {
        self.error(error).await;
    }

    async fn on_turn_finished(&self, reason: &StopReason, _usage: &TokenUsage) {
        self.finish_turn(reason).await;
    }

    async fn on_status(&self, status: &str) {
        // SSE 契约没 status 事件 — 仅 debug 日志.
        tracing::debug!(
            session = %self.session_id,
            status,
            "[output_sink] on_status (not forwarded to SSE)"
        );
    }
}

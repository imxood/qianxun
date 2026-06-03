// ACP output: 字段 capacity 当前未读, 留 buffer 调整时启用.
#![allow(dead_code)]

use async_trait::async_trait;
use crate::acp::types::SessionUpdateContent;
use qianxun_core::output::OutputSink;
use qianxun_core::types::{LlmError, StopReason, TokenUsage};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing;

/// 通过 mpsc 通道将 OutputSink 事件转发给 ACP 传输层
pub struct AcpOutputSink {
    session_id: String,
    tx: mpsc::UnboundedSender<AcpOutputEvent>,
}

pub enum AcpOutputEvent {
    Notification(Value),
    ToolCall(String, String, Value),
    /// session/prompt 的延迟 JSON-RPC 响应（处理完成后发送）
    PromptResponse {
        id: Value,
        stop_reason: String,
    },
}

impl AcpOutputSink {
    pub fn new(session_id: String, tx: mpsc::UnboundedSender<AcpOutputEvent>) -> Self {
        Self { session_id, tx }
    }
}

#[async_trait]
impl OutputSink for AcpOutputSink {
    async fn on_text(&self, text: &str) {
        let content = SessionUpdateContent::AgentMessageChunk {
            text: text.to_string(),
        };
        let notif = crate::acp::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_thinking(&self, text: &str) {
        if text.is_empty() { return; }
        let content = SessionUpdateContent::AgentThoughtChunk {
            text: text.to_string(),
        };
        let notif = crate::acp::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &Value) {
        let content = SessionUpdateContent::ToolCall {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
        };
        let notif = crate::acp::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        tracing::debug!("on_token_usage: input={} output={}", usage.input, usage.output);
        let content = SessionUpdateContent::Usage {
            input_tokens: usage.input,
            output_tokens: usage.output,
        };
        let notif = crate::acp::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_error(&self, error: &LlmError) {
        tracing::warn!("on_error: {error}");
        let content = SessionUpdateContent::Error {
            message: error.to_string(),
        };
        let notif = crate::acp::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_turn_finished(&self, reason: &StopReason, _usage: &TokenUsage) {
        tracing::debug!("on_turn_finished: {reason:?}");
        let reason_str = match reason {
            StopReason::EndTurn => "end_turn",
            StopReason::MaxTokens => "max_tokens",
            StopReason::StopSequence => "stop_sequence",
            StopReason::ToolUse => "tool_use",
            StopReason::ContentFiltered => "content_filtered",
            StopReason::Cancelled => "cancelled",
            StopReason::Error => "error",
            StopReason::Unknown(s) => s.as_str(),
        };
        let content = SessionUpdateContent::TurnFinished {
            reason: reason_str.to_string(),
        };
        let notif = crate::acp::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }
}

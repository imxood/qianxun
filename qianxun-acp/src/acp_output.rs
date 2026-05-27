use crate::types::SessionUpdateContent;
use async_trait::async_trait;
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
        let preview: String = text.chars().take(200).collect();
        if text.len() > 200 {
            tracing::debug!("on_text: {} bytes: \"{preview}...\"", text.len());
        } else {
            tracing::debug!("on_text: {} bytes: \"{preview}\"", text.len());
        }
        let content = SessionUpdateContent::AgentMessageChunk {
            text: text.to_string(),
        };
        let notif = crate::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_thinking(&self, text: &str) {
        if !text.is_empty() {
            let preview: String = text.chars().take(200).collect();
            if text.len() > 200 {
                tracing::debug!("on_thinking: {} bytes: \"{preview}...\"", text.len());
            } else {
                tracing::debug!("on_thinking: {} bytes: \"{preview}\"", text.len());
            }
        }
        let content = SessionUpdateContent::AgentThoughtChunk {
            text: text.to_string(),
        };
        let notif = crate::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &Value) {
        let args_str = serde_json::to_string(arguments).unwrap_or_default();
        let preview: String = args_str.chars().take(200).collect();
        if args_str.len() > 200 {
            tracing::debug!("on_tool_call: {tool_name} ({tool_call_id}) args: {preview}...");
        } else {
            tracing::debug!("on_tool_call: {tool_name} ({tool_call_id}) args: {preview}");
        }
        let content = SessionUpdateContent::ToolCall {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
        };
        let notif = crate::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        tracing::debug!("on_token_usage: input={} output={}", usage.input, usage.output);
        let content = SessionUpdateContent::Usage {
            input_tokens: usage.input,
            output_tokens: usage.output,
        };
        let notif = crate::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }

    async fn on_error(&self, error: &LlmError) {
        tracing::warn!("on_error: {error}");
        let content = SessionUpdateContent::Error {
            message: error.to_string(),
        };
        let notif = crate::types::session_update_notification(&self.session_id, &content);
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
            StopReason::Error => "error",
            StopReason::Unknown(s) => s.as_str(),
        };
        let content = SessionUpdateContent::TurnFinished {
            reason: reason_str.to_string(),
        };
        let notif = crate::types::session_update_notification(&self.session_id, &content);
        let _ = self.tx.send(AcpOutputEvent::Notification(notif));
    }
}

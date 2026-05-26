use crate::agent::message::Message;
use crate::types::{StopReason, ThinkingConfig, TokenUsage, ToolChoice};
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionRequest {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<crate::tools::ToolDefinition>,
    pub tool_choice: ToolChoice,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub thinking: ThinkingConfig,
    pub stop_sequences: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    Text(String),
    Thinking {
        text: String,
        signature: Option<String>,
    },
    ToolCall {
        id: String,
        tool_name: String,
        arguments: Value,
    },
    UsageUpdate(TokenUsage),
    Stop(StopReason),
}

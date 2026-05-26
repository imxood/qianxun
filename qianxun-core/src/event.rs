use std::time::Duration;

use crate::provider::types::LlmStreamEvent;
use crate::types::{LlmError, StopReason, TokenUsage};
use serde_json::Value;

pub type RequestId = String;
pub type SessionId = String;
pub type ToolCallId = String;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    // — 流式输出 —
    TextDelta(String),
    ThinkingDelta(String),

    // — LLM 生命周期 —
    LlmRequestSent {
        request_id: RequestId,
        system: String,
        messages: String,
        tools: String,
        model: String,
    },
    LlmResponseChunk {
        request_id: RequestId,
        chunk: LlmStreamEvent,
    },
    LlmResponseFinished {
        request_id: RequestId,
        usage: TokenUsage,
        duration: Duration,
    },

    // — 工具调用追踪 —
    ToolCallInitiated {
        id: ToolCallId,
        tool_name: String,
        arguments: Value,
    },
    ToolCallFinished {
        id: ToolCallId,
        result: Value,
        duration: Duration,
        is_error: bool,
    },

    // — 回合边界 —
    TurnStarted { turn_id: u32 },
    TurnFinished {
        turn_id: u32,
        reason: StopReason,
        usage: TokenUsage,
    },

    // — 系统 —
    TokenUsageUpdated(TokenUsage),
    Error(LlmError),
    SessionUpdate {
        session_id: String,
        status: String,
    },
}

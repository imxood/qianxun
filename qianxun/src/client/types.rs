//! 错误类型 + 5 个 DTO + SseEvent enum (从 client/mod.rs 抽, 2026-06-04 Commit 15)

use reqwest::Error as ReqwestError;
use serde::{Deserialize, Serialize};
use serde_json::Error as SerdeJsonError;
use std::io::Error as IoError;
use thiserror::Error;

// ─── 错误类型 ────────────────────────────────────────────────

/// Thin client 错误.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] ReqwestError),

    #[error("JSON parse failed: {0}")]
    Json(#[from] SerdeJsonError),

    #[error("SSE parse failed: {0}")]
    Sse(String),

    #[error("Daemon 返回 status {0}: {1}")]
    Status(u16, String),

    #[error("I/O error: {0}")]
    Io(#[from] IoError),
}

// ─── 响应数据结构 (与 daemon 端 router.rs 字段名严格一致) ─────

/// `GET /v1/system/health` 响应.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthStatus {
    pub status: String,
}

/// `POST /v1/chat/session` 响应.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionCreated {
    pub session_id: String,
}

/// `GET /v1/chat/session/{id}` 响应 (Stage 3 子集).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub session_id: String,
    #[serde(default)]
    pub status: String,
}

/// `GET /v1/chat/sessions` 响应 (Stage 3 §6.4 扩展).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionsList {
    pub sessions: Vec<Session>,
}

/// `POST /v1/chat/session/{id}/prompt` 请求体.
#[derive(Debug, Clone, Serialize)]
pub struct PromptRequest {
    pub messages: Vec<PromptMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: String,
}

impl PromptRequest {
    /// 简单文本 prompt (Stage 2 简化, 与 daemon `PromptMessage` 字段一致).
    pub fn text(user_content: &str) -> Self {
        Self {
            messages: vec![PromptMessage {
                role: "user".into(),
                content: user_content.into(),
            }],
        }
    }
}

// ─── SSE 事件 (与 daemon/src/daemon/sse.rs 12 个 variant 严格一致) ─────

/// SSE 事件 (与 shared-contract §3.2 严格一致, 12 种类型).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum SseEvent {
    #[serde(rename = "message_start")]
    MessageStart {
        session_id: String,
        model: String,
        max_tokens: u32,
    },

    #[serde(rename = "content_block_start")]
    ContentBlockStart { index: u32, block_type: String },

    #[serde(rename = "text_delta")]
    TextDelta { index: u32, text: String },

    #[serde(rename = "thinking_delta")]
    ThinkingDelta { index: u32, text: String },

    #[serde(rename = "tool_use_delta")]
    ToolUseDelta {
        index: u32,
        id: String,
        name: String,
        arguments_json: String,
    },

    #[serde(rename = "tool_use_complete")]
    ToolUseComplete {
        index: u32,
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        elapsed_ms: u64,
    },

    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },

    #[serde(rename = "usage")]
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        #[serde(default)]
        cache_creation_input_tokens: u64,
        #[serde(default)]
        cache_read_input_tokens: u64,
    },

    #[serde(rename = "message_delta")]
    MessageDelta { stop_reason: String },

    #[serde(rename = "message_stop")]
    MessageStop,

    #[serde(rename = "error")]
    Error { code: String, message: String },
}

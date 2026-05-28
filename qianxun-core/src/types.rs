use std::time::Duration;
use thiserror::Error;

// ─── LlmError ───────────────────────────────────────────

#[derive(Error, Debug, Clone)]
pub enum LlmError {
    #[error("API key not configured for provider {provider}")]
    NoApiKey { provider: String },

    #[error("rate limit exceeded for {provider}: retry after {retry_after:?}")]
    RateLimitExceeded {
        provider: String,
        retry_after: Option<Duration>,
    },

    #[error("API error from {provider}: {message}")]
    ApiError {
        provider: String,
        status: u16,
        message: String,
    },

    #[error("authentication failed for {provider}: {message}")]
    AuthenticationError { provider: String, message: String },

    #[error("prompt too large (tokens: {tokens:?})")]
    PromptTooLarge { tokens: Option<u64> },

    #[error("stream ended unexpectedly")]
    StreamEnded,
}

// ─── TokenUsage ──────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_creation_input: Option<u64>,
    pub cache_read_input: Option<u64>,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }
}

impl std::ops::Add for TokenUsage {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            input: self.input + rhs.input,
            output: self.output + rhs.output,
            cache_creation_input: self.cache_creation_input.or(rhs.cache_creation_input),
            cache_read_input: self.cache_read_input.or(rhs.cache_read_input),
        }
    }
}

// ─── StopReason ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    ContentFiltered,
    Cancelled,
    Error,
    Unknown(String),
}

// ─── ProviderCapabilities ───────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub thinking: bool,
    pub tool_use: bool,
    pub max_tokens: Option<u64>,
    pub supports_system_prompt: bool,
    pub supports_cache_control: bool,
}

// ─── ThinkingConfig ──────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ThinkingConfig {
    Disabled,
    Enabled { budget_tokens: u64 },
}

// ─── ToolChoice ──────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ToolChoice {
    Auto,
    Any,
    Tool(String),
}

// ─── AgentConfig ─────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentConfig {
    pub max_turns: u32,
    pub max_retries: u32,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub thinking: ThinkingConfig,
}

use std::time::Duration;
use thiserror::Error;

// ─── LlmError ───────────────────────────────────────────

/// LLM provider 错误. 每个 variant 携带 `kind: LlmErrorKind` (15 种语义分类),
/// 由 `provider::error_classifier::classify_http_status` 在 HTTP 错误时注入.
///
/// `kind` 字段缺省 (如 LlmError 来自反序列化或旧代码) → `LlmErrorKind::default() = Unknown`.
#[derive(Error, Debug, Clone)]
pub enum LlmError {
    #[error("API key not configured for provider {provider}")]
    NoApiKey {
        provider: String,
        kind: crate::provider::error_classifier::LlmErrorKind,
    },

    #[error("rate limit exceeded for {provider}: retry after {retry_after:?}")]
    RateLimitExceeded {
        provider: String,
        retry_after: Option<Duration>,
        kind: crate::provider::error_classifier::LlmErrorKind,
    },

    #[error("API error from {provider}: {message}")]
    ApiError {
        provider: String,
        status: u16,
        message: String,
        kind: crate::provider::error_classifier::LlmErrorKind,
    },

    #[error("authentication failed for {provider}: {message}")]
    AuthenticationError {
        provider: String,
        message: String,
        kind: crate::provider::error_classifier::LlmErrorKind,
    },

    #[error("prompt too large (tokens: {tokens:?})")]
    PromptTooLarge {
        tokens: Option<u64>,
        kind: crate::provider::error_classifier::LlmErrorKind,
    },

    #[error("stream ended unexpectedly")]
    StreamEnded {
        kind: crate::provider::error_classifier::LlmErrorKind,
    },
}

impl LlmError {
    /// 取出 `kind` 字段. 已被 `error_classifier::classify_http_status` 注入.
    /// 老代码构造的 LlmError (没填 kind) → `LlmErrorKind::default() = Unknown`.
    pub fn kind(&self) -> crate::provider::error_classifier::LlmErrorKind {
        match self {
            Self::NoApiKey { kind, .. }
            | Self::RateLimitExceeded { kind, .. }
            | Self::ApiError { kind, .. }
            | Self::AuthenticationError { kind, .. }
            | Self::PromptTooLarge { kind, .. }
            | Self::StreamEnded { kind, .. } => *kind,
        }
    }
}

// ─── TokenUsage ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::error_classifier::LlmErrorKind;

    /// LlmError::kind() 从任意 variant 提取 kind 字段. 验证 path 不会 panic.
    #[test]
    fn test_llm_error_kind_helper_extracts_kind() {
        let cases: [(LlmError, LlmErrorKind); 3] = [
            (
                LlmError::AuthenticationError {
                    provider: "deepseek".into(),
                    message: "expired".into(),
                    kind: LlmErrorKind::Auth,
                },
                LlmErrorKind::Auth,
            ),
            (
                LlmError::RateLimitExceeded {
                    provider: "deepseek".into(),
                    retry_after: None,
                    kind: LlmErrorKind::RateLimit,
                },
                LlmErrorKind::RateLimit,
            ),
            (
                LlmError::StreamEnded {
                    kind: LlmErrorKind::Timeout,
                },
                LlmErrorKind::Timeout,
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.kind(), expected);
        }
    }

    /// LlmErrorKind::default() = Unknown, 跟 serde untagged 兜底一致.
    #[test]
    fn test_llm_error_kind_default_is_unknown() {
        assert_eq!(LlmErrorKind::default(), LlmErrorKind::Unknown);
    }
}

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
    /// 单次响应最大 token 数
    pub max_tokens: Option<u64>,
    /// 上下文窗口大小 (input token 上限)
    pub max_input_tokens: Option<u64>,
    pub supports_system_prompt: bool,
    pub supports_cache_control: bool,
    /// 是否支持图片/图像输入 (多模态)
    pub supports_image_input: bool,
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

// ─── AgentPattern ─────────────────────────────────────────

/// Agent 工作模式。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPattern {
    /// React: 思考 -> 行动 -> 观察 循环（默认）
    #[default]
    React,
    /// Plan-and-Execute: 先制定计划再逐步执行
    PlanAndExecute,
    /// Reflective: 执行后自检一轮
    Reflective,
    /// Workflow: 按预设阶段序列执行
    Workflow,
}

// ─── AgentPattern 子配置 ──────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanAndExecuteConfig {
    #[serde(default)]
    pub auto_execute: bool,
    #[serde(default = "default_plan_turns")]
    pub max_plan_turns: u32,
    #[serde(default = "default_execute_turns")]
    pub max_execute_turns: u32,
    #[serde(default = "default_approval_timeout")]
    pub approval_timeout_sec: u64,
}

fn default_plan_turns() -> u32 {
    20
}
fn default_execute_turns() -> u32 {
    50
}
fn default_approval_timeout() -> u64 {
    300
}

impl Default for PlanAndExecuteConfig {
    fn default() -> Self {
        Self {
            auto_execute: false,
            max_plan_turns: 20,
            max_execute_turns: 50,
            approval_timeout_sec: 300,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReflectiveConfig {
    #[serde(default = "default_review_rounds")]
    pub max_review_rounds: u32,
    #[serde(default = "default_confidence")]
    pub review_confidence_threshold: u8,
    #[serde(default = "default_review_tool_only")]
    pub only_review_when_tool_used: bool,
}

fn default_review_rounds() -> u32 {
    2
}
fn default_confidence() -> u8 {
    8
}
fn default_review_tool_only() -> bool {
    true
}

impl Default for ReflectiveConfig {
    fn default() -> Self {
        Self {
            max_review_rounds: 2,
            review_confidence_threshold: 8,
            only_review_when_tool_used: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_stage_turns")]
    pub max_stage_turns: u32,
    #[serde(default)]
    pub custom_path: Option<String>,
}

fn default_stage_turns() -> u32 {
    30
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            max_stage_turns: 30,
            custom_path: None,
        }
    }
}

// ─── Mode ──────────────────────────────────────────────────

/// 千寻运行时模式（通过 `/mode` 命令切换）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 自动模式（默认）：可自由调用所有工具
    #[default]
    Auto,
    /// 计划模式：只允许 Read / Search / Think 类工具，禁止写操作
    Plan,
}

impl Mode {
    pub fn tool_filter(&self) -> crate::tools::ToolCategoryFilter {
        match self {
            Mode::Auto => crate::tools::ToolCategoryFilter::all(),
            Mode::Plan => crate::tools::ToolCategoryFilter::read_only(),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Plan => "plan",
        }
    }
}

// ─── AgentConfig ─────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentConfig {
    pub max_turns: u32,
    pub max_retries: u32,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f32>,
    pub thinking: ThinkingConfig,

    #[serde(default)]
    pub pattern: AgentPattern,
    #[serde(default)]
    pub plan_and_execute: PlanAndExecuteConfig,
    #[serde(default)]
    pub reflective: ReflectiveConfig,
    #[serde(default)]
    pub workflow: WorkflowConfig,
}

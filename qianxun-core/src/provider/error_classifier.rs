//! 缺口 02: LLM 错误分类与恢复.
//!
//! 在 `LlmError` (6 变体, thiserror 派生, 字段携带 provider/status/message) 之上,
//! 引入更细的 **`LlmErrorKind`** (15 种语义分类) + **`RecoveryAction`** 决策树.
//!
//! ## 设计原则
//!
//! - **不替换** `LlmError`, 仅在其之上叠加分类层. 现有 API 路径
//!   (`SseEventBuilder::error_from_llm`, HTTP 错误传播) 仍用 `LlmError` 直传.
//! - **分类 + 决策** 由两个纯函数完成: `classify_llm_error` + `decide_recovery`.
//!   方便缺口 12 (`ProviderStack` 三层 failover) 复用决策.
//! - **HTTP status code 细节映射** (402→Billing, 408→Timeout, 413→PayloadTooLarge,
//!   400 错误体含 `context_length_exceeded` → ContextOverflow 等) 在此层补齐,
//!   现有 `SseEventBuilder::error_from_llm` 只做了 4 大类粗分 (auth/rate_limit/api_error/internal).
//!
//! ## 不修改
//!
//! - `SseEvent::Error { code, message }` 序列化契约 (shared-contract §3.2)
//! - `LlmError` 6 变体 (现有 248 测试依赖)
//! - `error_from_llm` 4-code 映射 (现有测试依赖)
//!
//! ## 调用方
//!
//! - 缺口 12 ProviderStack: 用 `decide_recovery` 决定 Layer 1 升级到 Layer 2/3 的条件.
//! - 缺口 09 ContextWindow: 收到 `ContextOverflow` 时触发压缩.
//! - 客户端 UI: 通过 `SseEvent::Error.code` 已经能拿到粗分类 (auth/rate_limit/api_error/internal).

use std::time::Duration;

use crate::types::LlmError;

// ─── LlmErrorKind ──────────────────────────────────────────

/// LLM 错误的细粒度语义分类 (15 种).
///
/// 与 `LlmError` 的区别:
/// - `LlmError` 携带**结构化字段** (provider, status, message), 用于上游传播 + 日志.
/// - `LlmErrorKind` 是**扁平 enum**, 用于决策逻辑 (缺口 12 failover / 缺口 09 压缩 / 前端 toast).
///
/// 命名采用 snake_case 以便直接 `serde::Serialize` 出 JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmErrorKind {
    // ── 认证 / 计费 (不可重试) ──
    /// API key 临时无效 (e.g. token 过期, 可刷新)
    Auth,
    /// API key 永久无效 (e.g. 被吊销, 必须人工换)
    AuthPermanent,
    /// 账户余额不足
    Billing,

    // ── 限流 / 过载 (可重试或切换) ──
    /// 触发 rate limit (HTTP 429)
    RateLimit,
    /// provider 临时过载 (HTTP 529 / 503 with overload header)
    Overloaded,

    // ── 服务端错误 (可重试) ──
    /// 5xx 服务器错误
    ServerError,

    // ── 网络 / 超时 (可重试) ──
    /// 请求超时
    Timeout,

    // ── 请求过大 / 上下文超限 (需压缩) ──
    /// 上下文窗口溢出
    ContextOverflow,
    /// 单次请求体超限 (HTTP 413)
    PayloadTooLarge,

    // ── 模型 / 内容 (不可重试或切模型) ──
    /// 模型不存在 / 已下架
    ModelNotFound,
    /// 内容策略违规
    ContentPolicyBlocked,

    // ── 解析 / 协议错误 ──
    /// 响应格式错误 (JSON 解析失败等)
    FormatError,
    /// thinking signature 校验失败
    InvalidThinkingSig,

    // ── 兜底 ──
    /// 未分类
    Unknown,

    // ── 顶层 catch-all ──
    /// 所有 provider 都失败 (缺口 12 layer 3 抛出)
    AllProvidersFailed,
}

impl LlmErrorKind {
    /// 人类可读名 (snake_case 跟 serde 一致).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::AuthPermanent => "auth_permanent",
            Self::Billing => "billing",
            Self::RateLimit => "rate_limit",
            Self::Overloaded => "overloaded",
            Self::ServerError => "server_error",
            Self::Timeout => "timeout",
            Self::ContextOverflow => "context_overflow",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ModelNotFound => "model_not_found",
            Self::ContentPolicyBlocked => "content_policy_blocked",
            Self::FormatError => "format_error",
            Self::InvalidThinkingSig => "invalid_thinking_sig",
            Self::Unknown => "unknown",
            Self::AllProvidersFailed => "all_providers_failed",
        }
    }

    /// 该 kind 是否**绝对不可重试** (立即中止).
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::AuthPermanent | Self::Billing | Self::ContentPolicyBlocked
        )
    }
}

impl Default for LlmErrorKind {
    /// 兜底: 未分类时按 Unknown 处理. 用于 `#[serde(default)]` 反序列化和
    /// `LlmError` 字段缺失时的 fallthrough.
    fn default() -> Self {
        Self::Unknown
    }
}

// ─── RecoveryAction ────────────────────────────────────────

/// 恢复动作决策.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RecoveryAction {
    /// 同 provider 重试, 等待 `delay`.
    Retry { delay: Duration },
    /// 切换到下个 provider (缺口 12 layer 2).
    RotateProvider,
    /// 触发 context 压缩 (缺口 09).
    CompressContext,
    /// 切到 fallback 模型.
    FallbackModel { model: String },
    /// 立即中止, 不再尝试.
    Abort { reason: String },
}

// ─── Classifier ────────────────────────────────────────────

/// 把 `LlmError` 分类成 `LlmErrorKind`.
///
/// 涵盖缺口 02 §2.4 的 HTTP 状态码映射:
/// - 401 / 403 → `Auth` 或 `AuthPermanent` (按 message 关键词判定)
/// - 402 → `Billing`
/// - 408 / 网超时 → `Timeout`
/// - 413 → `PayloadTooLarge`
/// - 429 → `RateLimit`
/// - 500-504 → `ServerError` / `Overloaded`
/// - 400 + `context_length_exceeded` → `ContextOverflow`
/// - 400 + `model_not_found` → `ModelNotFound`
/// - 400 + `content_policy` / `safety` → `ContentPolicyBlocked`
pub fn classify_llm_error(err: &LlmError) -> LlmErrorKind {
    match err {
        LlmError::NoApiKey { .. } => LlmErrorKind::AuthPermanent,
        LlmError::AuthenticationError { message, .. } => {
            // "expired" / "invalid" 但 key 形态正常 → 可刷新 Auth;
            // "revoked" / "deleted" → AuthPermanent
            let m = message.to_ascii_lowercase();
            if m.contains("revoked") || m.contains("deleted") || m.contains("disabled") {
                LlmErrorKind::AuthPermanent
            } else {
                LlmErrorKind::Auth
            }
        }
        LlmError::RateLimitExceeded { .. } => LlmErrorKind::RateLimit,
        LlmError::PromptTooLarge { .. } => LlmErrorKind::ContextOverflow,
        LlmError::StreamEnded { .. } => LlmErrorKind::Timeout,
        LlmError::ApiError {
            status, message, ..
        } => classify_http_status(*status, message),
    }
}

/// HTTP 状态码 + 错误消息 → LlmErrorKind 映射 (缺口 02 §2.4 完整表).
///
/// 公开: anthropic_compat.rs 在构造 `LlmError` 之前先调本函数拿到精确分类,
/// 注入到 `kind` 字段. 不再走 `classify_llm_error` 内部 match.
pub fn classify_http_status(status: u16, message: &str) -> LlmErrorKind {
    let msg_lower = message.to_ascii_lowercase();
    match status {
        401 | 403 => {
            // 401/403 走 AuthenticationError 分支, 此处兜底
            if msg_lower.contains("revoked") || msg_lower.contains("deleted") {
                LlmErrorKind::AuthPermanent
            } else {
                LlmErrorKind::Auth
            }
        }
        402 => LlmErrorKind::Billing,
        408 => LlmErrorKind::Timeout,
        413 => LlmErrorKind::PayloadTooLarge,
        429 => LlmErrorKind::RateLimit,
        500 | 502 | 504 => LlmErrorKind::ServerError,
        503 => {
            // 503 + overload header 或消息 → Overloaded, 否则 ServerError
            if msg_lower.contains("overload") || msg_lower.contains("capacity") {
                LlmErrorKind::Overloaded
            } else {
                LlmErrorKind::ServerError
            }
        }
        529 => LlmErrorKind::Overloaded, // Anthropic-specific overloaded status
        400 => {
            // 400 类要看消息: Anthropic 错误体常含 type 字段
            if msg_lower.contains("context_length")
                || msg_lower.contains("context_length_exceeded")
                || msg_lower.contains("maximum context length")
            {
                LlmErrorKind::ContextOverflow
            } else if msg_lower.contains("model_not_found")
                || msg_lower.contains("model does not exist")
            {
                LlmErrorKind::ModelNotFound
            } else if msg_lower.contains("content_policy")
                || msg_lower.contains("safety")
                || msg_lower.contains("content_filter")
            {
                LlmErrorKind::ContentPolicyBlocked
            } else if msg_lower.contains("invalid_thinking")
                || msg_lower.contains("signature")
            {
                LlmErrorKind::InvalidThinkingSig
            } else {
                LlmErrorKind::FormatError
            }
        }
        _ if (400..500).contains(&status) => LlmErrorKind::FormatError,
        _ => LlmErrorKind::Unknown,
    }
}

// ─── RecoveryAction 决策 ───────────────────────────────────

/// 根据 `LlmErrorKind` + 当前重试次数, 决定 `RecoveryAction`.
///
/// 决策表见 `docs/设计/能力层/02_LLM错误分类与恢复.md` §2.3.
///
/// # 参数
///
/// - `kind`: 分类结果
/// - `attempt`: 当前已经重试过的次数 (0 表示首次失败)
/// - `fallback_model`: 可选 fallback 模型名 (用户配置)
pub fn decide_recovery(
    kind: LlmErrorKind,
    attempt: u32,
    fallback_model: Option<&str>,
) -> RecoveryAction {
    use LlmErrorKind::*;

    // 1. 绝对不可重试
    if kind.is_fatal() {
        return RecoveryAction::Abort {
            reason: kind.as_str().to_string(),
        };
    }

    match kind {
        Auth => RecoveryAction::Abort {
            reason: "API key invalid, please check".into(),
        },
        AuthPermanent | Billing | ContentPolicyBlocked => unreachable!("covered by is_fatal"),
        RateLimit => {
            // 指数 backoff: 1s, 2s, 4s, 8s (cap 30s)
            let delay_secs = (1u64 << attempt.min(5)).min(30);
            if attempt < 3 {
                RecoveryAction::Retry {
                    delay: Duration::from_secs(delay_secs),
                }
            } else {
                // 3 次后升级到 layer 2 rotate
                RecoveryAction::RotateProvider
            }
        }
        Overloaded => RecoveryAction::RotateProvider,
        ServerError => {
            if attempt < 2 {
                RecoveryAction::Retry {
                    delay: Duration::from_secs(1),
                }
            } else {
                RecoveryAction::RotateProvider
            }
        }
        Timeout => {
            if attempt < 1 {
                RecoveryAction::Retry {
                    delay: Duration::from_secs(2),
                }
            } else {
                RecoveryAction::RotateProvider
            }
        }
        ContextOverflow | PayloadTooLarge => {
            // 触发压缩; 若有 fallback 模型, 同时切到 mini 模型
            if let Some(model) = fallback_model {
                RecoveryAction::FallbackModel {
                    model: model.to_string(),
                }
            } else {
                RecoveryAction::CompressContext
            }
        }
        ModelNotFound => fallback_model
            .map(|m| RecoveryAction::FallbackModel { model: m.to_string() })
            .unwrap_or_else(|| RecoveryAction::Abort {
                reason: "model not found and no fallback configured".into(),
            }),
        FormatError | InvalidThinkingSig => {
            if attempt < 1 {
                RecoveryAction::Retry {
                    delay: Duration::from_millis(500),
                }
            } else {
                RecoveryAction::Abort {
                    reason: "format error persists, giving up".into(),
                }
            }
        }
        AllProvidersFailed => RecoveryAction::Abort {
            reason: "all providers failed".into(),
        },
        Unknown => RecoveryAction::Retry {
            delay: Duration::from_secs(1),
        },
    }
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_api_error(status: u16, message: &str) -> LlmError {
        LlmError::ApiError {
            provider: "test".into(),
            status,
            message: message.into(),
            kind: classify_http_status(status, message),
        }
    }

    // ── classify_llm_error: LlmError → LlmErrorKind ──

    #[test]
    fn test_classify_no_api_key_is_auth_permanent() {
        let e = LlmError::NoApiKey {
            provider: "deepseek".into(),
            kind: LlmErrorKind::AuthPermanent,
        };
        assert_eq!(classify_llm_error(&e), LlmErrorKind::AuthPermanent);
    }

    #[test]
    fn test_classify_auth_expired_is_auth() {
        let e = LlmError::AuthenticationError {
            provider: "deepseek".into(),
            message: "token expired, please refresh".into(),
            kind: LlmErrorKind::Auth,
        };
        assert_eq!(classify_llm_error(&e), LlmErrorKind::Auth);
    }

    #[test]
    fn test_classify_auth_revoked_is_auth_permanent() {
        let e = LlmError::AuthenticationError {
            provider: "deepseek".into(),
            message: "API key has been revoked".into(),
            kind: LlmErrorKind::AuthPermanent,
        };
        assert_eq!(classify_llm_error(&e), LlmErrorKind::AuthPermanent);
    }

    #[test]
    fn test_classify_rate_limit() {
        let e = LlmError::RateLimitExceeded {
            provider: "deepseek".into(),
            retry_after: Some(Duration::from_secs(5)),
            kind: LlmErrorKind::RateLimit,
        };
        assert_eq!(classify_llm_error(&e), LlmErrorKind::RateLimit);
    }

    #[test]
    fn test_classify_prompt_too_large_is_context_overflow() {
        let e = LlmError::PromptTooLarge {
            tokens: Some(50000),
            kind: LlmErrorKind::ContextOverflow,
        };
        assert_eq!(classify_llm_error(&e), LlmErrorKind::ContextOverflow);
    }

    #[test]
    fn test_classify_http_402_billing() {
        let e = make_api_error(402, "insufficient balance");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::Billing);
    }

    #[test]
    fn test_classify_http_408_timeout() {
        let e = make_api_error(408, "request timeout");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::Timeout);
    }

    #[test]
    fn test_classify_http_413_payload_too_large() {
        let e = make_api_error(413, "request body too large");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::PayloadTooLarge);
    }

    #[test]
    fn test_classify_http_429_rate_limit() {
        let e = make_api_error(429, "rate limit exceeded");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::RateLimit);
    }

    #[test]
    fn test_classify_http_500_server_error() {
        let e = make_api_error(500, "internal server error");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::ServerError);
    }

    #[test]
    fn test_classify_http_503_overload() {
        let e = make_api_error(503, "service overloaded, retry later");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::Overloaded);
    }

    #[test]
    fn test_classify_http_503_generic_is_server_error() {
        let e = make_api_error(503, "service unavailable for maintenance");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::ServerError);
    }

    #[test]
    fn test_classify_http_529_anthropic_overloaded() {
        let e = make_api_error(529, "overloaded");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::Overloaded);
    }

    #[test]
    fn test_classify_http_400_context_overflow_via_message() {
        let e = make_api_error(400, "context_length_exceeded: max 8192 tokens");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::ContextOverflow);
    }

    #[test]
    fn test_classify_http_400_model_not_found() {
        let e = make_api_error(400, "model_not_found: gpt-99 does not exist");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::ModelNotFound);
    }

    #[test]
    fn test_classify_http_400_content_policy() {
        let e = make_api_error(400, "content_policy_violation: unsafe content");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::ContentPolicyBlocked);
    }

    #[test]
    fn test_classify_http_400_invalid_thinking_signature() {
        let e = make_api_error(400, "invalid_thinking_signature detected");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::InvalidThinkingSig);
    }

    #[test]
    fn test_classify_http_400_unknown_is_format_error() {
        let e = make_api_error(400, "malformed JSON in tool_call");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::FormatError);
    }

    #[test]
    fn test_classify_http_5xx_unknown_is_unknown() {
        let e = make_api_error(599, "weird status");
        assert_eq!(classify_llm_error(&e), LlmErrorKind::Unknown);
    }

    #[test]
    fn test_classify_stream_ended_is_timeout() {
        let e = LlmError::StreamEnded {
            kind: LlmErrorKind::Timeout,
        };
        assert_eq!(classify_llm_error(&e), LlmErrorKind::Timeout);
    }

    // ── is_fatal ──

    #[test]
    fn test_is_fatal() {
        assert!(LlmErrorKind::AuthPermanent.is_fatal());
        assert!(LlmErrorKind::Billing.is_fatal());
        assert!(LlmErrorKind::ContentPolicyBlocked.is_fatal());
        assert!(!LlmErrorKind::Auth.is_fatal());
        assert!(!LlmErrorKind::RateLimit.is_fatal());
        assert!(!LlmErrorKind::ServerError.is_fatal());
    }

    // ── decide_recovery ──

    #[test]
    fn test_decide_recovery_fatal_aborts_immediately() {
        let action = decide_recovery(LlmErrorKind::Billing, 0, None);
        assert!(matches!(action, RecoveryAction::Abort { .. }));
    }

    #[test]
    fn test_decide_recovery_auth_aborts() {
        // Auth (not fatal) → still Abort, message says "check API key"
        let action = decide_recovery(LlmErrorKind::Auth, 0, None);
        match action {
            RecoveryAction::Abort { reason } => assert!(reason.contains("API key")),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn test_decide_recovery_rate_limit_backoff() {
        // attempt=0 → 1s
        match decide_recovery(LlmErrorKind::RateLimit, 0, None) {
            RecoveryAction::Retry { delay } => assert_eq!(delay, Duration::from_secs(1)),
            other => panic!("expected Retry 1s, got {other:?}"),
        }
        // attempt=1 → 2s
        match decide_recovery(LlmErrorKind::RateLimit, 1, None) {
            RecoveryAction::Retry { delay } => assert_eq!(delay, Duration::from_secs(2)),
            other => panic!("expected Retry 2s, got {other:?}"),
        }
        // attempt=3 → RotateProvider (升级 layer 2)
        assert!(matches!(
            decide_recovery(LlmErrorKind::RateLimit, 3, None),
            RecoveryAction::RotateProvider
        ));
    }

    #[test]
    fn test_decide_recovery_overloaded_rotates_immediately() {
        assert!(matches!(
            decide_recovery(LlmErrorKind::Overloaded, 0, None),
            RecoveryAction::RotateProvider
        ));
    }

    #[test]
    fn test_decide_recovery_server_error_retries_then_rotates() {
        // attempt=0 → Retry 1s
        assert!(matches!(
            decide_recovery(LlmErrorKind::ServerError, 0, None),
            RecoveryAction::Retry { .. }
        ));
        // attempt=2 → RotateProvider
        assert!(matches!(
            decide_recovery(LlmErrorKind::ServerError, 2, None),
            RecoveryAction::RotateProvider
        ));
    }

    #[test]
    fn test_decide_recovery_context_overflow_compresses() {
        assert!(matches!(
            decide_recovery(LlmErrorKind::ContextOverflow, 0, None),
            RecoveryAction::CompressContext
        ));
    }

    #[test]
    fn test_decide_recovery_context_overflow_with_fallback_uses_fallback() {
        match decide_recovery(LlmErrorKind::ContextOverflow, 0, Some("deepseek-mini")) {
            RecoveryAction::FallbackModel { model } => assert_eq!(model, "deepseek-mini"),
            other => panic!("expected FallbackModel, got {other:?}"),
        }
    }

    #[test]
    fn test_decide_recovery_model_not_found_without_fallback_aborts() {
        assert!(matches!(
            decide_recovery(LlmErrorKind::ModelNotFound, 0, None),
            RecoveryAction::Abort { .. }
        ));
    }

    #[test]
    fn test_decide_recovery_format_error_retries_once() {
        // attempt=0 → Retry 500ms
        match decide_recovery(LlmErrorKind::FormatError, 0, None) {
            RecoveryAction::Retry { delay } => assert_eq!(delay, Duration::from_millis(500)),
            other => panic!("expected Retry 500ms, got {other:?}"),
        }
        // attempt=1 → Abort
        assert!(matches!(
            decide_recovery(LlmErrorKind::FormatError, 1, None),
            RecoveryAction::Abort { .. }
        ));
    }

    #[test]
    fn test_decide_recovery_all_providers_failed_aborts() {
        assert!(matches!(
            decide_recovery(LlmErrorKind::AllProvidersFailed, 0, None),
            RecoveryAction::Abort { .. }
        ));
    }

    #[test]
    fn test_decide_recovery_unknown_retries() {
        assert!(matches!(
            decide_recovery(LlmErrorKind::Unknown, 0, None),
            RecoveryAction::Retry { .. }
        ));
    }

    // ── as_str ──

    #[test]
    fn test_as_str_matches_serde() {
        // 验证 as_str 跟 serde::Serialize 输出 snake_case 一致
        for kind in [
            LlmErrorKind::Auth,
            LlmErrorKind::AuthPermanent,
            LlmErrorKind::RateLimit,
            LlmErrorKind::ContextOverflow,
            LlmErrorKind::AllProvidersFailed,
        ] {
            let s = kind.as_str();
            let json = serde_json::to_string(&kind).unwrap();
            // json 形如 "\"rate_limit\""
            assert_eq!(json, format!("\"{s}\""), "mismatch for {kind:?}");
        }
    }
}
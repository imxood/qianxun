//! 缺口 12 Provider 三层 Failover — 务实版 (Layer 1 + Layer 2).
//!
//! ## 三层决策流程
//!
//! ```text
//! [ProviderStack::stream_completion]
//!    │
//!    ├─ Layer 1: 同一 provider 指数 backoff (decide_recovery 决策)
//!    │  │
//!    │  ├─ Retry  (RateLimit/ServerError/Timeout) → backoff(1s/2s/4s)
//!    │  ├─ Rotate (RateLimit × 3 / Overloaded)      → 切 Layer 2
//!    │  └─ Abort  (Auth/Billing/ContentPolicy)       → 立即终止
//!    │
//!    └─ Layer 2: 切下个 provider (按 HashMap 迭代顺序, 不保证稳定)
//!       │
//!       └─ 全部失败 → LlmError::ApiError { kind: AllProvidersFailed, .. }
//! ```
//!
//! **不实施 Layer 3 (AdaptiveRouter + scoreboard)** — 见 `docs/设计/能力层/12_Provider三层Failover.md` §"不做什么" / P2 备注.
//!
//! ## 重要设计: ProviderStack 自己 `impl LlmProvider`
//!
//! 这是一个 **forwarder pattern**: 业务调用方 (`processing_loop`, `compact`, `SharedState`)
//! 全部继续用 `&dyn LlmProvider`, 实际拿到的是 `ProviderStack` (通过 `Arc<dyn LlmProvider>` upcast).
//! 业务代码零修改, ProviderStack 对外透明.
//!
//! ## 行为差异 vs 旧版
//!
//! - **旧版** (`qianxun-core/src/agent/engine.rs:228-253`): 临时 RateLimit retry 循环, 只 retry 同 provider
//! - **新版** (本模块): 完整 Layer 1 + Layer 2, 决策走 `decide_recovery`, 全部失败统一标 `AllProvidersFailed`
//!
//! ## Fallback 顺序
//!
//! `config.providers` 是 `HashMap`, 无序. `ProviderStack::new` 内部按 HashMap 迭代顺序
//! 拆 active + fallbacks, **不保证稳定**. 升级稳定顺序时需改 `ResolvedConfig` 加 `provider_order: Vec<String>` 字段 (留 P2).

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::config::ResolvedProviderConfig;
use crate::provider::error_classifier::{decide_recovery, LlmErrorKind, RecoveryAction};
use crate::provider::types::{CompletionRequest, LlmStreamEvent};
use crate::provider::LlmProvider;
use crate::types::{LlmError, ProviderCapabilities};

/// 多 provider failover 包装器 (Layer 1 retry + Layer 2 rotate).
///
/// 业务调用方拿到 `Arc<dyn LlmProvider>` 后, 直接 `provider.stream_completion(req)` 即可.
/// 内部走 primary → fallback chain, 全部失败返 `LlmError::ApiError { kind: AllProvidersFailed, .. }`.
pub struct ProviderStack {
    /// 活跃 provider (来自 `config.active_provider`).
    primary: (String, Arc<dyn LlmProvider>),
    /// 备用 providers (按 HashMap 迭代顺序, 不保证稳定).
    fallbacks: Vec<(String, Arc<dyn LlmProvider>)>,
    /// 单个 provider 最大重试次数 (来自 `ResolvedConfig.agent.max_retries`).
    /// 注: `decide_recovery` 内部对 RateLimit 3 次后返 RotateProvider, 这里限制为 1+max_retries 次总尝试.
    max_retries_per_provider: u32,
}

impl ProviderStack {
    /// 构造 ProviderStack. 内部过滤 `api_key.is_empty()` 的 provider.
    ///
    /// # 参数
    ///
    /// - `entries`: 全部 (name, ResolvedProviderConfig, Box<dyn LlmProvider>) 元组
    ///   (来自 state.rs::build 遍历 `config.providers` HashMap)
    /// - `active_name`: 活跃 provider 名 (来自 `config.active_provider`)
    /// - `max_retries_per_provider`: 单 provider 重试上限 (来自 `config.agent.max_retries`)
    ///
    /// # 顺序
    ///
    /// HashMap 迭代顺序 — **不保证稳定**. 详细见模块级 doc.
    pub fn new(
        entries: Vec<(String, ResolvedProviderConfig, Box<dyn LlmProvider>)>,
        active_name: &str,
        max_retries_per_provider: u32,
    ) -> Self {
        // 1. 过滤空 api_key (没 key 的 provider 跳过, 不会出现在 fallbacks)
        let valid: Vec<(String, ResolvedProviderConfig, Box<dyn LlmProvider>)> = entries
            .into_iter()
            .filter(|(_, cfg, _)| !cfg.api_key.is_empty())
            .collect();

        // 2. 拆 active (primary) + fallbacks
        let mut primary: Option<(String, Arc<dyn LlmProvider>)> = None;
        let mut fallbacks: Vec<(String, Arc<dyn LlmProvider>)> = Vec::new();
        for (name, _, provider) in valid {
            let arc: Arc<dyn LlmProvider> = Arc::from(provider);
            if name == active_name {
                primary = Some((name, arc));
            } else {
                fallbacks.push((name, arc));
            }
        }

        // 3. 兜底: active 也没效 key (空 HashMap 或全空 key) — 用 fallbacks[0] 顶上, 仍无则构造一个空壳
        let primary = primary.unwrap_or_else(|| {
            fallbacks
                .first()
                .cloned()
                .unwrap_or_else(|| (active_name.to_string(), dummy_provider(active_name)))
        });

        Self {
            primary,
            fallbacks,
            max_retries_per_provider,
        }
    }

    /// 全部 provider 失败后构造的统一错误.
    fn all_failed_error(&self) -> LlmError {
        let total = 1 + self.fallbacks.len();
        LlmError::ApiError {
            provider: self.primary.0.clone(),
            status: 0,
            message: format!("all {total} providers failed (primary + {} fallbacks)", self.fallbacks.len()),
            kind: LlmErrorKind::AllProvidersFailed,
        }
    }

    /// 单 provider 尝试 (Layer 1 retry 循环).
    ///
    /// 返 `Success(stream)` / `Rotate` (切下个) / `Err` (终止).
    /// 错误透传: 致命错误 (`is_fatal`) 直接返原始 LlmError, 不包成 AllProvidersFailed.
    async fn try_provider(
        &self,
        p: &(String, Arc<dyn LlmProvider>),
        req: &CompletionRequest,
        mut attempt: u32,
    ) -> Result<ProviderAttempt, LlmError> {
        loop {
            match p.1.stream_completion(req.clone()).await {
                Ok(stream) => return Ok(ProviderAttempt::Success(stream)),
                Err(e) => {
                    let kind = e.kind();

                    // 致命错误 (AuthPermanent / Billing / ContentPolicyBlocked) 立即透传
                    if kind.is_fatal() {
                        tracing::error!(
                            provider = %p.0,
                            kind = kind.as_str(),
                            error = %e,
                            "[failover] fatal error, aborting"
                        );
                        return Err(e);
                    }

                    // 走 decide_recovery 决策
                    let action = decide_recovery(kind, attempt, None);
                    match action {
                        RecoveryAction::Retry { delay } => {
                            if attempt < self.max_retries_per_provider {
                                attempt += 1;
                                tracing::info!(
                                    provider = %p.0,
                                    attempt,
                                    delay_ms = delay.as_millis() as u64,
                                    kind = kind.as_str(),
                                    "[failover] retry same provider"
                                );
                                tokio::time::sleep(delay).await;
                                continue;
                            }
                            // 超过 max_retries, 升级到 Layer 2
                            tracing::warn!(
                                provider = %p.0,
                                attempts = attempt,
                                kind = kind.as_str(),
                                "[failover] exhausted retries, rotating"
                            );
                            return Ok(ProviderAttempt::Rotate);
                        }
                        RecoveryAction::RotateProvider => {
                            tracing::warn!(
                                provider = %p.0,
                                kind = kind.as_str(),
                                "[failover] recovery says rotate, switching"
                            );
                            return Ok(ProviderAttempt::Rotate);
                        }
                        RecoveryAction::Abort { reason } => {
                            // 透传 abort 原因 (包装成 ApiError 统一形态)
                            tracing::error!(
                                provider = %p.0,
                                reason = %reason,
                                kind = kind.as_str(),
                                "[failover] abort decision from recovery"
                            );
                            return Err(LlmError::ApiError {
                                provider: p.0.clone(),
                                status: 0,
                                message: reason,
                                kind,
                            });
                        }
                        RecoveryAction::CompressContext | RecoveryAction::FallbackModel { .. } => {
                            // Layer 3 缺失, 这俩 action 是 scoreboard hook — 当前阶段当 terminal
                            // 透传原始 LlmError 给上层 (engine.rs 走 sink.on_error)
                            tracing::error!(
                                provider = %p.0,
                                kind = kind.as_str(),
                                "[failover] unsupported action in layer1+2, aborting (Layer 3 missing)"
                            );
                            return Err(e);
                        }
                    }
                }
            }
        }
    }
}

enum ProviderAttempt {
    Success(BoxStream<'static, Result<LlmStreamEvent, LlmError>>),
    Rotate,
}

#[async_trait]
impl LlmProvider for ProviderStack {
    fn id(&self) -> &str {
        &self.primary.0
    }

    fn name(&self) -> &str {
        &self.primary.0
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        self.primary.1.capabilities()
    }

    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<LlmStreamEvent, LlmError>>, LlmError> {
        // Layer 1: 试 primary
        match self.try_provider(&self.primary, &request, 0).await? {
            ProviderAttempt::Success(stream) => Ok(stream),
            ProviderAttempt::Rotate => {
                // Layer 2: 按顺序切 fallbacks
                let total = self.fallbacks.len();
                for (i, fallback) in self.fallbacks.iter().enumerate() {
                    tracing::info!(
                        provider = %fallback.0,
                        index = i + 1,
                        total,
                        "[failover] rotating to fallback"
                    );
                    match self.try_provider(fallback, &request, 0).await? {
                        ProviderAttempt::Success(stream) => return Ok(stream),
                        ProviderAttempt::Rotate => continue,
                    }
                }
                // Layer 3 缺失, 全部失败 → 统一错误
                Err(self.all_failed_error())
            }
        }
    }
}

/// 全空 config 兜底: 构造一个空壳 provider. 实际不会被 stream_completion
/// 调, 因为 entries 为空时 `try_provider` 不会进入, `stream_completion` 会在
/// Layer 1 报 `all_failed_error` 终止. 这里只是满足类型 (无 unwrap).
fn dummy_provider(name: &str) -> Arc<dyn LlmProvider> {
    use crate::provider::create_provider;
    Arc::from(create_provider(name, &ResolvedProviderConfig {
        api_key: String::new(),
        model: "dummy".into(),
        base_url: String::new(),
        temperature: None,
        max_tokens: None,
    }))
}

// ─── 单元测试 ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StopReason;
    use futures::StreamExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    /// MockProvider: 头 N 次返指定 kind 的 LlmError, 之后返空 success stream.
    struct MockProvider {
        id: String,
        remaining_failures: Arc<AtomicU32>,
        fail_with: LlmErrorKind,
        /// 调用次数 (测试断言用, 不在 mock 行为里用)
        call_count: Arc<AtomicU32>,
    }

    impl MockProvider {
        fn new(id: &str, fail_times: u32, fail_with: LlmErrorKind) -> Self {
            Self {
                id: id.into(),
                remaining_failures: Arc::new(AtomicU32::new(fail_times)),
                fail_with,
                call_count: Arc::new(AtomicU32::new(0)),
            }
        }

        fn call_count(&self) -> Arc<AtomicU32> {
            self.call_count.clone()
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn id(&self) -> &str { &self.id }
        fn name(&self) -> &str { &self.id }
        fn capabilities(&self) -> &ProviderCapabilities {
            // 偷懒: 每次返回全新静态 — 测试不验证 capabilities 字段内容
            static CAPS: std::sync::OnceLock<ProviderCapabilities> = std::sync::OnceLock::new();
            CAPS.get_or_init(|| ProviderCapabilities {
                streaming: true,
                thinking: false,
                tool_use: false,
                max_tokens: Some(4096),
                max_input_tokens: Some(8192),
                supports_system_prompt: true,
                supports_cache_control: false,
                supports_image_input: false,
            })
        }

        async fn stream_completion(
            &self,
            _req: CompletionRequest,
        ) -> Result<BoxStream<'static, Result<LlmStreamEvent, LlmError>>, LlmError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let n = self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
            if n > 0 {
                return Err(match self.fail_with {
                    LlmErrorKind::RateLimit => LlmError::RateLimitExceeded {
                        provider: self.id.clone(),
                        retry_after: Some(Duration::from_millis(0)),
                        kind: LlmErrorKind::RateLimit,
                    },
                    LlmErrorKind::AuthPermanent => LlmError::AuthenticationError {
                        provider: self.id.clone(),
                        message: "revoked".into(),
                        kind: LlmErrorKind::AuthPermanent,
                    },
                    LlmErrorKind::Overloaded => LlmError::ApiError {
                        provider: self.id.clone(),
                        status: 503,
                        message: "overloaded".into(),
                        kind: LlmErrorKind::Overloaded,
                    },
                    LlmErrorKind::ServerError => LlmError::ApiError {
                        provider: self.id.clone(),
                        status: 500,
                        message: "internal error".into(),
                        kind: LlmErrorKind::ServerError,
                    },
                    _ => LlmError::ApiError {
                        provider: self.id.clone(),
                        status: 500,
                        message: "test mock".into(),
                        kind: LlmErrorKind::Unknown,
                    },
                });
            }
            // 成功: 返空 stream + EndTurn
            let events: Vec<Result<LlmStreamEvent, LlmError>> =
                vec![Ok(LlmStreamEvent::Stop(StopReason::EndTurn))];
            Ok(Box::pin(futures::stream::iter(events)))
        }
    }

    fn make_entry(id: &str, api_key: &str) -> (String, ResolvedProviderConfig, Box<dyn LlmProvider>) {
        (
            id.into(),
            ResolvedProviderConfig {
                api_key: api_key.into(),
                model: "test-model".into(),
                base_url: "http://test".into(),
                temperature: None,
                max_tokens: None,
            },
            Box::new(MockProvider::new(id, 0, LlmErrorKind::Unknown)),
        )
    }

    fn make_entry_with_mock(
        id: &str,
        api_key: &str,
        fail_times: u32,
        fail_with: LlmErrorKind,
    ) -> (String, ResolvedProviderConfig, Box<dyn LlmProvider>, Arc<AtomicU32>) {
        let mock = MockProvider::new(id, fail_times, fail_with);
        let calls = mock.call_count();
        let entry = (
            id.into(),
            ResolvedProviderConfig {
                api_key: api_key.into(),
                model: "test-model".into(),
                base_url: "http://test".into(),
                temperature: None,
                max_tokens: None,
            },
            Box::new(mock) as Box<dyn LlmProvider>,
        );
        (entry.0, entry.1, entry.2, calls)
    }

    fn default_request() -> CompletionRequest {
        CompletionRequest {
            system: None,
            messages: vec![],
            tools: vec![],
            tool_choice: crate::types::ToolChoice::Auto,
            max_tokens: Some(1024),
            temperature: None,
            thinking: crate::types::ThinkingConfig::Disabled,
            stop_sequences: vec![],
        }
    }

    // ── 1: 第一次就成功, 不进 retry ──

    #[tokio::test]
    async fn test_succeeds_on_first_try() {
        let (name, cfg, p, calls) = make_entry_with_mock("primary", "key1", 0, LlmErrorKind::RateLimit);
        let stack = ProviderStack::new(vec![(name, cfg, p)], "primary", 2);
        let mut stream = stack.stream_completion(default_request()).await.unwrap();
        // 消耗 stream 验证可读
        let _ = stream.next().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "只调 1 次");
    }

    // ── 2: Layer 1 重试, 同 provider RateLimit 2 次后第 3 次 Ok ──

    #[tokio::test]
    async fn test_retries_same_provider_on_rate_limit() {
        let (name, cfg, p, calls) = make_entry_with_mock("primary", "key1", 2, LlmErrorKind::RateLimit);
        let stack = ProviderStack::new(vec![(name, cfg, p)], "primary", 2);
        let mut stream = stack.stream_completion(default_request()).await.unwrap();
        let _ = stream.next().await;
        assert_eq!(calls.load(Ordering::SeqCst), 3, "primary 2 次失败 + 1 次成功 = 3 次");
    }

    // ── 3: Layer 1 耗尽 → 切 Layer 2 fallback ──

    #[tokio::test]
    async fn test_rotates_to_fallback_after_max_retries() {
        // primary: 3 次失败 (max_retries=2, attempt 0/1 retry, attempt 2 → Rotate)
        let (p_name, p_cfg, p_prov, p_calls) = make_entry_with_mock("primary", "key1", 3, LlmErrorKind::RateLimit);
        // fallback: 0 次失败, 1 次成功
        let (f_name, f_cfg, f_prov, f_calls) = make_entry_with_mock("fallback", "key2", 0, LlmErrorKind::RateLimit);

        let stack = ProviderStack::new(
            vec![(p_name, p_cfg, p_prov), (f_name, f_cfg, f_prov)],
            "primary", 2,
        );
        let mut stream = stack.stream_completion(default_request()).await.unwrap();
        let _ = stream.next().await;
        assert_eq!(p_calls.load(Ordering::SeqCst), 3, "primary 3 次");
        assert_eq!(f_calls.load(Ordering::SeqCst), 1, "fallback 1 次成功");
    }

    // ── 4: 构造过滤空 api_key ──

    #[tokio::test]
    async fn test_skips_providers_with_empty_api_key() {
        // 3 entries: p1 valid, p2 空, p3 valid. active=p1.
        let (n1, c1, p1) = make_entry("p1", "key1");
        let (n2, c2, p2) = make_entry("p2", "");
        let (n3, c3, p3) = make_entry("p3", "key3");
        let stack = ProviderStack::new(
            vec![(n1, c1, p1), (n2, c2, p2), (n3, c3, p3)],
            "p1", 2,
        );
        assert_eq!(stack.primary.0, "p1");
        assert_eq!(stack.fallbacks.len(), 1, "p2 空 key 跳过, 只剩 p3");
        assert_eq!(stack.fallbacks[0].0, "p3");
    }

    // ── 5: 全部 provider 失败 → AllProvidersFailed ──

    #[tokio::test]
    async fn test_all_providers_fail_returns_all_providers_failed() {
        let (p_name, p_cfg, p_prov, p_calls) = make_entry_with_mock("primary", "key1", 3, LlmErrorKind::RateLimit);
        let (f_name, f_cfg, f_prov, f_calls) = make_entry_with_mock("fallback", "key2", 3, LlmErrorKind::RateLimit);

        let stack = ProviderStack::new(
            vec![(p_name, p_cfg, p_prov), (f_name, f_cfg, f_prov)],
            "primary", 2,
        );
        // BoxStream 没有 Debug, 不能用 unwrap_err; 用 match
        match stack.stream_completion(default_request()).await {
            Err(LlmError::ApiError { kind, message, .. }) => {
                assert_eq!(kind, LlmErrorKind::AllProvidersFailed);
                assert!(message.contains("all 2 providers"), "message = {message}");
            }
            Err(other) => panic!("expected ApiError with AllProvidersFailed, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
        assert_eq!(p_calls.load(Ordering::SeqCst), 3);
        assert_eq!(f_calls.load(Ordering::SeqCst), 3);
    }

    // ── 6: 致命错误 (AuthPermanent) 立即 abort, 不包成 AllProvidersFailed ──

    #[tokio::test]
    async fn test_aborts_on_fatal_kind() {
        let (p_name, p_cfg, p_prov, p_calls) = make_entry_with_mock("primary", "key1", 1, LlmErrorKind::AuthPermanent);
        let (f_name, f_cfg, f_prov, f_calls) = make_entry_with_mock("fallback", "key2", 0, LlmErrorKind::Unknown);

        let stack = ProviderStack::new(
            vec![(p_name, p_cfg, p_prov), (f_name, f_cfg, f_prov)],
            "primary", 2,
        );
        // 原始 AuthenticationError 透传, 不包成 AllProvidersFailed
        match stack.stream_completion(default_request()).await {
            Err(LlmError::AuthenticationError { kind, message, .. }) => {
                assert_eq!(kind, LlmErrorKind::AuthPermanent);
                assert_eq!(message, "revoked");
            }
            Err(other) => panic!("expected AuthenticationError, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
        assert_eq!(p_calls.load(Ordering::SeqCst), 1, "fatal 立即终止, 只 1 次调用");
        assert_eq!(f_calls.load(Ordering::SeqCst), 0, "fallback 完全没碰");
    }
}


pub mod anthropic_compat;
pub mod deepseek;
pub mod error_classifier;
pub mod types;

use crate::config::ResolvedProviderConfig;
use crate::types::{LlmError, ProviderCapabilities};
use async_trait::async_trait;
use futures::stream::BoxStream;
use types::CompletionRequest;
use types::LlmStreamEvent;

/// Provider 类型字符串常量, 避免散落硬编码.
pub const PROVIDER_DEEPSEEK: &str = "deepseek";
pub const PROVIDER_MINIMAX: &str = "MiniMax";

/// 根据 provider 类型构造对应的 `LlmProvider` 实例.
///
/// # Arguments
///
/// * `provider_type` — 来自 `ResolvedConfig.active_provider`, 例如 `"deepseek"` / `"MiniMax"` / 其他
/// * `config` — `ResolvedProviderConfig`, 包含 api_key / base_url / model
///
/// # 当前实现
///
/// 所有 Anthropic 兼容服务 (`deepseek` / `MiniMax` / 未知自定义 base_url) 都通过
/// [`AnthropicCompatProvider`] 实现. 添加新 provider 几乎零成本:
/// 在 `~/.qianxun/config.json` 的 `providers` 加一个 section, 设置 `active_provider` 即可.
pub fn create_provider(
    provider_type: &str,
    config: &ResolvedProviderConfig,
) -> Box<dyn LlmProvider> {
    // 当前所有已知/未知 provider 都走 Anthropic 兼容协议.
    // 未来若添加 OpenAI 兼容 / Gemini 特定协议等, 在此处加 match arm 即可.
    tracing::info!(
        "[provider] creating provider: id={provider_type} model={} base_url={}",
        config.model,
        config.base_url
    );
    Box::new(anthropic_compat::AnthropicCompatProvider::new(
        provider_type.to_string(),
        config.api_key.clone(),
        config.base_url.clone(),
        config.model.clone(),
    ))
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn capabilities(&self) -> &ProviderCapabilities;

    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<LlmStreamEvent, LlmError>>, LlmError>;
}

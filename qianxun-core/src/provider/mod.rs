pub mod deepseek;
pub mod types;

use crate::config::ResolvedProviderConfig;
use crate::types::{LlmError, ProviderCapabilities};
use async_trait::async_trait;
use futures::stream::BoxStream;
use types::CompletionRequest;
use types::LlmStreamEvent;

/// Create a provider instance from resolved config.
///
/// Currently only supports "deepseek" (Anthropic-compatible API).
/// Additional providers can be added by extending this function
/// with a match on `config.provider_type` or similar dispatch.
pub fn create_provider(config: &ResolvedProviderConfig) -> Box<dyn LlmProvider> {
    Box::new(deepseek::DeepSeekProvider::new(
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

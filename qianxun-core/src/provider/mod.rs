pub mod deepseek;
pub mod types;

use crate::types::{LlmError, ProviderCapabilities};
use async_trait::async_trait;
use futures::stream::BoxStream;
use types::CompletionRequest;
use types::LlmStreamEvent;

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

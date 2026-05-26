pub mod memory;

use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ContextDocument {
    pub content: String,
    pub metadata: HashMap<String, String>,
    pub score: f64,
    pub source: String,
}

#[async_trait]
pub trait ContextProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn query(&self, query: &str, top_k: usize) -> Vec<ContextDocument>;
}

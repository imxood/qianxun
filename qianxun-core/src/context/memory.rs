use super::{ContextDocument, ContextProvider};
use async_trait::async_trait;
use std::path::PathBuf;

pub struct MemoryManager {
    pub base_dir: PathBuf,
}

impl MemoryManager {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn read(&mut self, _name: &str) -> Option<String> {
        // Phase 1: 骨架，实际文件读取在后续迭代中实现
        None
    }

    pub fn write(&mut self, _name: &str, _content: &str) {
        // Phase 1: 骨架
    }

    pub fn build_context(&mut self) -> String {
        // Phase 1: 返回空上下文
        String::new()
    }
}

#[async_trait]
impl ContextProvider for MemoryManager {
    fn name(&self) -> &str {
        "memory"
    }

    async fn query(&self, _query: &str, _top_k: usize) -> Vec<ContextDocument> {
        Vec::new()
    }
}

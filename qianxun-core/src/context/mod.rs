// pub mod memory; // 已迁移到 qianxun-memory crate

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

// ─── 搜索结果 ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub session_id: String,
    pub timestamp: String,
    pub title: String,
    pub narrative: String,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
    pub importance: u8,
    pub score: f64,
}

// ─── MemoryObserver trait ───────────────────────────────

/// 记忆观测者 —— AgentLoop 通过此 trait 与记忆子系统交互。
///
/// 实现由 `qianxun-memory` crate 提供。
#[async_trait]
pub trait MemoryObserver: Send + Sync {
    /// 记录一次工具调用观测。
    async fn observe(
        &self,
        hook_type: &str,
        tool_name: &str,
        tool_input: Option<Value>,
        tool_output: Option<&str>,
    );

    /// 构建记忆上下文字符串，按 token 预算裁剪。
    async fn build_context(&self, query: &str, token_budget: u32) -> String;

    /// 手动保存持久记忆。
    async fn remember(&self, content: &str, mem_type: &str) -> anyhow::Result<String>;

    /// 搜索历史记忆。
    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>>;

    /// 会话开始。
    async fn session_start(&self, session_id: &str, project: &str, cwd: &str);

    /// 会话结束。
    async fn session_end(&self);
}

// ─── ContextProvider trait（已有）──────────────────────

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

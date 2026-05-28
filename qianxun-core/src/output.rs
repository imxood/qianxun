use crate::types::{LlmError, StopReason, TokenUsage};
use async_trait::async_trait;

#[async_trait]
pub trait OutputSink: Send + Sync {
    async fn on_text(&self, text: &str);
    async fn on_thinking(&self, text: &str);
    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &serde_json::Value);
    async fn on_token_usage(&self, usage: &TokenUsage);
    async fn on_error(&self, error: &LlmError);
    async fn on_turn_finished(&self, reason: &StopReason, usage: &TokenUsage);
    /// 状态更新（如工具执行进度），sink 可酌情显示。
    async fn on_status(&self, status: &str) {
        // 默认空实现，兼容已有 sink
        let _ = status;
    }
}

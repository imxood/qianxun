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
        let _ = status;
    }

    /// Flush 思考内容缓冲区。sink 应将缓存的思考文本一次性输出。
    async fn on_thinking_flush(&self) {}

    /// 工具执行完成回调 (默认 no-op, 旧 sink 无需实现).
    ///
    /// engine 在 `tools.execute_async_with_filter` 之后调, 传入:
    /// - `tool_call_id`: 对应 LLM 工具调用的 id
    /// - `content`: 工具执行返回的文本内容
    /// - `is_error`: 工具是否报错
    /// - `elapsed_ms`: 工具执行耗时
    ///
    /// 新增原因: 让 daemon SSE 流能产出 `tool_result` 事件 (shared-contract §3.2
    /// 第 7 个事件). 早期 sink (TUI/CLI/ACP) 不需要更新 — 它们继承 default no-op,
    /// 由各 UI 自行从 conversation history 读 tool_result ContentBlock.
    async fn on_tool_result(
        &self,
        _tool_call_id: &str,
        _content: &str,
        _is_error: bool,
        _elapsed_ms: u64,
    ) {
    }
}

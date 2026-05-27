use async_trait::async_trait;
use qianxun_core::output::OutputSink;
use qianxun_core::types::{LlmError, StopReason, TokenUsage};

pub struct CliOutputSink;

#[async_trait]
impl OutputSink for CliOutputSink {
    async fn on_text(&self, text: &str) {
        print!("{}", text);
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }

    async fn on_thinking(&self, text: &str) {
        eprintln!("\x1b[90m[思考]\x1b[0m {}", text);
    }

    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &serde_json::Value) {
        eprintln!(
            "\x1b[36m[工具调用] {} ({})\x1b[0m {}",
            tool_name,
            tool_call_id,
            serde_json::to_string_pretty(arguments).unwrap_or_default()
        );
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        eprintln!(
            "\x1b[2m[Token] 输入: {}, 输出: {}\x1b[0m",
            usage.input, usage.output
        );
    }

    async fn on_error(&self, error: &LlmError) {
        eprintln!("\x1b[31m[错误] {}\x1b[0m", error);
    }

    async fn on_turn_finished(&self, reason: &StopReason, _usage: &TokenUsage) {
        eprintln!("\x1b[2m[回合结束] 原因: {:?}\x1b[0m", reason);
    }
}

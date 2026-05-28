use async_trait::async_trait;
use qianxun_core::output::OutputSink;
use qianxun_core::types::{LlmError, StopReason, TokenUsage};
use std::sync::Mutex;

pub struct CliOutputSink {
    thinking_buf: Mutex<String>,
    text_buf: Mutex<String>,
}

impl CliOutputSink {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for CliOutputSink {
    fn default() -> Self {
        Self {
            thinking_buf: Mutex::new(String::new()),
            text_buf: Mutex::new(String::new()),
        }
    }
}

impl CliOutputSink {
    fn flush_thinking_buf(&self) {
        let mut buf = self.thinking_buf.lock().unwrap();
        if !buf.is_empty() {
            eprintln!("\x1b[90m[思考]\x1b[0m {}", &*buf);
            buf.clear();
        }
    }

    /// 刷新 stdout 缓冲区，确保以换行结尾（避免与后续 stderr 输出搅在一行）
    fn flush_text_buf(&self) {
        let mut buf = self.text_buf.lock().unwrap();
        if !buf.is_empty() {
            use std::io::Write;
            if buf.ends_with('\n') {
                print!("{}", &*buf);
            } else {
                println!("{}", &*buf);
            }
            let _ = std::io::stdout().flush();
            buf.clear();
        }
    }
}

#[async_trait]
impl OutputSink for CliOutputSink {
    async fn on_text(&self, text: &str) {
        let mut buf = self.text_buf.lock().unwrap();
        buf.push_str(text);

        // 逐行输出：遇到 \n 就输出完整行，剩余内容继续累积
        while let Some(pos) = buf.find('\n') {
            let line = buf[..=pos].to_string();
            use std::io::Write;
            print!("{}", line);
            let _ = std::io::stdout().flush();
            buf.drain(..=pos);
        }
    }

    async fn on_thinking(&self, text: &str) {
        let mut buf = self.thinking_buf.lock().unwrap();
        buf.push_str(text);
        if buf.len() >= 200 {
            eprintln!("\x1b[90m[思考]\x1b[0m {}", &*buf);
            buf.clear();
        }
    }

    async fn on_thinking_flush(&self) {
        self.flush_text_buf();
        self.flush_thinking_buf();
    }

    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &serde_json::Value) {
        self.flush_text_buf();
        self.flush_thinking_buf();
        eprintln!(
            "\x1b[36m[工具调用] {} ({})\x1b[0m {}",
            tool_name,
            tool_call_id,
            serde_json::to_string_pretty(arguments).unwrap_or_default()
        );
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        tracing::info!("[Token] 输入: {}, 输出: {}", usage.input, usage.output);
        eprintln!(
            "\x1b[2m[Token] 输入: {}, 输出: {}\x1b[0m",
            usage.input, usage.output
        );
    }

    async fn on_error(&self, error: &LlmError) {
        self.flush_text_buf();
        self.flush_thinking_buf();
        eprintln!("\x1b[31m[错误] {}\x1b[0m", error);
    }

    async fn on_turn_finished(&self, reason: &StopReason, _usage: &TokenUsage) {
        self.flush_text_buf();
        self.flush_thinking_buf();
        eprintln!("\x1b[2m[回合结束] 原因: {:?}\x1b[0m", reason);
    }

    async fn on_status(&self, status: &str) {
        tracing::info!("[状态] {status}");
        self.flush_text_buf();
        self.flush_thinking_buf();
        eprintln!("\x1b[2m[状态] {status}\x1b[0m");
    }
}

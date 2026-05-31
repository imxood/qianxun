use async_trait::async_trait;
use console::style;
use indicatif::ProgressBar;
use qianxun_core::output::OutputSink;
use qianxun_core::types::{LlmError, StopReason, TokenUsage};
use std::sync::Mutex;

// ─── 框线字符 ──────────────────────────────────────────────

const HORIZ: &str = "─";
const TOP_L: &str = "╭";
const BOT_L: &str = "╰";
const VERT: &str = "│";

// ─── 辅助函数 ────────────────────────────────────────────────

/// 用框线包裹内容行。
fn boxed_lines(title: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    // 计算内容最大显示宽度 (忽略 ANSI)
    let max_content = lines.iter().map(|l| console::measure_text_width(l)).fold(0, usize::max);
    let title_part = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let inner_w = max_content.max(title_part.chars().count().saturating_sub(2));
    let mut out = String::new();

    // 顶框: ╭── title ──╮
    out.push_str(&format!(
        "{}{}{}\n",
        style(TOP_L).color256(236),
        title_part,
        style(HORIZ.repeat(inner_w.saturating_sub(title_part.chars().count()) + 2)).color256(236),
    ));

    // 内容行
    for line in lines {
        out.push_str(&format!("{} {line}\n", style(VERT).color256(236)));
    }

    // 底框
    out.push_str(&format!(
        "{}{}\n",
        style(BOT_L).color256(236),
        style(HORIZ.repeat(inner_w + 2)).color256(236),
    ));

    out
}

/// 将工具调用的 arguments 紧凑格式化，每行一个 key: value。
fn format_args_compact(args: &serde_json::Value) -> Vec<String> {
    match args {
        serde_json::Value::Object(map) => {
            let mut lines = Vec::new();
            for (k, v) in map {
                let val_str = format_value_compact(v);
                lines.push(format!("{}: {}", style(k).bold(), val_str));
            }
            lines
        }
        other => {
            vec![format_value_compact(other)]
        }
    }
}

/// 将 JSON value 截断为紧凑字符串。
fn format_value_compact(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => {
            if s.len() > 80 {
                let end = s.char_indices().nth(77).map(|(i, _)| i).unwrap_or(s.len());
                format!("{:?}…", &s[..end])
            } else {
                format!("{s:?}")
            }
        }
        serde_json::Value::Array(a) => {
            if a.len() > 3 {
                format!("[{} 项]", a.len())
            } else if a.is_empty() {
                "[]".into()
            } else {
                let items: Vec<String> = a.iter().map(format_value_compact).collect();
                format!("[{}]", items.join(", "))
            }
        }
        serde_json::Value::Object(o) => {
            if o.is_empty() {
                "{}".into()
            } else if o.len() == 1 {
                let (k, v) = o.iter().next().unwrap();
                format!("{{{k}: {}}}", format_value_compact(v))
            } else {
                format!("{{{}}}", o.keys().map(|k| k.as_str()).collect::<Vec<_>>().join(", "))
            }
        }
        serde_json::Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// StopReason → 中文标签
fn stop_reason_label(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "正常结束",
        StopReason::MaxTokens => "达到最大 Token",
        StopReason::StopSequence => "停止序列",
        StopReason::ToolUse => "工具调用",
        StopReason::ContentFiltered => "内容过滤",
        StopReason::Error => "错误",
        StopReason::Cancelled => "已取消",
        StopReason::Unknown(_) => "未知",
    }
}

// ─── 主数据结构 ──────────────────────────────────────────────

pub struct CliOutputSink {
    thinking_buf: Mutex<String>,
    text_buf: Mutex<String>,
    /// 可选的 spinner，在 LLM 生成期间显示。
    /// 所有终端输出前会暂停 spinner，输出后恢复。
    spinner: Mutex<Option<ProgressBar>>,
}

impl CliOutputSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// 绑定一个 spinner，输出事件会暂停/恢复它。
    pub fn attach_spinner(&self, spinner: ProgressBar) {
        *self.spinner.lock().unwrap() = Some(spinner);
    }

    /// 解绑并停止 spinner。
    pub fn detach_spinner(&self) {
        if let Some(sp) = self.spinner.lock().unwrap().take() {
            sp.finish_and_clear();
        }
    }

    /// 暂停 spinner 执行闭包（用于输出文本时避免覆盖）。
    fn suspend_spinner<F: FnOnce()>(&self, f: F) {
        // Clone 出 handle 后释放锁，避免死锁
        let sp = self.spinner.lock().unwrap().clone();
        match sp {
            Some(spinner) => spinner.suspend(f),
            None => f(),
        }
    }

    fn flush_thinking_buf(&self) {
        let mut buf = self.thinking_buf.lock().unwrap();
        if buf.is_empty() {
            return;
        }
        let total = buf.len();
        let excerpt: String = if total > 200 {
            let end = buf.char_indices().nth(197).map(|(i, _)| i).unwrap_or(buf.len());
            format!("{}…", &buf[..end])
        } else {
            buf.clone()
        };
        let excerpt = style(excerpt).color256(244).to_string();
        let summary = style(format!("(共 {total} 字符)")).color256(244).to_string();
        let lines = vec![excerpt, summary];
        let block = boxed_lines("思考过程", &lines);

        let block_clone = block.clone();
        self.suspend_spinner(move || {
            eprint!("{block_clone}");
        });

        buf.clear();
    }

    fn flush_text_buf(&self) {
        let mut buf = self.text_buf.lock().unwrap();
        if buf.is_empty() {
            return;
        }

        use std::io::Write;
        let text = if buf.ends_with('\n') {
            buf.clone()
        } else {
            format!("{}\n", buf)
        };

        self.suspend_spinner(move || {
            print!("{text}");
            let _ = std::io::stdout().flush();
        });

        buf.clear();
    }

    /// 强制 flush（在输出分割点调用）。
    pub fn force_flush(&self) {
        self.flush_text_buf();
        self.flush_thinking_buf();
    }
}

impl Default for CliOutputSink {
    fn default() -> Self {
        Self {
            thinking_buf: Mutex::new(String::new()),
            text_buf: Mutex::new(String::new()),
            spinner: Mutex::new(None),
        }
    }
}

#[async_trait]
impl OutputSink for CliOutputSink {
    async fn on_text(&self, text: &str) {
        let mut buf = self.text_buf.lock().unwrap();
        buf.push_str(text);

        // 逐行 flush
        while let Some(pos) = buf.find('\n') {
            let line = buf[..=pos].to_string();
            self.suspend_spinner(move || {
                use std::io::Write;
                print!("{}", line);
                let _ = std::io::stdout().flush();
            });
            buf.drain(..=pos);
        }
    }

    async fn on_thinking(&self, text: &str) {
        // 思考内容静默累积，不实时输出到终端
        let mut buf = self.thinking_buf.lock().unwrap();
        buf.push_str(text);
        if buf.len() >= 500 {
            tracing::debug!("[thinking accumulated {} chars]", buf.len());
        }
    }

    async fn on_thinking_flush(&self) {
        self.flush_text_buf();
        self.flush_thinking_buf();
    }

    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &serde_json::Value) {
        self.flush_text_buf();
        self.flush_thinking_buf();

        let content_lines = format_args_compact(arguments);
        let title = format!(
            "{} {}",
            style(format!("工具调用: {tool_name}")).color256(75),
            style(format!("({tool_call_id})")).color256(244),
        );
        let block = boxed_lines(&title, &content_lines);

        self.suspend_spinner(move || {
            eprint!("{block}");
        });
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        tracing::info!("[Token] 输入: {}, 输出: {}", usage.input, usage.output);
        let msg = format!(
            "  {} {}",
            style(format!("{BOT_L}─")).color256(236),
            style(format!("Token: 输入 {}, 输出 {}", usage.input, usage.output)).dim(),
        );
        self.suspend_spinner(move || {
            eprintln!("{msg}");
        });
    }

    async fn on_error(&self, error: &LlmError) {
        self.flush_text_buf();
        self.flush_thinking_buf();
        let lines = vec![style(format!("{error}")).color256(196).to_string()];
        let block = boxed_lines(&format!("{}", style("错误").color256(196)), &lines);

        self.suspend_spinner(move || {
            eprint!("{block}");
        });
    }

    async fn on_turn_finished(&self, reason: &StopReason, _usage: &TokenUsage) {
        self.flush_text_buf();
        self.flush_thinking_buf();

        let label = stop_reason_label(reason);
        let msg = format!(
            "  {} {}  {}",
            style(format!("{BOT_L}─")).color256(236),
            style(label).color256(244),
            style(format!("({reason:?})")).dim(),
        );
        self.suspend_spinner(move || {
            eprintln!("{msg}");
        });
    }

    async fn on_status(&self, status: &str) {
        tracing::info!("[status] {status}");

        // 如果 spinner 存在，更新其消息
        {
            let guard = self.spinner.lock().unwrap();
            if let Some(ref spinner) = *guard {
                if let Some(tool_name) = status.strip_prefix("执行工具: ") {
                    spinner.set_message(format!("⚡ {tool_name}"));
                } else if status == "工具执行完成，继续请求 LLM..." {
                    spinner.set_message("继续思考...");
                } else {
                    spinner.set_message(status.to_string());
                }
                return; // spinner 状态下只更新消息，不输出
            }
        }

        // 无 spinner 时，直接输出状态
        self.suspend_spinner(move || {
            if let Some(tool_name) = status.strip_prefix("执行工具: ") {
                eprintln!("  {} {}", style(format!("{BOT_L}─")).color256(236), style(format!("执行工具: {tool_name}")).color256(75));
            } else {
                eprintln!("  {} {}", style(format!("{BOT_L}─")).color256(236), style(status).color256(244));
            }
        });
    }
}

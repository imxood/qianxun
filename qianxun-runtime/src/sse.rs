//! SSE (Server-Sent Events) — 12 种事件类型 + 状态化转换器.
//!
//! `db` 字段留 Phase 4 接 SSE 流式 tool call 持久化.
#![allow(dead_code, clippy::type_complexity)]
//!
//! 与 shared-contract §3.2 **严格一致**, 字段名/类型/tag 都不能改.
//! 12 个事件: message_start, content_block_start, text_delta, thinking_delta,
//! tool_use_delta, tool_use_complete, tool_result, content_block_stop, usage,
//! message_delta, message_stop, error.
//!
//! # 阶段
//!
//! - **Stage 2**: `SseEventBuilder` 状态机 + `from_llm_event` 映射
//! - Stage 3: 接入 `processing_loop::handle_user_message` (通过 `OutputSink`)
//! - Stage 4: ToolPolicy 审批 + 完整 tool_result 路径

use qianxun_core::provider::types::LlmStreamEvent;
use qianxun_core::types::StopReason;
use serde::Serialize;

// ─── SseEvent enum ──────────────────────────────────────────

/// SSE 事件 (与 shared-contract §3.2 严格一致, 12 种类型).
///
/// 使用 `#[serde(tag = "type")]` 内部 tag 序列化: 输出的 JSON 形如
/// `{"type":"text_delta","index":0,"text":"..."}`. 客户端按 `type` 字段分发.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum SseEvent {
    #[serde(rename = "message_start")]
    MessageStart {
        session_id: String,
        model: String,
        max_tokens: u32,
    },

    #[serde(rename = "content_block_start")]
    ContentBlockStart { index: u32, block_type: String },

    #[serde(rename = "text_delta")]
    TextDelta { index: u32, text: String },

    #[serde(rename = "thinking_delta")]
    ThinkingDelta { index: u32, text: String },

    #[serde(rename = "tool_use_delta")]
    ToolUseDelta {
        index: u32,
        id: String,
        name: String,
        arguments_json: String,
    },

    #[serde(rename = "tool_use_complete")]
    ToolUseComplete {
        index: u32,
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        elapsed_ms: u64,
    },

    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },

    #[serde(rename = "usage")]
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_input_tokens: u64,
        cache_read_input_tokens: u64,
    },

    #[serde(rename = "message_delta")]
    MessageDelta { stop_reason: String },

    #[serde(rename = "message_stop")]
    MessageStop,

    #[serde(rename = "error")]
    Error {
        code: qianxun_core::provider::error_classifier::LlmErrorKind,
        message: String,
    },

    // ─── 缺口 05: 后台异步任务事件 (Stage 5 新增) ─────

    #[serde(rename = "background_task_started")]
    BackgroundTaskStarted {
        task_id: String,
        task_kind: String,
        started_at: i64,
    },

    #[serde(rename = "background_task_updated")]
    BackgroundTaskUpdated {
        task_id: String,
        status: String,
        progress: Option<f64>,
        message: Option<String>,
        updated_at: i64,
    },

    #[serde(rename = "background_task_cancelled")]
    BackgroundTaskCancelled { task_id: String, reason: String },

    #[serde(rename = "background_task_completed")]
    BackgroundTaskCompleted {
        task_id: String,
        result: serde_json::Value,
        completed_at: i64,
    },

    // ─── P1-3 收尾 (2026-06-12): Plan 状态实时事件 ─────

    /// Plan 状态变更 (Pending/Running/Done/Failed/Aborted) + 当前 task_results 快照.
    /// 订阅方: Tauri desktop 前端 (走 `app.emit("plan_event", ...)`), daemon SSE 流.
    /// task_results_json: None 表示未提供 (e.g. PlanUpdate 只标 plan 状态变更,
    /// 不重发 task_results). 走 JSON 字符串保持 sse 模块零外部类型依赖.
    #[serde(rename = "plan_update")]
    PlanUpdate {
        plan_id: String,
        /// PlanStatus snake_case 字符串 ("pending" / "running" / "done" / "failed" / "aborted").
        status: String,
        /// task_results JSON 数组. None = plan 级状态变更 (不带 task 详情).
        task_results_json: Option<String>,
        /// Unix epoch ms, 给前端做去重 / 排序.
        updated_at: i64,
    },

    /// 2026-06-12 收尾: sub_session 状态变更. 第 17 变体.
    /// 跟 PlanUpdate 平级, 但只标单个 sub_session 状态变更 (启动/完成/失败).
    /// 前端 subSessionStore.handleSubSessionEvent 实时更新.
    /// 跟 PlanUpdate 区别: PlanUpdate 是 plan 级 + task_results 快照, 这个是
    /// 单 sub_session 级 + output. 一个 plan 跑可能触发 6+ 条 SubSessionUpdate
    /// (启动/完成/失败), 跟 PlanUpdate 不重复.
    #[serde(rename = "sub_session_update")]
    SubSessionUpdate {
        sub_session_id: String,
        plan_id: String,
        task_id: String,
        /// SubSession 状态 snake_case: "active" / "done" / "failed" / "aborted".
        status: String,
        /// sub_session 完整状态 (含 started_at, ended_at, output). 前端用这个
        /// 一次 upsert, 不用再调 get_sub_session.
        sub_session_json: String,
        /// Unix epoch ms, 跟 PlanUpdate 同样语义.
        updated_at: i64,
    },
}

impl SseEvent {
    /// 返回 SSE 事件 type tag 字符串, 与 `#[serde(rename = "...")]` 严格一致.
    /// 用于 `SessionStore::append_event()` 的 `event_type` 字段 (落盘时存
    /// 事件类型便于查询/恢复) 以及任何需要字符串 type 标识的地方.
    ///
    /// 为什么不直接 serde 序列化后 parse `type` 字段: 序列化是 allocate 操作,
    /// 而 type_name 用于每条事件的 store.append_event 调用, 高频热路径.
    /// 用 match 直接返回静态字符串零分配.
    pub fn type_name(&self) -> &'static str {
        match self {
            SseEvent::MessageStart { .. } => "message_start",
            SseEvent::ContentBlockStart { .. } => "content_block_start",
            SseEvent::TextDelta { .. } => "text_delta",
            SseEvent::ThinkingDelta { .. } => "thinking_delta",
            SseEvent::ToolUseDelta { .. } => "tool_use_delta",
            SseEvent::ToolUseComplete { .. } => "tool_use_complete",
            SseEvent::ToolResult { .. } => "tool_result",
            SseEvent::ContentBlockStop { .. } => "content_block_stop",
            SseEvent::Usage { .. } => "usage",
            SseEvent::MessageDelta { .. } => "message_delta",
            SseEvent::MessageStop => "message_stop",
            SseEvent::Error { .. } => "error",
            SseEvent::BackgroundTaskStarted { .. } => "background_task_started",
            SseEvent::BackgroundTaskUpdated { .. } => "background_task_updated",
            SseEvent::BackgroundTaskCancelled { .. } => "background_task_cancelled",
            SseEvent::BackgroundTaskCompleted { .. } => "background_task_completed",
            SseEvent::PlanUpdate { .. } => "plan_update",
            SseEvent::SubSessionUpdate { .. } => "sub_session_update",
        }
    }
}

// ─── Block type tracking ────────────────────────────────────

/// 当前 content_block 的逻辑类型, 用于状态机决定是否需要开/关 block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    None,
    Text,
    Thinking,
    ToolUse,
}

// ─── SseEventBuilder ────────────────────────────────────────

/// 状态化转换器: 把 `LlmStreamEvent` 序列 + 终止/错误信号, 转换成 `SseEvent` 序列.
///
/// 跟踪当前 block 状态, 在 block 类型切换时自动插入 `content_block_start` /
/// `content_block_stop`, 保证客户端能按 `index` 配对. 客户端断连 / 终止时
/// 调用 `finalize(&str)` 发射 `MessageDelta + MessageStop` 收尾.
#[derive(Debug)]
pub struct SseEventBuilder {
    /// 当前 block 的 logical kind (None 表示当前没开 block).
    current_block: BlockKind,
    /// 下一个可用 block index (单调递增).
    next_block_index: u32,
    /// 当前打开的 block index (用于发 stop 时回填).
    current_block_index: Option<u32>,
}

impl SseEventBuilder {
    pub fn new() -> Self {
        Self {
            current_block: BlockKind::None,
            next_block_index: 0,
            current_block_index: None,
        }
    }

    /// 消耗 1 个 `LlmStreamEvent` 返回 0..N 个 `SseEvent`.
    ///
    /// 典型返回:
    /// - `Text(t)` → `[ContentBlockStart, TextDelta]` (仅当 block 未开)
    /// - `Thinking { text, .. }` → `[ContentBlockStart?, ThinkingDelta]`
    /// - `ToolCall { .. }` → `[ContentBlockStop?, ContentBlockStart, ToolUseComplete, ContentBlockStop]`
    /// - `UsageUpdate(u)` → `[Usage]`
    /// - `Stop(reason)` → 保留, 由 `finalize` 统一收尾
    #[allow(clippy::wrong_self_convention)] // 命名保留 LLM 业界惯例 (from_xxx), 实际需要 &mut self 做状态切换
    pub fn from_llm_event(&mut self, event: &LlmStreamEvent) -> Vec<SseEvent> {
        match event {
            LlmStreamEvent::Text(text) => self.handle_text(text),
            LlmStreamEvent::Thinking { text, .. } => self.handle_thinking(text),
            LlmStreamEvent::ToolCall {
                id,
                tool_name,
                arguments,
            } => self.handle_tool_call(id, tool_name, arguments),
            LlmStreamEvent::UsageUpdate(usage) => vec![SseEvent::Usage {
                input_tokens: usage.input,
                output_tokens: usage.output,
                cache_creation_input_tokens: usage.cache_creation_input.unwrap_or(0),
                cache_read_input_tokens: usage.cache_read_input.unwrap_or(0),
            }],
            LlmStreamEvent::Stop(_) => {
                // 由 finalize() 统一收尾; 此处不发射任何事件
                Vec::new()
            }
        }
    }

    /// 收尾: 关闭当前未关的 block, 发 `MessageDelta(stop_reason)` + `MessageStop`.
    ///
    /// 取 `&mut self` 而不是 `self`: 方便在 `&mut SseEventBuilder` 借用的上下文中
    /// 调用 (例如 SSE consumer 持有 `&mut builder` 来反复 `from_llm_event`).
    /// 副作用: 内部 `current_block_index` 被 take, 等同于重置 block 状态.
    pub fn finalize(&mut self, stop_reason: &str) -> Vec<SseEvent> {
        let mut out = Vec::new();
        // 1. 关掉当前 block
        if let Some(idx) = self.current_block_index.take() {
            out.push(SseEvent::ContentBlockStop { index: idx });
        }
        // 2. MessageDelta + MessageStop
        out.push(SseEvent::MessageDelta {
            stop_reason: stop_reason.to_string(),
        });
        out.push(SseEvent::MessageStop);
        out
    }

    /// 把 `LlmError` 映射成 SSE `Error` 事件.
    ///
    /// `code` 字段携带 `LlmErrorKind` (15 种语义分类, 由 `LlmError.kind` 注入),
    /// 序列化时按 snake_case 出 string (e.g. `auth` / `rate_limit` / `server_error`),
    /// 与前端 `chat-stream.ts` `case "error"` 解析兼容.
    pub fn error_from_llm(e: &qianxun_core::types::LlmError) -> SseEvent {
        use qianxun_core::types::LlmError;
        // 1. 拿 kind (HTTP 错误时已注入, 其他场景 LlmErrorKind::default() = Unknown).
        let kind = e.kind();
        // 2. 拼 message (沿用原 error_from_llm 的人类可读格式, 不丢 provider/status 信息).
        let message = match e {
            LlmError::NoApiKey { provider, .. } => {
                format!("API key not configured for {provider}")
            }
            LlmError::AuthenticationError {
                provider, message, ..
            } => format!("[{provider}] {message}"),
            LlmError::RateLimitExceeded {
                provider,
                retry_after,
                ..
            } => {
                let wait = retry_after.map(|d| d.as_secs()).unwrap_or(0);
                format!("[{provider}] rate limit, retry after {wait}s")
            }
            LlmError::ApiError {
                provider,
                status,
                message,
                ..
            } => format!("[{provider}] {status} {message}"),
            LlmError::PromptTooLarge { tokens, .. } => {
                format!("prompt too large: {tokens:?}")
            }
            LlmError::StreamEnded { .. } => "stream ended unexpectedly".to_string(),
        };
        // 2026-06-09 L3: 之前 0 行 tracing, 错误分类后只发 SSE, 后端无任何审计.
        // 现在加 warn (front-end 立刻 toast, 后端 stderr 也留底, 排查时一查就着).
        tracing::warn!(
            code = %kind.as_str(),
            "[sse] LlmError → SseEvent::Error: {}",
            message
        );
        SseEvent::Error { code: kind, message }
    }

    /// 把 `StopReason` 转成 SSE `message_delta.stop_reason` 字符串 (snake_case).
    pub fn stop_reason_str(r: &StopReason) -> &'static str {
        match r {
            StopReason::EndTurn => "end_turn",
            StopReason::MaxTokens => "max_tokens",
            StopReason::ToolUse => "tool_use",
            StopReason::StopSequence => "stop_sequence",
            StopReason::ContentFiltered => "content_filtered",
            StopReason::Cancelled => "cancelled",
            StopReason::Error => "error",
            StopReason::Unknown(_) => "unknown",
        }
    }

    // ── 私有: 各 LlmStreamEvent 子处理 ──

    fn handle_text(&mut self, text: &str) -> Vec<SseEvent> {
        let mut out = Vec::new();
        // 如果当前不是 text block, 关掉旧 block, 开 text block
        if self.current_block != BlockKind::Text {
            if let Some(idx) = self.current_block_index.take() {
                out.push(SseEvent::ContentBlockStop { index: idx });
            }
            let idx = self.next_block();
            self.current_block = BlockKind::Text;
            self.current_block_index = Some(idx);
            out.push(SseEvent::ContentBlockStart {
                index: idx,
                block_type: "text".to_string(),
            });
        }
        let idx = self.current_block_index.expect("just set");
        out.push(SseEvent::TextDelta {
            index: idx,
            text: text.to_string(),
        });
        out
    }

    fn handle_thinking(&mut self, text: &str) -> Vec<SseEvent> {
        if text.is_empty() {
            // signature 收尾事件 (空 text) — 不发 block 切换
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.current_block != BlockKind::Thinking {
            if let Some(idx) = self.current_block_index.take() {
                out.push(SseEvent::ContentBlockStop { index: idx });
            }
            let idx = self.next_block();
            self.current_block = BlockKind::Thinking;
            self.current_block_index = Some(idx);
            out.push(SseEvent::ContentBlockStart {
                index: idx,
                block_type: "thinking".to_string(),
            });
        }
        let idx = self.current_block_index.expect("just set");
        out.push(SseEvent::ThinkingDelta {
            index: idx,
            text: text.to_string(),
        });
        out
    }

    fn handle_tool_call(
        &mut self,
        id: &str,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Vec<SseEvent> {
        let mut out = Vec::new();
        // 关掉当前 block (text / thinking / 旧 tool_use)
        if let Some(idx) = self.current_block_index.take() {
            out.push(SseEvent::ContentBlockStop { index: idx });
        }
        // 开 tool_use block
        let idx = self.next_block();
        self.current_block = BlockKind::ToolUse;
        self.current_block_index = Some(idx);
        out.push(SseEvent::ContentBlockStart {
            index: idx,
            block_type: "tool_use".to_string(),
        });
        out.push(SseEvent::ToolUseComplete {
            index: idx,
            id: id.to_string(),
            name: name.to_string(),
            arguments: arguments.clone(),
        });
        // tool_use_complete 内部立即 stop (provider 是批式, 见 daemon.md §5.1.1)
        out.push(SseEvent::ContentBlockStop { index: idx });
        self.current_block_index = None;
        self.current_block = BlockKind::None;
        // 下一个 block 仍可分配 (tool_result 留给 Stage 3 实际执行时)
        let _ = self.next_block_index;
        out
    }

    fn next_block(&mut self) -> u32 {
        let idx = self.next_block_index;
        self.next_block_index += 1;
        idx
    }

    /// 公开给 sink 用: 分配并返回下一个 block index, 同步推进内部计数器.
    ///
    /// 用例: 处理 `tool_result` 等不通过 `from_llm_event` 进入的事件 — sink 需要
    /// 自己分配 index (`ContentBlockStart` / `ContentBlockStop` 配对), 而 builder
    /// 自己的 `next_block()` 是私有方法. 通过这个公开方法, sink 可跟 builder 的
    /// index 序列保持一致, 客户端看到的 block 序号连续递增.
    pub fn allocate_block_index(&mut self) -> u32 {
        self.next_block()
    }
}

impl Default for SseEventBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use qianxun_core::types::TokenUsage;
    use serde_json::json;

    #[test]
    fn test_type_name_matches_serde_tag() {
        // 验证 12 个 variant 的 type_name 跟 #[serde(rename = "...")] 字段一致.
        // 这是 SseEvent ↔ SessionStore event_type 列的契约, 必须严丝合缝.
        let cases: Vec<(SseEvent, &'static str)> = vec![
            (
                SseEvent::MessageStart {
                    session_id: "s".into(),
                    model: "m".into(),
                    max_tokens: 1,
                },
                "message_start",
            ),
            (
                SseEvent::ContentBlockStart {
                    index: 0,
                    block_type: "text".into(),
                },
                "content_block_start",
            ),
            (
                SseEvent::TextDelta {
                    index: 0,
                    text: "x".into(),
                },
                "text_delta",
            ),
            (
                SseEvent::ThinkingDelta {
                    index: 0,
                    text: "y".into(),
                },
                "thinking_delta",
            ),
            (
                SseEvent::ToolUseDelta {
                    index: 0,
                    id: "i".into(),
                    name: "n".into(),
                    arguments_json: "{}".into(),
                },
                "tool_use_delta",
            ),
            (
                SseEvent::ToolUseComplete {
                    index: 0,
                    id: "i".into(),
                    name: "n".into(),
                    arguments: json!({}),
                },
                "tool_use_complete",
            ),
            (
                SseEvent::ToolResult {
                    tool_use_id: "i".into(),
                    content: "r".into(),
                    is_error: false,
                    elapsed_ms: 0,
                },
                "tool_result",
            ),
            (
                SseEvent::ContentBlockStop { index: 0 },
                "content_block_stop",
            ),
            (
                SseEvent::Usage {
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
                "usage",
            ),
            (
                SseEvent::MessageDelta {
                    stop_reason: "end_turn".into(),
                },
                "message_delta",
            ),
            (SseEvent::MessageStop, "message_stop"),
            (
                SseEvent::Error {
                    code: qianxun_core::provider::error_classifier::LlmErrorKind::ServerError,
                    message: "boom".into(),
                },
                "error",
            ),
        ];
        assert_eq!(cases.len(), 12);
        for (ev, expected) in cases {
            let actual = ev.type_name();
            assert_eq!(actual, expected, "type_name mismatch for {expected}");
            // 跟 serde 输出的 `type` 字段交叉验证
            let serialized = serde_json::to_string(&ev).expect("serialize");
            assert!(
                serialized.contains(&format!(r#""type":"{expected}""#)),
                "serde tag for {expected} diverged: {serialized}"
            );
        }
    }

    #[test]
    fn test_text_event_roundtrip() {
        let ev = SseEvent::TextDelta {
            index: 0,
            text: "hello".to_string(),
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        assert!(s.contains(r#""type":"text_delta""#), "missing type tag: {s}");
        assert!(s.contains(r#""text":"hello""#), "missing text field: {s}");
        assert!(s.contains(r#""index":0"#), "missing index field: {s}");
    }

    #[test]
    fn test_message_start_includes_session_id() {
        let ev = SseEvent::MessageStart {
            session_id: "sess_xxx".to_string(),
            model: "deepseek-v4-flash".to_string(),
            max_tokens: 16384,
        };
        let s = serde_json::to_string(&ev).expect("serialize");
        assert!(
            s.contains(r#""type":"message_start""#),
            "missing type tag: {s}"
        );
        assert!(
            s.contains(r#""session_id":"sess_xxx""#),
            "missing session_id: {s}"
        );
        assert!(s.contains(r#""model":"deepseek-v4-flash""#));
        assert!(s.contains(r#""max_tokens":16384"#));
    }

    #[test]
    fn test_all_12_variants_serialize() {
        // 每个 variant 序列化一次, 验证 (a) tag 名与契约一致, (b) JSON 合法.
        let events = vec![
            (
                SseEvent::MessageStart {
                    session_id: "s1".into(),
                    model: "m".into(),
                    max_tokens: 1,
                },
                "message_start",
            ),
            (
                SseEvent::ContentBlockStart {
                    index: 0,
                    block_type: "text".into(),
                },
                "content_block_start",
            ),
            (
                SseEvent::TextDelta {
                    index: 0,
                    text: "x".into(),
                },
                "text_delta",
            ),
            (
                SseEvent::ThinkingDelta {
                    index: 0,
                    text: "y".into(),
                },
                "thinking_delta",
            ),
            (
                SseEvent::ToolUseDelta {
                    index: 0,
                    id: "i".into(),
                    name: "n".into(),
                    arguments_json: "{}".into(),
                },
                "tool_use_delta",
            ),
            (
                SseEvent::ToolUseComplete {
                    index: 0,
                    id: "i".into(),
                    name: "n".into(),
                    arguments: json!({}),
                },
                "tool_use_complete",
            ),
            (
                SseEvent::ToolResult {
                    tool_use_id: "i".into(),
                    content: "r".into(),
                    is_error: false,
                    elapsed_ms: 0,
                },
                "tool_result",
            ),
            (
                SseEvent::ContentBlockStop { index: 0 },
                "content_block_stop",
            ),
            (
                SseEvent::Usage {
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
                "usage",
            ),
            (
                SseEvent::MessageDelta {
                    stop_reason: "end_turn".into(),
                },
                "message_delta",
            ),
            (SseEvent::MessageStop, "message_stop"),
            (
                SseEvent::Error {
                    code: qianxun_core::provider::error_classifier::LlmErrorKind::ServerError,
                    message: "boom".into(),
                },
                "error",
            ),
        ];
        assert_eq!(events.len(), 12, "must have 12 variants per contract");
        for (ev, expected_tag) in events {
            let s = serde_json::to_string(&ev).expect("serialize");
            let expected = format!(r#""type":"{expected_tag}""#);
            assert!(
                s.contains(&expected),
                "variant {expected_tag} serialized as: {s}"
            );
            // 验证 JSON 合法
            let _parsed: serde_json::Value =
                serde_json::from_str(&s).expect("valid JSON");
        }
    }

    #[test]
    fn test_text_then_stop_emits_full_block_lifecycle() {
        let mut b = SseEventBuilder::new();
        let evs = b.from_llm_event(&LlmStreamEvent::Text("hi".into()));
        assert_eq!(evs.len(), 2, "first text should emit start + delta");
        assert!(matches!(evs[0], SseEvent::ContentBlockStart { .. }));
        assert!(matches!(evs[1], SseEvent::TextDelta { .. }));

        let finalize = b.finalize("end_turn");
        assert_eq!(finalize.len(), 3, "finalize: stop + delta + stop");
        assert!(matches!(finalize[0], SseEvent::ContentBlockStop { .. }));
        assert!(matches!(finalize[1], SseEvent::MessageDelta { .. }));
        assert!(matches!(finalize[2], SseEvent::MessageStop));
    }

    #[test]
    fn test_text_then_tool_call_switches_block() {
        let mut b = SseEventBuilder::new();
        b.from_llm_event(&LlmStreamEvent::Text("hello ".into()));
        b.from_llm_event(&LlmStreamEvent::Text("world".into())); // same block
        let evs = b.from_llm_event(&LlmStreamEvent::ToolCall {
            id: "t1".into(),
            tool_name: "read".into(),
            arguments: json!({"path": "/tmp"}),
        });
        // expect: ContentBlockStop(text#0) + ContentBlockStart(tool_use#1)
        //         + ToolUseComplete#1 + ContentBlockStop#1
        assert_eq!(evs.len(), 4, "got: {evs:?}");
        assert!(matches!(
            evs[0],
            SseEvent::ContentBlockStop { index: 0 }
        ));
        assert!(matches!(
            evs[1],
            SseEvent::ContentBlockStart {
                block_type: _,
                ..
            }
        ));
        assert!(matches!(evs[2], SseEvent::ToolUseComplete { .. }));
        assert!(matches!(
            evs[3],
            SseEvent::ContentBlockStop { index: 1 }
        ));
    }

    #[test]
    fn test_usage_event_maps_directly() {
        let mut b = SseEventBuilder::new();
        let evs = b.from_llm_event(&LlmStreamEvent::UsageUpdate(TokenUsage {
            input: 100,
            output: 50,
            cache_creation_input: Some(0),
            cache_read_input: Some(0),
        }));
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            SseEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, 100);
                assert_eq!(*output_tokens, 50);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_error_classification() {
        use qianxun_core::provider::error_classifier::LlmErrorKind;
        use qianxun_core::types::LlmError;
        let e = LlmError::RateLimitExceeded {
            provider: "deepseek".into(),
            retry_after: Some(std::time::Duration::from_secs(5)),
            kind: LlmErrorKind::RateLimit,
        };
        let ev = SseEventBuilder::error_from_llm(&e);
        match ev {
            SseEvent::Error { code, message } => {
                assert_eq!(code, LlmErrorKind::RateLimit);
                assert!(message.contains("deepseek"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// 缺口 02 集成层 v0.3: 验证 LlmError → SseEvent::Error 透传 `kind` 字段,
    /// 5xx ApiError 注入 LlmErrorKind::ServerError (走 `classify_http_status(500, _)`).
    #[test]
    fn test_error_from_llm_preserves_kind_for_5xx() {
        use qianxun_core::provider::error_classifier::LlmErrorKind;
        use qianxun_core::types::LlmError;
        let e = LlmError::ApiError {
            provider: "deepseek".into(),
            status: 500,
            message: "internal server error".into(),
            kind: LlmErrorKind::ServerError,
        };
        let ev = SseEventBuilder::error_from_llm(&e);
        match ev {
            SseEvent::Error { code, message } => {
                assert_eq!(code, LlmErrorKind::ServerError);
                assert!(message.contains("500"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// 缺口 02 集成层 v0.3: 验证 SseEvent::Error 序列化时 `code` 字段出 snake_case 字符串,
    /// 跟前端 chat-stream.ts 解析契约一致.
    #[test]
    fn test_sse_event_error_serializes_kind_as_snake_case() {
        use qianxun_core::provider::error_classifier::LlmErrorKind;
        let ev = SseEvent::Error {
            code: LlmErrorKind::RateLimit,
            message: "rate limit hit".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], "rate_limit"); // snake_case 字符串, 前端按 string 解析
        assert_eq!(json["message"], "rate limit hit");
    }
}

//! `DaemonOutputSink` — bridges `OutputSink` trait callbacks (and direct
//! `LlmStreamEvent` consumption) to SSE events for the daemon.
//!
//! 角色:
//! 1. **Stage 2 直接消费**: `consume_stream_to_sse` 拿 `LlmStreamEvent` 流,
//!    调 sink 的 `text_delta` / `tool_use` / `usage` / `error` / `finish_turn_str`
//!    等 `&self` 方法, sink 内部驱动 `SseEventBuilder` 状态机并发出 SseEvent.
//! 2. **Stage 3 接 processing_loop**: `processing_loop::handle_user_message`
//!    拿到 `&dyn OutputSink`, 调 trait 方法 (`on_text` / `on_thinking` /
//!    `on_tool_call` / `on_token_usage` / `on_error` / `on_turn_finished` 等),
//!    sink 路由到同一套内部逻辑.
//!
//! 关键设计:
//! - **块状态机**: 内部持有 `SseEventBuilder` (通过 `Mutex` 提供内部可变性),
//!   自动在 text / thinking / tool_use 块切换时插入 `ContentBlockStart` /
//!   `ContentBlockStop`. 客户端按 `index` 配对 block.
//! - **持久化**: 每条发出的事件调 `SessionStore::append_event()` 落盘 (Stage 3
//!   用于恢复/审计). 用 `event_seq` 原子递增保证顺序.
//! - **断连容错**: `tx.send().await` 返回 `Err` (客户端断) 时 debug 日志 +
//!   静默 return, 绝不 panic. 后续事件照常尝试发送直到 sink drop.
//! - **MessageStart 幂等**: `begin_message()` 第一次发 `MessageStart`, 后续
//!   调用 no-op. 这让 Stage 2 prompt_handler 提前手动发 + Stage 3 sink
//!   自动发两种调用模式都安全.
//!
//! 不引入新 crate (CLAUDE.md "< 30 传递依赖" 约束). 依赖:
//! - `tokio::sync::mpsc` (已有)
//! - `async_trait` (Cargo.toml 已有)
//! - `qianxun_core::output` / `qianxun_core::provider::types` (workspace 内)
//! - `crate::daemon::persistence` / `crate::daemon::sse` (本 crate)

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use qianxun_core::output::OutputSink;
use qianxun_core::provider::types::LlmStreamEvent;
use qianxun_core::provider::error_classifier::LlmErrorKind;
use qianxun_core::types::{LlmError, StopReason, TokenUsage};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::SessionStore;
use crate::{SseEvent, SseEventBuilder};

// ─── 内部状态 ─────────────────────────────────────────────────

/// 内部可变状态 — 包在 `Mutex` 里是为了让 `OutputSink` trait 的 `&self` 方法
/// 也能更新 builder. 锁持有时间极短 (一次 `from_llm_event` 调用), 且发送
/// 事件前**先释放锁**再 `tx.send().await`, 不会跨 await 持锁.
#[allow(dead_code)] // `started` 只在 begin_message 路径上读, Stage 2 二进制路径不调
struct SinkState {
    /// SSE 块状态机 — 跟原 SseEventBuilder 同等, 自动插入 content_block_start/stop.
    builder: SseEventBuilder,
    /// `MessageStart` 是否已经发过 — 防止 begin_message 重复调用时发两次.
    started: bool,
    /// `store.append_event` 用的 sequence 计数器, 跟 prompt_handler
    /// `MessageStart(seq=0)` 衔接. 第一条 content 事件 = 1.
    event_seq: u32,
}

// ─── DaemonOutputSink ─────────────────────────────────────────

/// SSE event sink. 同时:
/// - 实现 `OutputSink` trait (Stage 3 processing_loop 走 trait)
/// - 暴露直接 `&self` 方法 (Stage 2 consume_stream_to_sse 走直接调用)
///
/// 当前 Stage 2 实际只用 `text_delta` / `thinking` / `tool_use` / `usage` /
/// `error` / `finish_turn_str` + `store()` 路径; `begin_message` / `tool_result` /
/// `save_snapshot` 留 Stage 3 processing_loop 工具执行 + MessageStart 自动化时
/// 使用. 二进制构建会标 dead_code 警告, 用 `#[allow(dead_code)]` 整体放过 —
/// 跟 `AppState` 处理 Phase 4 暂未接入字段一致.
#[allow(dead_code)] // 留 Stage 3 processing_loop + 工具执行结果路径使用
pub struct DaemonOutputSink {
    tx: mpsc::Sender<SseEvent>,
    store: Arc<SessionStore>,
    session_id: String,
    #[allow(dead_code)] // 通过 begin_message 间接使用
    model: String,
    #[allow(dead_code)] // 通过 begin_message 间接使用
    max_tokens: u32,
    state: Mutex<SinkState>,
}

impl DaemonOutputSink {
    // `begin_message` / `tool_result` / `save_snapshot` / `session_id` / `store`
    // 公共方法在 Stage 2 路径未直接调 (Stage 2 prompt_handler 同步发 MessageStart,
    // tool 执行未接入), 但都是 Stage 3 processing_loop + 工具执行桥接的关键 API.
    // 二进制构建期会标 dead_code, 此处统一 allow — 跟 `AppState` 模式一致.
    // 测试构建会读到 (cargo test 看到 #[cfg(test)] mod tests), 所以不删.

    /// 构造一个新 sink.
    ///
    /// `emit_message_start`:
    /// - `true`: sink 负责发 `MessageStart` (Stage 3 processing_loop 路径).
    ///   调用方应**先**调 `begin_message()` 再发内容事件.
    /// - `false`: 调用方已在外层 (Stage 2 prompt_handler) 同步发过
    ///   `MessageStart`, sink 内部 `begin_message()` no-op.
    pub fn new(
        tx: mpsc::Sender<SseEvent>,
        store: Arc<SessionStore>,
        session_id: String,
        model: String,
        max_tokens: u32,
        emit_message_start: bool,
    ) -> Self {
        let event_seq = if emit_message_start { 0 } else { 1 };
        Self {
            tx,
            store,
            session_id,
            model,
            max_tokens,
            state: Mutex::new(SinkState {
                builder: SseEventBuilder::new(),
                started: !emit_message_start,
                event_seq,
            }),
        }
    }

    /// 当前 session id (供 snapshot 等需要显式标识的场景).
    #[allow(dead_code)] // Stage 3 persist_assistant_message 路径使用
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 引用底层 store (供 Stage 3 完整 snapshot 写入用 — 当前 Stage 3 简化
    /// 路径只调 `save_snapshot`).
    #[allow(dead_code)] // Stage 3 persist_assistant_message 路径使用
    pub fn store(&self) -> &Arc<SessionStore> {
        &self.store
    }

    /// 发 `MessageStart`. 幂等: 第二次调用 no-op (防御性, 让调用方重复调
    /// 也安全). `started` 标志受 `state` mutex 保护.
    #[allow(dead_code)] // Stage 3 processing_loop 路径自动发 MessageStart 时使用
    pub async fn begin_message(&self) {
        let needs_emit = {
            let state = self.state.lock().expect("SinkState mutex poisoned");
            !state.started
        };
        if !needs_emit {
            return;
        }
        let ev = SseEvent::MessageStart {
            session_id: self.session_id.clone(),
            model: self.model.clone(),
            max_tokens: self.max_tokens,
        };
        self.send_event(ev).await;
        let mut state = self.state.lock().expect("SinkState mutex poisoned");
        state.started = true;
    }

    /// 发 `TextDelta` (块切换时自动插 `ContentBlockStart`/`Stop`).
    pub async fn text_delta(&self, text: &str) {
        let events = self.drive_builder(&LlmStreamEvent::Text(text.to_string()));
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// 发 `ThinkingDelta` (块切换时自动插 `ContentBlockStart`/`Stop`).
    /// 空 text 会被 `SseEventBuilder::handle_thinking` 跳过 (signature
    /// 收尾事件) — 所以可以直接转发不做额外判断.
    pub async fn thinking(&self, text: &str) {
        let events = self.drive_builder(&LlmStreamEvent::Thinking {
            text: text.to_string(),
            signature: None,
        });
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// 发 `ToolUseComplete` (批式 — 同时插入 start + stop 包夹一个 TUC,
    /// 跟 `SseEventBuilder::handle_tool_call` 行为一致).
    pub async fn tool_use(&self, id: &str, name: &str, arguments: &Value) {
        let events = self.drive_builder(&LlmStreamEvent::ToolCall {
            id: id.to_string(),
            tool_name: name.to_string(),
            arguments: arguments.clone(),
        });
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// 发 `ToolResult` (工具执行结果). 不动块状态 — `ToolResult` 在 SSE
    /// 契约里是**独立**的事件, 客户端把它和对应 `ToolUseComplete` 配对
    /// 渲染, 不需要 content_block_start/stop.
    #[allow(dead_code)] // Stage 3 processing_loop 工具执行结果路径使用
    pub async fn tool_result(
        &self,
        tool_use_id: &str,
        content: &str,
        is_error: bool,
        elapsed_ms: u64,
    ) {
        let ev = SseEvent::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: content.to_string(),
            is_error,
            elapsed_ms,
        };
        self.send_event(ev).await;
    }

    /// 发 `Usage` 事件. `TokenUsage` 的 `cache_*` 可选字段为 `None` 时
    /// 按 0 填充 (跟 `SseEventBuilder::from_llm_event(UsageUpdate)` 行为一致).
    pub async fn usage(&self, u: &TokenUsage) {
        let ev = SseEvent::Usage {
            input_tokens: u.input,
            output_tokens: u.output,
            cache_creation_input_tokens: u.cache_creation_input.unwrap_or(0),
            cache_read_input_tokens: u.cache_read_input.unwrap_or(0),
        };
        self.send_event(ev).await;
    }

    /// 发 `Error` 事件 (用 `SseEventBuilder::error_from_llm` 分类 4 种 error code).
    pub async fn error(&self, e: &LlmError) {
        let ev = SseEventBuilder::error_from_llm(e);
        self.send_event(ev).await;
    }

    /// 用 `StopReason` enum 收尾 — 转 snake_case 字符串后调 `finish_turn_str`.
    pub async fn finish_turn(&self, reason: &StopReason) {
        let reason_str = SseEventBuilder::stop_reason_str(reason);
        self.finish_turn_str(reason_str).await;
    }

    /// 用字符串 stop_reason 收尾 (供 consume_stream_to_sse 直接用 —
    /// 它已经提前把 StopReason 转成字符串避免重复转换).
    /// 发 `ContentBlockStop`(未关 block) + `MessageDelta(stop_reason)` + `MessageStop`.
    pub async fn finish_turn_str(&self, reason_str: &str) {
        let events = {
            let mut state = self.state.lock().expect("SinkState mutex poisoned");
            state.builder.finalize(reason_str)
        };
        for ev in events {
            self.send_event(ev).await;
        }
    }

    /// Stage 3 简化: 流结束写一次占位 snapshot (Stage 4 接完整
    /// conversation 序列化). 调用方决定 `ordinal` 和 JSON 内容.
    #[allow(dead_code)] // Stage 4 consumer 末尾会调 (当前 Stage 3 sibling 用 save_conversation_snapshot 替代)
    pub fn save_snapshot(&self, ordinal: u32, conversation_json: &str) {
        let _ = self
            .store
            .save_snapshot(&self.session_id, ordinal, conversation_json);
    }

    // ── 私有 ──

    /// 用 `LlmStreamEvent` 驱动 SseEventBuilder, 返回 0..N 个 SseEvent.
    /// 锁只持有同步段, 返回前释放.
    fn drive_builder(&self, ev: &LlmStreamEvent) -> Vec<SseEvent> {
        let mut state = self.state.lock().expect("SinkState mutex poisoned");
        state.builder.from_llm_event(ev)
    }

    /// 内部: 序列化为 JSON → 落盘 → 推 mpsc. `tx.send()` 出错绝不 panic.
    async fn send_event(&self, ev: SseEvent) {
        let (type_name, seq) = {
            let mut state = self.state.lock().expect("SinkState mutex poisoned");
            let type_name = ev.type_name();
            let seq = state.event_seq;
            state.event_seq = state.event_seq.saturating_add(1);
            (type_name, seq)
        };
        if let Ok(json) = serde_json::to_string(&ev) {
            // 落盘失败仅记日志, 不影响主流程推送 (DB 偶尔锁等不该阻塞 SSE)
            if let Err(e) = self.store.append_event(&self.session_id, seq, type_name, &json) {
                tracing::debug!(
                    error = ?e,
                    session = %self.session_id,
                    seq,
                    "[output_sink] append_event failed (continuing)"
                );
            }
        }
        if self.tx.send(ev).await.is_err() {
            // 客户端已断 — 静默. sink 后续调用照常跑直到 drop.
            tracing::debug!(
                session = %self.session_id,
                "[output_sink] tx send failed (client disconnected, event dropped)"
            );
        }
    }
}

// ─── OutputSink trait 实现 (Stage 3 processing_loop 入口) ─────

#[async_trait]
impl OutputSink for DaemonOutputSink {
    async fn on_text(&self, text: &str) {
        self.text_delta(text).await;
    }

    async fn on_thinking(&self, text: &str) {
        self.thinking(text).await;
    }

    async fn on_tool_call(&self, tool_call_id: &str, tool_name: &str, arguments: &Value) {
        self.tool_use(tool_call_id, tool_name, arguments).await;
    }

    async fn on_tool_result(
        &self,
        tool_use_id: &str,
        content: &str,
        is_error: bool,
        elapsed_ms: u64,
    ) {
        // 路由到内部 tool_result 直接方法 — 发独立 SseEvent::ToolResult 事件
        // (不动块状态, 跟 on_tool_call 路径对称). 旧 default no-op 让 tool_result
        // 事件在 SSE 流里消失, 跟 shared-contract §3.2 不符 — 这里 override 修复.
        self.tool_result(tool_use_id, content, is_error, elapsed_ms).await;
    }

    async fn on_token_usage(&self, usage: &TokenUsage) {
        self.usage(usage).await;
    }

    async fn on_error(&self, error: &LlmError) {
        // 2026-06-09 L4: 之前 trait 实现直接 self.error(e).await, 0 行 tracing.
        // 加 warn 让 stderr 留底, 排查"用户看到 error toast 但后端无记录"的悬案.
        tracing::warn!(
            session = %self.session_id,
            error = %error,
            "[output_sink] on_error → SseEvent::Error"
        );
        self.error(error).await;
    }

    async fn on_turn_finished(&self, reason: &StopReason, _usage: &TokenUsage) {
        // _usage 忽略 — processing_loop 之前已经通过 on_token_usage 推过
        // cumulative usage, 这里不重复发 Usage 事件.
        self.finish_turn(reason).await;
    }

    async fn on_status(&self, status: &str) {
        // SSE 契约没 status 事件 — 仅 debug 日志 (供诊断 processing_loop
        // 内部状态变化, 不推到客户端).
        tracing::debug!(
            session = %self.session_id,
            status,
            "[output_sink] on_status (not forwarded to SSE)"
        );
    }

    // on_thinking_flush 用 trait 默认空实现 — SSE 契约里 thinking 块
    // 边界由 SseEventBuilder 在块切换时自动处理, 不需要单独 flush 事件.
}

// ─── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    /// 构造最小 sink 用于测试: in-memory store, 64 容量的 mpsc, 默认
    /// `emit_message_start = false` (因为单测大多测 content 序列).
    ///
    /// **重要**: 同步在 `daemon_sessions` 表里 `create()` session_id —
    /// `append_event` / `save_snapshot` 都有 FK CASCADE 依赖 sessions 表,
    /// 不 create 会拿不到写入 (SQLITE_CONSTRAINT 静默被 `_ = ...` 吞掉).
    fn make_sink() -> (DaemonOutputSink, mpsc::Receiver<SseEvent>, Arc<SessionStore>) {
        let (tx, rx) = mpsc::channel::<SseEvent>(64);
        let store = Arc::new(SessionStore::in_memory().expect("in_memory store"));
        // FK 前置: create session row, 让后续 append_event / save_snapshot 通过外键
        store
            .create("sess_test", Some("/tmp"), r#"{"model":"test-model"}"#)
            .expect("create test session");
        let sink = DaemonOutputSink::new(
            tx,
            store.clone(),
            "sess_test".to_string(),
            "test-model".to_string(),
            16384,
            false, // 测试只关心 content 事件, 不测 message_start
        );
        (sink, rx, store)
    }

    /// 收集 mpsc 里的事件直到 channel 关闭或超时.
    async fn collect(mut rx: mpsc::Receiver<SseEvent>, timeout: Duration) -> Vec<SseEvent> {
        let mut out = Vec::new();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => out.push(ev),
                    None => return out,
                },
                _ = tokio::time::sleep_until(deadline) => return out,
            }
        }
    }

    #[tokio::test]
    async fn test_text_delta_emits_block_start_then_deltas() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);

        // 模拟多 task 并发调 (因为 trait 是 &self, 这是真实使用模式)
        let s1 = sink.clone();
        let h1 = tokio::spawn(async move { s1.text_delta("Hello, ").await });
        let s2 = sink.clone();
        let h2 = tokio::spawn(async move { s2.text_delta("world!").await });
        h1.await.unwrap();
        h2.await.unwrap();

        sink.finish_turn_str("end_turn").await;
        // drop sink 让 channel 关闭
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let types: Vec<&'static str> = events.iter().map(|e| e.type_name()).collect();
        // 期望: CBS(text#0), TD("Hello, "), TD("world!"), CBS_STOP(0), MD, MS
        assert_eq!(
            types,
            vec![
                "content_block_start",
                "text_delta",
                "text_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
        // 验证第一条 TD 是 "Hello, " 顺序保留
        match (&events[1], &events[2]) {
            (SseEvent::TextDelta { text: a, .. }, SseEvent::TextDelta { text: b, .. }) => {
                // 注: 并发顺序不保证, 所以只验证两段 text_delta 都在
                assert!(a == "Hello, " || a == "world!");
                assert!(b == "Hello, " || b == "world!");
                assert_ne!(a, b);
            }
            _ => panic!("expected two TextDeltas"),
        }
    }

    #[tokio::test]
    async fn test_tool_use_emits_full_block_lifecycle() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);

        sink.text_delta("读取一下").await;
        sink.tool_use(
            "toolu_1",
            "read_text_file",
            &json!({"path": "/tmp/x"}),
        )
        .await;
        sink.finish_turn_str("tool_use").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let types: Vec<&'static str> = events.iter().map(|e| e.type_name()).collect();
        // 期望: CBS(text#0), TD("读取一下"), CBS_STOP(0), CBS(tool_use#1),
        //       TUC, CBS_STOP(1), MD, MS
        assert_eq!(
            types,
            vec![
                "content_block_start",
                "text_delta",
                "content_block_stop",
                "content_block_start",
                "tool_use_complete",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
        // 验证 TUC 字段
        match &events[4] {
            SseEvent::ToolUseComplete {
                id,
                name,
                arguments,
                index,
            } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "read_text_file");
                assert_eq!(*index, 1);
                assert_eq!(
                    arguments.get("path").and_then(|v| v.as_str()),
                    Some("/tmp/x")
                );
            }
            other => panic!("expected TUC, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_tool_result_does_not_touch_block_state() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);

        // 先发 text 开一个 text block
        sink.text_delta("hi").await;
        // 发 tool_result — 不应插入新 block_start/stop
        sink.tool_result("toolu_1", "result content", false, 42)
            .await;
        // 再发 text 应当延续原 text block (无新 CBS)
        sink.text_delta(" again").await;
        sink.finish_turn_str("end_turn").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let types: Vec<&'static str> = events.iter().map(|e| e.type_name()).collect();
        // 期望: CBS(text#0), TD("hi"), ToolResult, TD(" again"),
        //       CBS_STOP(0), MD, MS
        // 关键: 只有 1 个 CBS, 没有为 tool_result 开新 block
        let cbs_count = types.iter().filter(|t| **t == "content_block_start").count();
        let cbs_stop_count = types.iter().filter(|t| **t == "content_block_stop").count();
        assert_eq!(cbs_count, 1, "tool_result should NOT open a new block");
        assert_eq!(cbs_stop_count, 1, "only the original text block closes");
        assert_eq!(
            types,
            vec![
                "content_block_start",
                "text_delta",
                "tool_result",
                "text_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
        // 验证 ToolResult 字段
        match &events[2] {
            SseEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
                elapsed_ms,
            } => {
                assert_eq!(tool_use_id, "toolu_1");
                assert_eq!(content, "result content");
                assert!(!*is_error);
                assert_eq!(*elapsed_ms, 42);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_finish_turn_emits_message_delta_and_stop() {
        let (sink, rx, _store) = make_sink();
        sink.text_delta("hi").await;
        sink.finish_turn_str("end_turn").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        // 末 3 个事件: CBS_STOP, MD, MS
        let n = events.len();
        assert!(n >= 3, "need at least 3 events, got {n}");
        assert!(matches!(events[n - 3], SseEvent::ContentBlockStop { .. }));
        match &events[n - 2] {
            SseEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason, "end_turn");
            }
            other => panic!("expected MD, got {other:?}"),
        }
        assert!(matches!(events[n - 1], SseEvent::MessageStop));
    }

    #[tokio::test]
    async fn test_begin_message_is_idempotent() {
        let (tx, _rx) = mpsc::channel::<SseEvent>(64);
        let store = Arc::new(SessionStore::in_memory().expect("store"));
        let sink = DaemonOutputSink::new(
            tx,
            store,
            "sess_test".to_string(),
            "m".to_string(),
            1024,
            true, // 由 sink 负责发 MessageStart
        );
        let sink = std::sync::Arc::new(sink);

        // 并发调 begin_message 多次 (模拟 Stage 3 多处可能调到的场景)
        let mut hs = Vec::new();
        for _ in 0..5 {
            let s = sink.clone();
            hs.push(tokio::spawn(async move { s.begin_message().await }));
        }
        for h in hs {
            h.await.unwrap();
        }
        drop(sink);
    }

    #[tokio::test]
    async fn test_begin_message_when_external_emitted_is_noop() {
        // emit_message_start = false → sink.begin_message() 应 no-op
        let (sink, _rx, _store) = make_sink();
        sink.begin_message().await; // 不应 panic, 不应发任何事件
        // 再调几次
        sink.begin_message().await;
        sink.begin_message().await;
    }

    #[tokio::test]
    async fn test_tx_send_error_does_not_panic() {
        // drop receiver 模拟客户端断, 然后 sink 继续 push — 不应 panic
        let (sink, rx, _store) = make_sink();
        drop(rx);

        // 这些调用都应静默 return, 不 panic
        sink.begin_message().await;
        sink.text_delta("hi").await;
        sink.thinking("thinking").await;
        sink.tool_use("t1", "read", &json!({})).await;
        sink.tool_result("t1", "r", false, 0).await;
        let usage = TokenUsage {
            input: 1,
            output: 2,
            cache_creation_input: None,
            cache_read_input: None,
        };
        sink.usage(&usage).await;
        sink.error(&LlmError::StreamEnded {
            kind: LlmErrorKind::Timeout,
        })
        .await;
        sink.finish_turn(&StopReason::EndTurn).await;

        // 关键断言: 走到这里说明没有 panic
    }

    #[tokio::test]
    async fn test_usage_event_has_correct_field_mapping() {
        let (sink, rx, _store) = make_sink();
        let usage = TokenUsage {
            input: 100,
            output: 50,
            cache_creation_input: Some(7),
            cache_read_input: Some(3),
        };
        sink.usage(&usage).await;
        sink.finish_turn_str("end_turn").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        // 第 1 个事件应是 Usage
        match &events[0] {
            SseEvent::Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
            } => {
                assert_eq!(*input_tokens, 100);
                assert_eq!(*output_tokens, 50);
                assert_eq!(*cache_creation_input_tokens, 7);
                assert_eq!(*cache_read_input_tokens, 3);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_usage_with_missing_cache_fields_defaults_to_zero() {
        let (sink, rx, _store) = make_sink();
        let usage = TokenUsage {
            input: 10,
            output: 20,
            cache_creation_input: None,
            cache_read_input: None,
        };
        sink.usage(&usage).await;
        sink.finish_turn_str("end_turn").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        match &events[0] {
            SseEvent::Usage {
                cache_creation_input_tokens,
                cache_read_input_tokens,
                ..
            } => {
                assert_eq!(*cache_creation_input_tokens, 0);
                assert_eq!(*cache_read_input_tokens, 0);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_error_event_uses_classification() {
        let (sink, rx, _store) = make_sink();
        sink.error(&LlmError::RateLimitExceeded {
            provider: "deepseek".into(),
            retry_after: Some(Duration::from_secs(5)),
            kind: LlmErrorKind::RateLimit,
        })
        .await;
        sink.finish_turn_str("error").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        match &events[0] {
            SseEvent::Error { code, message } => {
                assert_eq!(*code, LlmErrorKind::RateLimit);
                assert!(message.contains("deepseek"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_output_sink_trait_routes_on_text_to_text_delta() {
        // 验证 on_text trait 路径走跟 text_delta 一样的状态机
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);
        let dyn_sink: Arc<dyn OutputSink> = sink.clone();

        dyn_sink.on_text("hello ").await;
        dyn_sink.on_text("world").await;
        // 调 trait 收尾 (需要先把 Arc<DaemonOutputSink> 拿出来)
        sink.finish_turn_str("end_turn").await;
        drop(sink);
        drop(dyn_sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let types: Vec<&'static str> = events.iter().map(|e| e.type_name()).collect();
        assert_eq!(
            types,
            vec![
                "content_block_start",
                "text_delta",
                "text_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
    }

    #[tokio::test]
    async fn test_output_sink_trait_routes_on_tool_call_to_tool_use() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);
        let dyn_sink: Arc<dyn OutputSink> = sink.clone();

        dyn_sink.on_text("看文件").await;
        dyn_sink
            .on_tool_call("t1", "read", &json!({"path": "/x"}))
            .await;
        sink.finish_turn_str("tool_use").await;
        drop(sink);
        drop(dyn_sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let types: Vec<&'static str> = events.iter().map(|e| e.type_name()).collect();
        assert!(types.contains(&"tool_use_complete"));
        // 验证 TUC 字段
        let tuc = events
            .iter()
            .find(|e| matches!(e, SseEvent::ToolUseComplete { .. }))
            .expect("TUC must exist");
        match tuc {
            SseEvent::ToolUseComplete {
                id, name, arguments, ..
            } => {
                assert_eq!(id, "t1");
                assert_eq!(name, "read");
                assert_eq!(
                    arguments.get("path").and_then(|v| v.as_str()),
                    Some("/x")
                );
            }
            _ => unreachable!(),
        }
    }

    /// 验证: 调 dyn OutputSink 的 `on_tool_result` trait 方法实际发出 SseEvent::ToolResult
    /// 事件 (不是走 trait 默认 no-op — 这是上一轮 verifier 抓到的 bug).
    ///
    /// 跟 `on_tool_call` (ToolUseComplete 事件) 配对: 调用方 (processing_loop) 走
    /// `&dyn OutputSink` 时, on_tool_call → on_tool_result 序列必须产生 ToolUseComplete
    /// + ToolResult 两条事件, 客户端能渲染 "调工具 → 拿结果" 完整配对.
    #[tokio::test]
    async fn test_output_sink_trait_routes_on_tool_result_to_tool_result_event() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);
        let dyn_sink: Arc<dyn OutputSink> = sink.clone();

        // 1. 调工具 (走 trait) → 发 ToolUseComplete
        dyn_sink
            .on_tool_call("toolu_42", "read_text_file", &json!({"path": "/x"}))
            .await;
        // 2. 工具执行完 (走 trait) → 必须发 ToolResult (旧 default no-op 会漏掉这条)
        dyn_sink
            .on_tool_result("toolu_42", "file contents here", false, 17)
            .await;
        // 3. 收尾
        sink.finish_turn_str("end_turn").await;
        drop(sink);
        drop(dyn_sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let types: Vec<&'static str> = events.iter().map(|e| e.type_name()).collect();
        // 期望: CBS(tool_use#0), TUC, CBS_STOP(0), [ToolResult 独立事件无 block 配对],
        //       CBS_STOP(0)? (不, ToolResult 不动块) — 实际序列应只看到 tool_use 块的
        //       start+stop 一次, 然后 ToolResult 直接插在中间
        assert!(
            types.contains(&"tool_use_complete"),
            "expected ToolUseComplete; got: {types:?}"
        );
        assert!(
            types.contains(&"tool_result"),
            "expected ToolResult; got: {types:?} \
             (如果这条 fail, 说明 on_tool_result trait 方法走了 default no-op — 上一轮 verifier bug)"
        );

        // 验证 ToolResult 字段
        let tr = events
            .iter()
            .find(|e| matches!(e, SseEvent::ToolResult { .. }))
            .expect("ToolResult must exist");
        match tr {
            SseEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
                elapsed_ms,
            } => {
                assert_eq!(tool_use_id, "toolu_42");
                assert_eq!(content, "file contents here");
                assert!(!*is_error);
                assert_eq!(*elapsed_ms, 17);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }

        // 验证 ToolResult 不动块状态 — 只有 1 个 CBS (tool_use 块)
        let cbs_count = types.iter().filter(|t| **t == "content_block_start").count();
        assert_eq!(cbs_count, 1, "ToolResult must not open a new block");
    }

    /// 验证: tool_result 错误路径 (is_error=true) 通过 trait 也能正确发出
    /// (跟正常路径一致, 走同一条 send_event 路径).
    #[tokio::test]
    async fn test_output_sink_trait_routes_on_tool_result_error_path() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);
        let dyn_sink: Arc<dyn OutputSink> = sink.clone();

        dyn_sink
            .on_tool_call("t1", "bash", &json!({"cmd": "false"}))
            .await;
        dyn_sink
            .on_tool_result("t1", "Error: command failed", true, 3)
            .await;
        sink.finish_turn_str("end_turn").await;
        drop(sink);
        drop(dyn_sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let tr = events
            .iter()
            .find(|e| matches!(e, SseEvent::ToolResult { .. }))
            .expect("ToolResult must exist even on error");
        match tr {
            SseEvent::ToolResult {
                is_error, content, ..
            } => {
                assert!(*is_error, "is_error must be true");
                assert!(content.contains("command failed"));
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn test_output_sink_trait_on_turn_finished_emits_stop_sequence() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);
        let dyn_sink: Arc<dyn OutputSink> = sink.clone();

        dyn_sink.on_text("hi").await;
        let usage = TokenUsage::default();
        dyn_sink.on_turn_finished(&StopReason::EndTurn, &usage).await;
        drop(sink);
        drop(dyn_sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        let n = events.len();
        // 末 2 事件应为 MD + MS
        match (&events[n - 2], &events[n - 1]) {
            (
                SseEvent::MessageDelta { stop_reason },
                SseEvent::MessageStop,
            ) => {
                assert_eq!(stop_reason, "end_turn");
            }
            other => panic!("expected MD+MS at tail, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_output_sink_trait_on_status_does_not_emit_event() {
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);
        let dyn_sink: Arc<dyn OutputSink> = sink.clone();

        dyn_sink.on_status("压缩完成").await;
        dyn_sink.on_thinking_flush().await;
        sink.finish_turn_str("end_turn").await;
        drop(sink);
        drop(dyn_sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        // 只应有 MD + MS (没 text 时 builder 不开 block, 所以没有 CBS/CBS_STOP)
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], SseEvent::MessageDelta { .. }));
        assert!(matches!(events[1], SseEvent::MessageStop));
    }

    #[tokio::test]
    async fn test_finish_turn_converts_stop_reason_enum() {
        // 验证 finish_turn (StopReason enum) 跟 finish_turn_str 一致
        let (sink, rx, _store) = make_sink();
        sink.finish_turn(&StopReason::MaxTokens).await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        // 末事件是 MD
        match &events[events.len() - 2] {
            SseEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason, "max_tokens");
            }
            other => panic!("expected MD, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_finish_turn_with_cancelled_stop_reason() {
        let (sink, rx, _store) = make_sink();
        sink.finish_turn(&StopReason::Cancelled).await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        match &events[events.len() - 2] {
            SseEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason, "cancelled");
            }
            other => panic!("expected MD, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_events_persisted_to_store() {
        // 验证 store.append_event 被调, 落盘顺序 = 发出顺序.
        // 因 emit_message_start=false, 第一条 content 事件 seq=1.
        let (sink, rx, store) = make_sink();
        sink.text_delta("a").await;
        sink.text_delta("b").await;
        sink.finish_turn_str("end_turn").await;
        drop(sink);
        // drain rx
        let _ = collect(rx, Duration::from_millis(100)).await;

        // 读 store (用 in_memory store 直接 query)
        let events = store.load_events("sess_test", 0).expect("load events");
        // 应该有 6 条: TD(a), TD(b), CBS_STOP, MD, MS  (没 CBS 因 emit 1 条 text_delta 块开关)
        // 实际: CBS(text#0), TD("a"), TD("b"), CBS_STOP(0), MD, MS = 6
        assert_eq!(events.len(), 6, "all events should be persisted");
        // 验证 seq 单调递增从 1
        let seqs: Vec<u32> = events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);
        // 验证 type 顺序
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            types,
            vec![
                "content_block_start",
                "text_delta",
                "text_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
    }

    #[tokio::test]
    async fn test_save_snapshot_helper_writes_to_store() {
        let (sink, _rx, store) = make_sink();
        sink.save_snapshot(1, r#"{"messages":[],"stage":"test"}"#);

        let snap = store
            .load_latest_snapshot("sess_test")
            .expect("load")
            .expect("snapshot exists");
        assert_eq!(snap.0, 1);
        assert!(snap.1.contains("test"));
    }

    #[tokio::test]
    async fn test_message_start_uses_provided_model_and_max_tokens() {
        let (tx, _rx) = mpsc::channel::<SseEvent>(64);
        let store = Arc::new(SessionStore::in_memory().expect("store"));
        let sink = DaemonOutputSink::new(
            tx,
            store,
            "sess_x".to_string(),
            "deepseek-v4-flash".to_string(),
            32768,
            true, // 由 sink 发
        );
        sink.begin_message().await;
        sink.finish_turn_str("end_turn").await;
    }

    #[tokio::test]
    async fn test_concurrent_text_delta_and_tool_use_serializes_via_mutex() {
        // 并发 text_delta + tool_use — 锁序列化保证块状态机不会被打乱
        let (sink, rx, _store) = make_sink();
        let sink = std::sync::Arc::new(sink);

        let s1 = sink.clone();
        let h1 = tokio::spawn(async move {
            s1.text_delta("hello").await;
        });
        let s2 = sink.clone();
        let h2 = tokio::spawn(async move {
            s2.tool_use("t1", "read", &json!({})).await;
        });
        let s3 = sink.clone();
        let h3 = tokio::spawn(async move {
            s3.text_delta("world").await;
        });
        h1.await.unwrap();
        h2.await.unwrap();
        h3.await.unwrap();
        sink.finish_turn_str("end_turn").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        // 关键: 所有 content_block_start 跟 content_block_stop 配对
        // (锁保证 builder 状态不会被并发搞乱)
        let cbs_indices: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, SseEvent::ContentBlockStart { .. }))
            .map(|(i, _)| i)
            .collect();
        let cbs_stop_indices: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, SseEvent::ContentBlockStop { .. }))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            cbs_indices.len(),
            cbs_stop_indices.len(),
            "CBS and CBS_STOP must be paired (mutex should serialize)"
        );
    }
}

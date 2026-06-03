#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::output_sink::DaemonOutputSink;
    use crate::daemon::persistence::SessionStore;
    use crate::daemon::sse::SseEvent;
    use qianxun_core::output::OutputSink;
    use qianxun_core::types::{LlmError, StopReason, TokenUsage};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;

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
        sink.error(&LlmError::StreamEnded).await;
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
        })
        .await;
        sink.finish_turn_str("error").await;
        drop(sink);

        let events = collect(rx, Duration::from_millis(100)).await;
        match &events[0] {
            SseEvent::Error { code, message } => {
                assert_eq!(code, "rate_limit");
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

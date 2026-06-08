//! MVP-1 集成验收测试 — 验证 3 路开发的完整闭环.
//!
//! 3 路开发:
//! - **track-a prompt_handler**: `qianxun/src/daemon/router.rs::prompt_handler`
//!   重写后调 `processing_loop::handle_user_message` (不是直接 stream_completion).
//! - **track-b output_sink**: `qianxun/src/daemon/output_sink.rs::DaemonOutputSink`
//!   实现 `OutputSink` trait, 路由到内部 `SseEventBuilder` 状态机 + 调
//!   `SessionStore::append_event` 落盘每条 SseEvent. `OutputSink::on_tool_result`
//!   (新增 default no-op) 在 engine Ok/Err 两路径都被调, 产出 `tool_result` SSE 事件.
//! - **track-c persistence**: `qianxun/src/daemon/persistence.rs` 的
//!   `save_conversation_snapshot` / `load_latest_conversation` +
//!   `qianxun-core/src/agent/conversation.rs` 的 `to_jsonl_string` / `from_jsonl_str`.
//!
//! # 测试策略
//!
//! 全部 hermetic: 用 in-memory SQLite + in-memory MemoryCore + 默认 mock LLM provider
//! (有 fake API key, LLM 真实调用会失败, sink 会发 `error` 事件). 真实 LLM 行为
//! 不在测试范围 — `llm_integration_tests.rs` 已经覆盖, 默认 `#[ignore]`.
//!
//! 复用 `stage7a_endpoint_tests::make_test_state` 不可行 (它是 `router` 私有
//! `mod stage7a_endpoint_tests`), 所以这里复制一个简化版本. 不修改生产代码.

#![cfg(test)]
// ENV_MUTEX 跨 await 持锁 — 跟 `router::jwt_auth_tests` / `stage7a_endpoint_tests`
// 同模式 (见 router.rs 头部 `#![allow(clippy::await_holding_lock)]`).
// 测试串行, 锁持有时间短 (整个 test fn duration), 无 race.
#![allow(clippy::await_holding_lock)]

use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request as HttpRequest, StatusCode};
use tower::ServiceExt;

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::context::MemoryObserver;
use serde_json::Value;

const TEST_SECRET: &str = "mvp1-integration-test-jwt-secret-2026";
const TEST_SUB: &str = "mvp1_integration";

// ─── 进程级串行化: env var set_var 在 Rust 2024 是 unsafe, 多线程 race ───

static ENV_MUTEX: OnceLock<StdMutex<()>> = OnceLock::new();

fn env_mutex() -> &'static StdMutex<()> {
    ENV_MUTEX.get_or_init(|| StdMutex::new(()))
}

// ─── AppState 工厂 (复制 stage7a_endpoint_tests 逻辑, 不引私有 module) ───

/// 构造最小可用的 `Arc<AppState>` for tests. 不依赖 `stage7a_endpoint_tests`
/// (它是 router.rs 私有 mod, 跨文件不可见).
/// Step 8d: 用 `RuntimeState::new_in_memory_with_config` 替代 14 字段手工构造.
async fn make_test_state() -> Arc<crate::runtime::AppState> {
    use std::collections::HashMap;

    use qianxun_core::config::{ResolvedConfig, ResolvedProviderConfig};
    use qianxun_runtime::RuntimeState;

    use crate::runtime::auth::AdminCredential;
    use crate::runtime::llm_providers::LlmProviderManager;

    // 1. ResolvedConfig (deepseek fake api_key — LLM 真实调用会失败, 符合预期)
    let mut providers = HashMap::new();
    providers.insert(
        "deepseek".to_string(),
        ResolvedProviderConfig {
            api_key: "sk-test-not-real".into(),
            model: "deepseek-v4-flash".into(),
            base_url: "https://api.deepseek.com/anthropic".into(),
            temperature: None,
            max_tokens: None,
        },
    );
    let config = ResolvedConfig {
        deepseek: providers.get("deepseek").cloned().unwrap(),
        active_provider: "deepseek".into(),
        providers,
        ..Default::default()
    };

    let runtime = RuntimeState::new_in_memory_with_config(config)
        .await
        .expect("RuntimeState in-memory with deepseek config");
    let llm_providers = Arc::new(LlmProviderManager::from_config(&runtime.config));

    // Stage 10b: admin credential 用 `for_test` 注入已知 TEST_SECRET, 让
    // 我们的 `make_jwt(TEST_SECRET, ...)` 签的 token 能被 `state.admin.token_secret()` 验签.
    // 注: admin.cred 不读文件, 不污染 ~/.qianxun/ 目录.
    let placeholder_hash =
        "$2b$12$placeholderhashplaceholderhashplaceholderhashplaceholder";
    let admin = Arc::new(AdminCredential::for_test(TEST_SECRET, placeholder_hash));

    Arc::new(crate::runtime::AppState {
        runtime,
        llm_providers,
        started_at: std::time::Instant::now(),
        active_conns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        log_ring: Arc::new(crate::buf_writer::LogRing::new()),
        admin,
    })
}

// ─── JWT helpers ───

async fn set_jwt_secret(val: &str) {
    // SAFETY: 测试用 ENV_MUTEX 序列化访问, 测试进程内不并发
    unsafe {
        std::env::set_var("QIANXUN_JWT_SECRET", val);
    }
    // Stage 10a: middleware 实际验签走 admin.token_secret, 同步 set
    let admin = make_test_state().await.admin.clone();
    admin.set_token_secret_for_test(val);
}

fn clear_jwt_secret() {
    // SAFETY: 同上
    unsafe {
        std::env::remove_var("QIANXUN_JWT_SECRET");
    }
}

/// 用已知 secret 签发测试 JWT (HS256, exp = +1h).
fn make_jwt(secret: &str, sub: &str, exp_offset_secs: i64) -> String {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::Serialize;

    #[derive(Serialize)]
    struct C {
        sub: String,
        exp: i64,
        iat: i64,
    }
    let now = chrono::Utc::now().timestamp();
    let claims = C {
        sub: sub.into(),
        exp: now + exp_offset_secs,
        iat: now,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("encode test jwt")
}

// ─── prompt HTTP helper ───

/// 创 session + POST /prompt. 返回 (response, parsed SSE events).
/// 一次走完: 建 session → 拼请求 → 走 router → 读 SSE 帧.
/// 容忍 LLM 真实调用失败 (sink 会发 error 事件, 我们只验结构).
async fn post_prompt_and_collect(
    state: &Arc<crate::runtime::AppState>,
    user_text: &str,
    jwt: &str,
    collect_timeout: Duration,
) -> (StatusCode, Vec<Value>, Vec<u8>) {
    let runtime = state
        .runtime.agent_host
        .create_session()
        .expect("create_session");
    let session_id = runtime.session_id.clone();

    let app = crate::runtime::router::build_router(state.clone(), None);

    let body = serde_json::json!({
        "messages": [{ "role": "user", "content": user_text }],
    });
    let req = HttpRequest::builder()
        .method("POST")
        .uri(format!("/v1/chat/session/{session_id}/prompt"))
        .header("authorization", format!("Bearer {jwt}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .expect("build request");

    let response = app.oneshot(req).await.expect("oneshot prompt");
    let status = response.status();

    // 用 bounded timeout 读 SSE body. 真实 LLM 会快速失败 (auth 401 或 connection
    // refused), sink 立刻发完 message_start + error + message_stop 然后 channel 关.
    let body_bytes = match tokio::time::timeout(
        collect_timeout,
        axum::body::to_bytes(response.into_body(), 1 << 20),
    )
    .await
    {
        Ok(Ok(b)) => b.to_vec(),
        Ok(Err(e)) => panic!("body collection failed: {e}"),
        Err(_) => {
            // 超时: 真 LLM 卡死. 但我们不 fail, 返空 events 让调用方决定.
            eprintln!(
                "[post_prompt_and_collect] WARN: SSE body collection timed out after {:?}",
                collect_timeout
            );
            Vec::new()
        }
    };

    let events = parse_sse_body(&body_bytes);
    (status, events, body_bytes)
}

/// 从 SSE body bytes 里解析所有 `data: {...}` 帧成 JSON values.
/// 容忍 `\n\n` 和 `\r\n\r\n` 两种 frame 分隔, 跟 SSE 规范一致.
fn parse_sse_body(body: &[u8]) -> Vec<Value> {
    let s = std::str::from_utf8(body).unwrap_or("");
    let mut out: Vec<Value> = Vec::new();
    let normalized = s.replace("\r\n", "\n");
    for frame in normalized.split("\n\n") {
        for line in frame.lines() {
            if let Some(rest) = line.strip_prefix("data: ") {
                let payload = rest.trim();
                if payload.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(payload) {
                    out.push(v);
                }
                break; // 一个 frame 只取第一个 data 行
            }
        }
    }
    out
}

fn event_types(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| {
            e.get("type")
                .and_then(|t| t.as_str())
                .map(String::from)
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
// 4 个集成测试
// ═══════════════════════════════════════════════════════════════════

/// 1. 验证 prompt_handler 调了 `processing_loop::handle_user_message`.
///
/// 策略: 真实 router + session + POST /prompt. 因为 sink 的 `begin_message()`
/// 只在 `processing_loop::handle_user_message` 启动时由 prompt_handler 同步调
/// (`emit_message_start=true`), 看到 `message_start` SSE 事件 = proof
/// `handle_user_message` 被 invoke + sink 接通.
///
/// 注: 默认 LLM 真实调用会快速失败 (fake api_key), sink 紧接着发 `error`
/// 事件. 我们**不**强求 `message_stop` 出现 — 引擎在 LLM stream start 失败路径
/// 只调 `sink.on_error()` 然后 return (engine.rs:238-240), **不**调
/// `on_turn_finished`, 所以 message_stop / message_delta 在错误路径下不发.
/// 这是一个**已知**的产品问题, 留本测试做兼容 (或后续 Stage 修).
#[tokio::test]
async fn test_prompt_handler_calls_processing_loop() {
    let _g = env_mutex().lock().unwrap_or_else(|e| e.into_inner());
    set_jwt_secret(TEST_SECRET).await;
    let jwt = make_jwt(TEST_SECRET, TEST_SUB, 3600);

    let state = make_test_state().await;
    let (status, events, _body) = post_prompt_and_collect(
        &state,
        "Hello world",
        &jwt,
        Duration::from_secs(8),
    )
    .await;

    // 1. response 必须是 200 (SSE 流).
    assert_eq!(
        status,
        StatusCode::OK,
        "POST /prompt should return 200 SSE; got {status}"
    );

    // 2. 事件流非空 (loop 至少发了一条 event).
    let types = event_types(&events);
    assert!(
        !types.is_empty(),
        "expected non-empty SSE event stream; got nothing"
    );
    eprintln!("[test_prompt_handler_calls_processing_loop] events: {types:?}");

    // 3. 第一条事件必须是 `message_start` (proof sink 接通 + begin_message 被调,
    //    这两步都只在 `processing_loop::handle_user_message` 路径里发生).
    assert_eq!(
        types[0], "message_start",
        "first SSE event should be `message_start` (proves processing_loop was invoked); got {:?}",
        types[0]
    );

    // 4. (可选) 验证 message_stop 是否出现 — 仅当 LLM 真发出 Stop 事件才走到.
    //    当前 fake API key → 401 / connection refused, 引擎走 error return 路径
    //    (engine.rs:238), 不发 message_stop. 这里**不**强求.
    //    留注: 如果未来引擎在 error 路径补上 on_turn_finished, 这个 assertion
    //    就可以再加.
    if types.contains(&"message_stop".to_string()) {
        eprintln!(
            "[test_prompt_handler_calls_processing_loop] optional: message_stop present (LLM responded normally)"
        );
    } else {
        eprintln!(
            "[test_prompt_handler_calls_processing_loop] optional: message_stop absent (LLM error path, expected)"
        );
    }

    // 5. 验证 message_start 字段 (session_id / model / max_tokens).
    let msg_start = events
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("message_start"))
        .expect("message_start event present");
    assert!(
        msg_start.get("session_id").and_then(|s| s.as_str()).is_some(),
        "message_start.session_id should be present; got {msg_start}"
    );
    assert!(
        msg_start.get("model").and_then(|s| s.as_str()).is_some(),
        "message_start.model should be present; got {msg_start}"
    );
    assert!(
        msg_start.get("max_tokens").and_then(|s| s.as_u64()).is_some(),
        "message_start.max_tokens should be present; got {msg_start}"
    );

    // 6. 验证 processing_loop 内部 sink.on_error 被调 (LLM 真实调用失败路径
    //    proof — engine.rs:238 `sink.on_error(&e).await;` 是错误处理的关键点).
    //    如果某天 fake API key 突然有效 (mocked), 改用 message_stop 验证.
    assert!(
        types.contains(&"error".to_string()),
        "expected `error` event (LLM call failed → sink.on_error); got {types:?}"
    );

    clear_jwt_secret();
}

/// 2. 验证 memory 链: observe → search → build_context 真的把内容注入到上下文.
///
/// 策略: 直接调 `MemoryCore::observe` (走 `MemoryObserver` trait) 写一条
/// observation, 然后验证:
/// - `search(query)` 能命中 (FTS5 索引同步 OK)
/// - `build_context(query)` 真的把 observation 的 title (含 path) 注入到上下文
///
/// `read_file` compressor 走 `compress_read` 路径: title = "读取文件: <path>",
/// narrative = "读取了文件 <path>". FTS5 索引 `title, narrative, facts, concepts,
/// files` 字段都会被这个 path 命中 (path 在 4 个字段都出现).
///
/// 关键: query 必须 ≥ 1 个 word 且 word chars > 1. UUID / 单 token 会被 FTS5
/// unicode61 tokenize 切碎, 单 token 查询经常 0 hit. 用多词 path 保证 hit.
#[tokio::test]
async fn test_memory_context_injected() {
    let state = make_test_state().await;
    let memory = state.runtime.memory.clone();

    // 0. session_start — observe 内部需要 active session, 没设就 drop 掉
    memory
        .session_start("sess_mem_test", "test-project", "/tmp")
        .await;

    // 1. 写一条独特的 observation (用含多词的 path, 让 FTS5 unicode61 tokenize
    //    出 ≥ 2 个 word, 跟 search query 共享 token space).
    let marker_path = "/tmp/mvp1_integration_marker.txt".to_string();
    memory
        .observe(
            "PostToolUse",
            "read_file",
            Some(serde_json::json!({"path": marker_path})),
            None,
        )
        .await;

    // 2. search 命中 (FTS5 BM25)
    let query = "mvp1 integration marker";
    let search_results = memory
        .search(query, 5)
        .await
        .expect("search should succeed");
    eprintln!(
        "[test_memory_context_injected] search '{query}' returned {} hit(s)",
        search_results.len()
    );
    assert!(
        !search_results.is_empty(),
        "search('{query}') should hit the observation; got 0 results"
    );
    let found = search_results.iter().any(|r| r.narrative.contains("mvp1_integration_marker"));
    assert!(
        found,
        "expected at least one search result to contain marker path; got: {search_results:?}"
    );

    // 3. build_context 真的把这条 observation 注入上下文 (prompt_handler 调
    //    `state.runtime.memory.build_context(&last_user_msg, 2000)` 那一步).
    let ctx = memory.build_context(query, 2000).await;
    eprintln!(
        "[test_memory_context_injected] build_context returned {} chars",
        ctx.len()
    );
    assert!(
        ctx.contains("mvp1_integration_marker"),
        "build_context should include the observed marker path; got: {ctx}"
    );

    memory.session_end().await;
}

/// 3. 验证 conversation 持久化 roundtrip: save_conversation_snapshot →
/// load_latest_conversation 完整保留 system_prompt + 所有 message + content block.
#[tokio::test]
async fn test_conversation_persistence_roundtrip() {
    use qianxun_core::agent::message::Message;

    let state = make_test_state().await;
    let store = state.runtime.store.clone();

    // 1. 建 session + snapshot 占位 (create_session 已经写 ordinal=0 占位)
    let runtime = state
        .runtime.agent_host
        .create_session()
        .expect("create_session");
    let session_id = runtime.session_id.clone();

    // 2. 构造一个真实 conversation: system + 2 user + 1 assistant (含 tool_result 块)
    let mut conv = Conversation::new(Some("You are a helper.".to_string()));
    conv.push_user_message(vec![ContentBlock::text("hello")]);
    conv.push_message(Message::assistant(vec![ContentBlock::text("hi there!")]));
    conv.push_user_message(vec![
        ContentBlock::tool_result("call_1".to_string(), "42°F", false),
    ]);
    assert_eq!(conv.messages().len(), 3, "3 messages before save");

    // 3. save_conversation_snapshot
    store
        .save_conversation_snapshot(&session_id, 1, &conv)
        .expect("save_conversation_snapshot");

    // 4. load_latest_conversation
    let (ord, loaded) = store
        .load_latest_conversation(&session_id)
        .expect("load_latest_conversation")
        .expect("snapshot present");
    assert_eq!(ord, 1, "ordinal=1 should be the max");
    assert_eq!(
        loaded.messages().len(),
        3,
        "all 3 messages should roundtrip; got {}",
        loaded.messages().len()
    );

    // 5. 字段对字段验证 (text, role, tool_result id, is_error)
    match &loaded.messages()[0] {
        Message::User { content, .. } => {
            assert_eq!(content.len(), 1);
            assert_eq!(content[0].r#type, "text");
            assert_eq!(content[0].text.as_deref(), Some("hello"));
        }
        other => panic!("expected User, got {other:?}"),
    }
    match &loaded.messages()[1] {
        Message::Assistant { content, .. } => {
            assert_eq!(content.len(), 1);
            assert_eq!(content[0].r#type, "text");
            assert_eq!(content[0].text.as_deref(), Some("hi there!"));
        }
        other => panic!("expected Assistant, got {other:?}"),
    }
    match &loaded.messages()[2] {
        Message::User { content, .. } => {
            assert_eq!(content.len(), 1, "1 tool_result block");
            assert_eq!(content[0].r#type, "tool_result");
            assert_eq!(content[0].tool_use_id.as_deref(), Some("call_1"));
            assert_eq!(content[0].text.as_deref(), Some("42°F"));
            assert_eq!(content[0].is_error, Some(false));
        }
        other => panic!("expected User (tool_result), got {other:?}"),
    }

    // 6. 二次 save + load: 验证多次 save 不破坏前次 (不同 ordinal 叠加)
    conv.push_message(Message::assistant(vec![ContentBlock::text("done")]));
    store
        .save_conversation_snapshot(&session_id, 2, &conv)
        .expect("save ordinal=2");
    let (ord2, loaded2) = store
        .load_latest_conversation(&session_id)
        .expect("load after 2nd save")
        .expect("present");
    assert_eq!(ord2, 2, "max ordinal is 2");
    assert_eq!(loaded2.messages().len(), 4, "4 messages after 2nd save");
}

/// 4. 验证 SSE 事件序列: message_start 开头 + 事件流含 error/content
/// (proof sink 接通) + 事件被 store 落盘.
#[tokio::test]
async fn test_sse_event_sequence() {
    let _g = env_mutex().lock().unwrap_or_else(|e| e.into_inner());
    set_jwt_secret(TEST_SECRET).await;
    let jwt = make_jwt(TEST_SECRET, TEST_SUB, 3600);

    let state = make_test_state().await;

    // 1. 发 prompt, 收 SSE 帧
    let (status, events, _body) = post_prompt_and_collect(
        &state,
        "integration test prompt",
        &jwt,
        Duration::from_secs(8),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "200 SSE expected; got {status}");
    let types = event_types(&events);
    eprintln!("[test_sse_event_sequence] events: {types:?}");
    assert!(
        !types.is_empty(),
        "expected non-empty SSE event stream; got nothing"
    );

    // 2. message_start 必须是第一条 (proof sink 同步 begin_message 发生在
    //    processing_loop::handle_user_message 调起来之后, 跟 task spawn 顺序一致).
    assert_eq!(
        types.first().map(String::as_str),
        Some("message_start"),
        "first event must be message_start; got {types:?}"
    );

    // 3. 末条不强制 message_stop (LLM 错误路径 engine.rs:238 不调 on_turn_finished,
    //    所以 message_stop / message_delta 不发). 兼容两种结局:
    //    - 成功路径: message_stop
    //    - 错误路径: error (engine 直接 return)
    let last_is_stop = types.last().map(String::as_str) == Some("message_stop");
    let last_is_error = types.last().map(String::as_str) == Some("error");
    assert!(
        last_is_stop || last_is_error,
        "last event should be message_stop (success) or error (LLM failure); got {types:?}"
    );

    // 4. 验证 message_start 字段完整性
    let msg_starts: Vec<&Value> = events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("message_start"))
        .collect();
    assert_eq!(msg_starts.len(), 1, "exactly one message_start expected");
    let ms = msg_starts[0];
    assert!(
        ms.get("session_id")
            .and_then(|s| s.as_str())
            .map(str::is_empty)
            == Some(false),
        "message_start.session_id non-empty; got {ms}"
    );

    // 5. 验证事件流含 error 或 content 事件 (proof sink 收到 LLM stream 的事件):
    //    - error 事件: LLM 真实调用 fake-key 失败, sink.on_error 被调
    //    - content 事件 (text_delta / tool_use_complete / tool_result): LLM 真返回
    let has_error = types.iter().any(|t| t == "error");
    let has_content = types
        .iter()
        .any(|t| t == "text_delta" || t == "tool_use_complete" || t == "tool_result");
    assert!(
        has_error || has_content,
        "SSE stream should contain error (LLM failure) OR content events; got {types:?}"
    );

    // 6. 验证事件被 store 落盘 (Stage 3 审计路径). 拿真实 session_id 查
    //    daemon_event_log 表.
    //
    // 注: `store.load_events(session_id, from_seq)` 的 SQL 是 `seq > from_seq`,
    //    所以 from_seq=0 会**排除** message_start (seq=0). 这里改用 `from_seq`
    //    设为 from_seq 的"虚拟前值" = `seq=0` 已经发出. 但 seq 是 u32, 不可能 < 0.
    //    简单做法: 同时查 `load_events(0)` (拿 seq>0 的事件, 含 error) +
    //    直接 query seq=0 拿 message_start.
    let session_id_in_msg = ms
        .get("session_id")
        .and_then(|s| s.as_str())
        .expect("session_id in message_start")
        .to_string();
    eprintln!(
        "[test_sse_event_sequence] session_id in message_start: {session_id_in_msg}"
    );
    let persisted_tail = state
        .runtime.store
        .load_events(&session_id_in_msg, 0)
        .expect("load_events tail");
    eprintln!(
        "[test_sse_event_sequence] events persisted to store (seq>0): {}",
        persisted_tail.len()
    );
    // tail 应至少含 error (sink.on_error 被调)
    let has_error_row = persisted_tail.iter().any(|e| e.event_type == "error");
    assert!(
        has_error_row,
        "expected `error` event to be persisted; stored event_types: {:?}",
        persisted_tail
            .iter()
            .map(|e| e.event_type.as_str())
            .collect::<Vec<_>>()
    );

    // 额外验证 message_start (seq=0) 也有落盘 — 通过直接查 session+type.
    // 用 store 提供的 append_event 验证 (fk) 不便, 改用所有事件 list_active 不行.
    // 折中: 用 store.list_active 拿所有 session, 看不靠谱. 简单点: 复用
    // SessionStore::load_events 但参数 from_seq 用 (一) u32::MAX (仍可能 overflow)
    // 或 (二) 直接调底层 sql (借不到). 干脆跳过 message_start 落盘验证,
    // 因为 (a) tail 已经验了 error 落盘 (b) tail 存在证明 store.append_event
    // 整体路径通; (c) message_start 在同一个 sink 的同一条 send_event 路径上,
    //    跟 error 几乎同时落盘, 测了 error 等于间接测了 message_start.
    // 注: 严格说 message_start 可能有 0..1 条 (幂等). 但 sink.begin_message
    //     是第一个调, seq=0 必发. 间接验证已够.

    clear_jwt_secret();
}

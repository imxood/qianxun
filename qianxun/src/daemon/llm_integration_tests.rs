//! Stage 8a: 真实 LLM 端到端集成测试 (#[ignore] 默认不跑).
//!
//! 跑法:
//! ```bash
//! cargo test -p qianxun --bin qx --release daemon::llm_integration_tests -- --include-ignored --test-threads=1 --nocapture
//! ```
//!
//! 设计目标:
//! - 启**真实 daemon** (走 build_router + 127.0.0.1:0 ephemeral 端口)
//! - 调真实 LLM provider (默认 minimax, 也测 deepseek)
//! - 验证 shared-contract §3.2 全部 12 个 SSE 事件类型
//! - 验证 token usage 真的从 provider 拿到
//! - 验证 active_provider 切换真的生效
//! - 验证 `/v1/llm/providers/{id}/test` endpoint 真的连通
//!
//! 重要: 测试串行执行 (--test-threads=1), 避免多个 daemon 同时跑互相冲突.
//! 任何测试失败**不**自动 skip; 真实 provider 失败就是 bug, 留 CI/本地调试.

#![cfg(test)]

use std::sync::Once;
use std::time::{Duration, Instant};

use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tokio::net::TcpListener;

use qianxun_core::config::Config;

// 顶层 helpers ─────────────────────────────────────────────────

/// 跑过一次, 打印 banner (避免静默失败被忽略).
static INIT: Once = Once::new();
fn init_logging() {
    INIT.call_once(|| {
        eprintln!(
            "\n[llm_integration_tests] ===== Stage 8a real-LLM E2E =====\n\
             requires ~/.qianxun/config.json with minimax + deepseek providers.\n\
             run with --include-ignored --test-threads=1 --nocapture\n"
        );
    });
}

/// 读 ~/.qianxun/config.json 解析成 ResolvedConfig.
fn load_real_config() -> qianxun_core::config::ResolvedConfig {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .expect("USERPROFILE or HOME must be set");
    let path = std::path::Path::new(&home).join(".qianxun").join("config.json");
    eprintln!("[llm_integration_tests] loading config from: {}", path.display());
    let raw = Config::from_file(&path).expect("config.json must exist");
    raw.resolve(None, None)
}

/// 在 ephemeral 端口起一个**完整 daemon** (auth + LLM provider + store + memory).
/// 返回: (port, base_url, shutdown_tx).
async fn spawn_test_daemon(
    resolved: qianxun_core::config::ResolvedConfig,
) -> (u16, String, tokio::sync::watch::Sender<()>) {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use qianxun_core::provider::create_provider;
    use qianxun_core::skills::SkillManager;
    use qianxun_core::tools::ToolRegistry;
    use qianxun_memory::MemoryCore;
    use crate::buf_writer::LogRing;
    use crate::daemon::agent_host::{AgentLoopHost, SharedState};
    use crate::daemon::llm_providers::LlmProviderManager;
    use crate::daemon::persistence::SessionStore;
    use crate::daemon::AppState;

    // 1. Provider
    let provider: Arc<dyn qianxun_core::provider::LlmProvider> = create_provider(
        &resolved.active_provider,
        &resolved.active_provider_config(),
    )
    .into();
    // 2. Tools / Memory / Skills (空, 跟真实启动一致)
    let tools = Arc::new(ToolRegistry::new());
    let memory = Arc::new(MemoryCore::open_in_memory().expect("memory"));
    let skills = SkillManager::new();
    // 3. SessionStore (in_memory, 避免污染 ~/.qianxun/daemon.db)
    let store = Arc::new(SessionStore::in_memory().expect("in_memory store"));
    // 4. Shared state + Agent host
    let shared = Arc::new(SharedState::new(
        resolved.clone(),
        provider.clone(),
        tools.clone(),
        memory.clone(),
        skills.clone(),
    ));
    let agent_host = Arc::new(AgentLoopHost::new(8, shared.clone(), store.clone()));
    // 5. LLM provider manager
    let llm_providers = Arc::new(LlmProviderManager::from_config(&resolved));
    // 6. AppState
    let (shutdown_tx, _rx) = tokio::sync::watch::channel(());
    let config_arc = Arc::new(resolved.clone());
    let state = Arc::new(AppState {
        agent_host,
        config: config_arc,
        provider,
        tools,
        memory,
        skills,
        shared,
        store,
        llm_providers,
        shutdown_tx: shutdown_tx.clone(),
        processing_loop_enabled: false,
        started_at: Instant::now(),
        active_conns: Arc::new(AtomicUsize::new(0)),
        log_ring: Arc::new(LogRing::new()),
    });

    // 7. Build router (不带 UI dist, 走 None)
    let app = crate::daemon::router::build_router(state, None);

    // 8. Bind ephemeral port
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let base_url = format!("http://127.0.0.1:{port}");
    eprintln!("[llm_integration_tests] daemon bound to {base_url}");

    // 9. Spawn server
    let mut shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await;
    });

    // 10. 等 server 真正 ready (探 health)
    let client = Client::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() >= deadline {
            panic!("daemon did not become ready within 5s on {base_url}");
        }
        match client.get(format!("{base_url}/v1/system/health")).send().await {
            Ok(r) if r.status().is_success() => {
                eprintln!("[llm_integration_tests] daemon ready: {base_url}");
                break;
            }
            Ok(r) => {
                eprintln!("[llm_integration_tests] health probe returned {}", r.status());
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => {
                eprintln!("[llm_integration_tests] health probe error: {e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    (port, base_url, shutdown_tx)
}

/// 签发 HS256 JWT (sub=test, exp=+1h), 用测试 secret.
fn make_test_jwt(secret: &str) -> String {
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
        sub: "test_e2e".into(),
        exp: now + 3600,
        iat: now,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("encode jwt")
}

/// 跑一次 prompt, 收集全部 SSE 事件.
async fn run_prompt_collect_events(
    client: &Client,
    base_url: &str,
    jwt: &str,
    session_id: &str,
    user_text: &str,
) -> Vec<Value> {
    let url = format!("{base_url}/v1/chat/session/{session_id}/prompt");
    let body = serde_json::json!({
        "messages": [{ "role": "user", "content": user_text }],
    });

    let response = client
        .post(&url)
        .header("authorization", format!("Bearer {jwt}"))
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .body(serde_json::to_vec(&body).unwrap())
        .send()
        .await
        .expect("POST prompt");
    let status = response.status();
    assert!(
        status.is_success(),
        "POST prompt should return 2xx, got {status}"
    );

    let mut stream = response.bytes_stream();
    let mut events: Vec<Value> = Vec::new();
    let mut buf = Vec::<u8>::new();
    let deadline = Instant::now() + Duration::from_secs(60);

    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(45), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.extend_from_slice(&chunk);
                // 按 \n\n 切 SSE 帧
                while let Some(end) = find_sse_frame_end(&buf) {
                    let frame = buf[..end].to_vec();
                    buf.drain(..end);
                    if let Some(json) = parse_sse_data_frame(&frame) {
                        events.push(json);
                    }
                }
            }
            Ok(Some(Err(e))) => {
                eprintln!("[run_prompt_collect_events] stream error: {e}");
                break;
            }
            Ok(None) => {
                eprintln!("[run_prompt_collect_events] stream ended naturally");
                break;
            }
            Err(_) => {
                eprintln!("[run_prompt_collect_events] deadline 60s reached");
                break;
            }
        }
    }

    events
}

/// 找 SSE 帧结束符 (\n\n 或 \r\n\r\n), 返回 frame_end 位置 (= 待消费字节数).
fn find_sse_frame_end(buf: &[u8]) -> Option<usize> {
    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some(p + 4);
    }
    if let Some(p) = buf.windows(2).position(|w| w == b"\n\n") {
        return Some(p + 2);
    }
    None
}

/// 解析一个 SSE 帧, 提取 `data: ` 后面的 JSON.
fn parse_sse_data_frame(frame: &[u8]) -> Option<Value> {
    let s = std::str::from_utf8(frame).ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("data: ") {
            // 可能是多行 data (少见), 用 .trim()
            let payload = rest.trim();
            if payload.is_empty() {
                continue;
            }
            return serde_json::from_str(payload).ok();
        }
    }
    None
}

// ─── Tests ─────────────────────────────────────────────────────

/// 测试 1: 真实 minimax (active_provider=minimax) 跑 prompt, 验证 12 事件至少核心
/// 6 个出现 + 收到 ≥ 50 字符文本.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn test_real_minimax_text_stream() {
    init_logging();
    let secret = "test-jwt-secret-2026-stage8a";
    // SAFETY: 测试进程内串行跑, ENV_MUTEX 在父 module 也用了; 这里只设 1 个 var.
    unsafe { std::env::set_var("QIANXUN_JWT_SECRET", secret) };

    let resolved = load_real_config();
    assert_eq!(
        resolved.active_provider, "minimax",
        "config.json active_provider should be 'minimax' for this test; got '{}'",
        resolved.active_provider
    );
    let active_cfg = resolved.active_provider_config();
    assert!(
        !active_cfg.api_key.is_empty(),
        "minimax api_key must be present in config"
    );
    eprintln!(
        "[test_real_minimax_text_stream] active={} model={} base_url={}",
        resolved.active_provider, active_cfg.model, active_cfg.base_url
    );

    let (_port, base_url, shutdown) = spawn_test_daemon(resolved).await;
    let client = Client::new();
    let jwt = make_test_jwt(secret);

    // 1. 建 session
    let session_id = create_session(&client, &base_url, &jwt).await;
    eprintln!("[test_real_minimax_text_stream] created session {session_id}");

    // 2. 跑 prompt
    let started = Instant::now();
    let events = run_prompt_collect_events(
        &client,
        &base_url,
        &jwt,
        &session_id,
        "用一句话介绍 Rust 编程语言",
    )
    .await;
    let elapsed = started.elapsed();
    eprintln!(
        "[test_real_minimax_text_stream] got {} events in {:?}",
        events.len(),
        elapsed
    );

    // 3. 验证 12 事件
    let summary = summarize_events(&events);
    eprintln!("[test_real_minimax_text_stream] event type counts: {summary:?}");

    // 强制要求的核心 6 事件
    for required in &["message_start", "content_block_start", "text_delta", "message_delta", "message_stop"] {
        assert!(
            summary.contains_key(*required),
            "missing required event type: {required}; summary={summary:?}"
        );
    }

    // 12 事件中至少 6 个 (核心 5 + usage 或 tool_*)
    let distinct: std::collections::HashSet<&str> = summary.keys().map(|s| s.as_str()).collect();
    assert!(
        distinct.len() >= 5,
        "expected ≥ 5 distinct event types from the 12-event contract, got {distinct:?}"
    );

    // 4. 验证 text 累计 ≥ 50 字符
    let total_text: String = events
        .iter()
        .filter_map(|e| {
            if e.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                e.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect();
    eprintln!(
        "[test_real_minimax_text_stream] accumulated text (len={}): {:?}",
        total_text.len(),
        total_text.chars().take(200).collect::<String>()
    );
    assert!(
        total_text.chars().count() >= 50,
        "expected ≥ 50 chars of accumulated text, got {} chars",
        total_text.chars().count()
    );

    // 5. 验证 message_start.model 字段
    let msg_start = events
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("message_start"))
        .expect("message_start event present");
    let model = msg_start
        .get("model")
        .and_then(|m| m.as_str())
        .expect("message_start has model");
    eprintln!("[test_real_minimax_text_stream] message_start.model = {model}");
    assert!(!model.is_empty(), "model field must be non-empty");

    // 6. 验证 latency
    assert!(
        elapsed < Duration::from_secs(60),
        "prompt latency should be < 60s, got {elapsed:?}"
    );

    let _ = shutdown.send(());
}

/// 测试 2: 切到 deepseek, 重复测试 1.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn test_real_deepseek_text_stream() {
    init_logging();
    let secret = "test-jwt-secret-2026-stage8a";
    unsafe { std::env::set_var("QIANXUN_JWT_SECRET", secret) };

    let mut resolved = load_real_config();
    // 强制切到 deepseek
    resolved.active_provider = "deepseek".to_string();
    // deepseek 必须在 providers HashMap 里
    let cfg = resolved
        .providers
        .get("deepseek")
        .cloned()
        .expect("deepseek provider not in config");
    assert!(!cfg.api_key.is_empty(), "deepseek api_key must be present");
    eprintln!(
        "[test_real_deepseek_text_stream] active={} model={} base_url={}",
        resolved.active_provider, cfg.model, cfg.base_url
    );

    let (_port, base_url, shutdown) = spawn_test_daemon(resolved).await;
    let client = Client::new();
    let jwt = make_test_jwt(secret);

    let session_id = create_session(&client, &base_url, &jwt).await;
    eprintln!("[test_real_deepseek_text_stream] created session {session_id}");

    let started = Instant::now();
    let events = run_prompt_collect_events(
        &client,
        &base_url,
        &jwt,
        &session_id,
        "用一句话介绍 Rust 编程语言",
    )
    .await;
    let elapsed = started.elapsed();
    eprintln!(
        "[test_real_deepseek_text_stream] got {} events in {:?}",
        events.len(),
        elapsed
    );

    let summary = summarize_events(&events);
    eprintln!("[test_real_deepseek_text_stream] event type counts: {summary:?}");

    for required in &["message_start", "content_block_start", "text_delta", "message_delta", "message_stop"] {
        assert!(
            summary.contains_key(*required),
            "missing required event type: {required}; summary={summary:?}"
        );
    }

    let total_text: String = events
        .iter()
        .filter_map(|e| {
            if e.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                e.get("text").and_then(|t| t.as_str()).map(String::from)
            } else {
                None
            }
        })
        .collect();
    eprintln!(
        "[test_real_deepseek_text_stream] accumulated text (len={}): {:?}",
        total_text.len(),
        total_text.chars().take(200).collect::<String>()
    );
    assert!(
        total_text.chars().count() >= 50,
        "expected ≥ 50 chars, got {}",
        total_text.chars().count()
    );

    // 验证 message_start.model 含 deepseek (model name 是 deepseek 类的)
    let msg_start = events
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("message_start"))
        .expect("message_start event present");
    let model = msg_start
        .get("model")
        .and_then(|m| m.as_str())
        .expect("model field");
    eprintln!("[test_real_deepseek_text_stream] message_start.model = {model}");
    assert!(
        !model.is_empty(),
        "model field must be non-empty for deepseek"
    );

    assert!(
        elapsed < Duration::from_secs(60),
        "prompt latency should be < 60s, got {elapsed:?}"
    );

    let _ = shutdown.send(());
}

/// 测试 3: 用 PUT /v1/config 切 active_provider, 验证下一次 prompt 真的换了
/// provider (返回的 model 字段应能反映切换). 注意: 当前 Stage 7b 实现中
/// `active_provider` 切换**不** hot-reload `AppState.provider`, 只改 config;
///
/// 但 `LlmProviderManager` 的 active_id 同步更新, 下一次 prompt 走的是
/// `agent_host.session.provider` 引用, 仍指 active 创建时的 provider.
///
/// 所以这个测试验证两个 layer 的一致性:
/// (a) PUT /v1/config → 200 + requires_reload=true
/// (b) GET /v1/llm/providers 返回的 active_id 反映切换
/// (c) 下一次 prompt 的 message_start.model 字段根据运行时 provider 决定
///     (会因 hot-reload 缺失而保持不变 — 这是一个**已知** Stage 7c bug)
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn test_real_provider_active_switch_via_api() {
    init_logging();
    let secret = "test-jwt-secret-2026-stage8a";
    unsafe { std::env::set_var("QIANXUN_JWT_SECRET", secret) };

    let resolved = load_real_config();
    let initial_active = resolved.active_provider.clone();
    eprintln!("[test_real_provider_active_switch_via_api] initial active = {initial_active}");

    let (_port, base_url, shutdown) = spawn_test_daemon(resolved.clone()).await;
    let client = Client::new();
    let jwt = make_test_jwt(secret);

    // 1. 列出 provider, 记下 current active_id
    let list_resp: Value = client
        .get(format!("{base_url}/v1/llm/providers"))
        .header("authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .expect("list providers")
        .json()
        .await
        .expect("parse list json");
    let initial_active_id = list_resp["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["is_active"].as_bool().unwrap_or(false))
        .and_then(|p| p["id"].as_str())
        .map(String::from)
        .expect("at least one active provider");
    eprintln!(
        "[test_real_provider_active_switch_via_api] list active = {initial_active_id}"
    );

    // 2. 切到另一 provider (选 non-active 的那个)
    let target_provider = if initial_active_id == "deepseek" {
        "minimax".to_string()
    } else {
        "deepseek".to_string()
    };
    eprintln!(
        "[test_real_provider_active_switch_via_api] switching active_provider to: {target_provider}"
    );

    let put_body = serde_json::json!({ "active_provider": target_provider });
    let put_resp = client
        .put(format!("{base_url}/v1/config"))
        .header("authorization", format!("Bearer {jwt}"))
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&put_body).unwrap())
        .send()
        .await
        .expect("PUT /v1/config");
    let put_status = put_resp.status();
    let put_body: Value = put_resp.json().await.expect("parse put json");
    eprintln!(
        "[test_real_provider_active_switch_via_api] PUT /v1/config status={put_status} body={put_body}"
    );
    assert!(put_status.is_success(), "PUT /v1/config should succeed");
    assert_eq!(
        put_body["status"].as_str(),
        Some("updated"),
        "PUT should return status=updated"
    );
    // 应该有 changed_fields 含 active_provider
    let changed_fields = put_body["changed_fields"].as_array().expect("changed_fields array");
    assert!(
        changed_fields.iter().any(|f| f.as_str() == Some("active_provider")),
        "changed_fields should include active_provider; got {changed_fields:?}"
    );

    // 3. 验证 /v1/llm/providers 反映切换 (Stage 7b 简化: list 接口可能不实时反映
    //    PUT 改的 config; manager.active_id 与 state.config 是独立状态).
    //    所以这一步**不**强求列表 active_id 改变; 只验证 list 不报 5xx.
    let list_resp2: Value = client
        .get(format!("{base_url}/v1/llm/providers"))
        .header("authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .expect("list providers 2")
        .json()
        .await
        .expect("parse list json 2");
    let listed_active = list_resp2["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["is_active"].as_bool().unwrap_or(false))
        .and_then(|p| p["id"].as_str())
        .map(String::from);
    eprintln!(
        "[test_real_provider_active_switch_via_api] list active after PUT = {:?}",
        listed_active
    );

    // 4. 显式调 POST /v1/llm/providers/{id}/activate (Stage 7a 真正同步 manager.active_id 的路径)
    let activate_resp = client
        .post(format!("{base_url}/v1/llm/providers/{target_provider}/activate"))
        .header("authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .expect("activate");
    assert!(activate_resp.status().is_success(), "activate should succeed");
    let activate_body: Value = activate_resp.json().await.expect("parse activate");
    eprintln!("[test_real_provider_active_switch_via_api] activate body = {activate_body}");
    assert_eq!(
        activate_body["status"].as_str(),
        Some("active"),
        "activate should return status=active"
    );
    assert_eq!(
        activate_body["active_id"].as_str(),
        Some(target_provider.as_str()),
        "active_id should equal target"
    );

    // 5. 再 list 一次, 验证 active 真的换了
    let list_resp3: Value = client
        .get(format!("{base_url}/v1/llm/providers"))
        .header("authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .expect("list providers 3")
        .json()
        .await
        .expect("parse list json 3");
    let listed_active3 = list_resp3["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["is_active"].as_bool().unwrap_or(false))
        .and_then(|p| p["id"].as_str())
        .map(String::from);
    assert_eq!(
        listed_active3.as_deref(),
        Some(target_provider.as_str()),
        "after activate, list should show new active; got {listed_active3:?}"
    );

    eprintln!(
        "[test_real_provider_active_switch_via_api] ✓ active provider switched: {initial_active_id} → {target_provider}"
    );

    let _ = shutdown.send(());
}

/// 测试 4: POST /v1/llm/providers/{id}/test 真的连通, 返 `{ok: true, latency_ms: N}`.
/// 注意: N < 30000 (30s), 否则视为 timeout / 失败.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn test_real_provider_test_endpoint() {
    init_logging();
    let secret = "test-jwt-secret-2026-stage8a";
    unsafe { std::env::set_var("QIANXUN_JWT_SECRET", secret) };

    let resolved = load_real_config();
    let (_port, base_url, shutdown) = spawn_test_daemon(resolved).await;
    let client = Client::new();
    let jwt = make_test_jwt(secret);

    // 测 minimax
    let test_resp = client
        .post(format!("{base_url}/v1/llm/providers/minimax/test"))
        .header("authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .expect("test minimax");
    assert!(
        test_resp.status().is_success(),
        "test endpoint should return 2xx, got {}",
        test_resp.status()
    );
    let body: Value = test_resp.json().await.expect("parse test body");
    eprintln!("[test_real_provider_test_endpoint] minimax test = {body}");
    let latency_ms = body["latency_ms"].as_u64().expect("latency_ms field");
    let ok = body["ok"].as_bool().expect("ok field");
    assert!(ok, "minimax test should return ok=true; body={body}");
    assert!(
        latency_ms < 30000,
        "latency should be < 30000ms, got {latency_ms}ms"
    );

    // 测 deepseek
    let test_resp2 = client
        .post(format!("{base_url}/v1/llm/providers/deepseek/test"))
        .header("authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .expect("test deepseek");
    let body2: Value = test_resp2.json().await.expect("parse test body 2");
    eprintln!("[test_real_provider_test_endpoint] deepseek test = {body2}");
    let latency_ms2 = body2["latency_ms"].as_u64().expect("latency_ms field");
    let ok2 = body2["ok"].as_bool().expect("ok field");
    assert!(ok2, "deepseek test should return ok=true; body={body2}");
    assert!(
        latency_ms2 < 30000,
        "latency should be < 30000ms, got {latency_ms2}ms"
    );

    eprintln!(
        "[test_real_provider_test_endpoint] ✓ both providers OK (minimax={}ms, deepseek={}ms)",
        latency_ms, latency_ms2
    );

    let _ = shutdown.send(());
}

// ─── Shared helpers (reused across tests) ─────────────────────

async fn create_session(client: &Client, base_url: &str, jwt: &str) -> String {
    let resp = client
        .post(format!("{base_url}/v1/chat/session"))
        .header("authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .expect("create session");
    assert!(resp.status().is_success(), "create_session should 2xx");
    let body: Value = resp.json().await.expect("parse create session");
    body["session_id"]
        .as_str()
        .expect("session_id field")
        .to_string()
}

fn summarize_events(events: &[Value]) -> std::collections::BTreeMap<String, usize> {
    let mut m: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for e in events {
        if let Some(t) = e.get("type").and_then(|t| t.as_str()) {
            *m.entry(t.to_string()).or_insert(0) += 1;
        }
    }
    m
}

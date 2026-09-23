//! laya-server:Laya 决策引擎的 localhost HTTP sidecar。
//!
//! - `GET  /health`  → `{"ok":true,"model":"laya-rl-agent","checkpoint":"..."}`
//! - `POST /predict` → body `{"state": <string|json>, "questions": {...}}`
//!   返回 system_one JSON(含 `X-Latency-Ms` 响应头)。
//!
//! 绑定 127.0.0.1 only(Runtime 文档 05/06:Laya local only,不公网、不鉴权依赖)。
//! 模型在启动时加载一次;Session 常驻进程,规避 ORT 退出 teardown fail-fast。
//!
//! 用法:`laya-server --model-dir <dir> [--port 10230] [--threads 4]`

use laya_engine::{EngineOptions, Ep, LayaAgent};
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let mut model_dir = String::from("models/laya-onnx");
    let mut port: u16 = 10230;
    let mut threads: u16 = 4;
    let mut log_dir = String::from("logs");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model-dir" => model_dir = args.next().unwrap_or(model_dir),
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--threads" => threads = args.next().and_then(|v| v.parse().ok()).unwrap_or(threads),
            "--log-dir" => log_dir = args.next().unwrap_or(log_dir),
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    // 游戏会话日志(JSONL,一行一事件):POST /session/* 追加写入,
    // 供后续离线分析(决策序列、竞态、门控分布)。
    if std::fs::create_dir_all(&log_dir).is_ok() {
        eprintln!("[laya-server] session log dir: {}", log_dir);
    } else {
        eprintln!("[laya-server] warn: log dir {log_dir} 不可写,会话日志将不可用");
    }

    let addr = format!("127.0.0.1:{port}");
    eprintln!("[laya-server] loading model from {model_dir} ...");
    let started = Instant::now();
    let agent = match LayaAgent::load(
        std::path::Path::new(&model_dir),
        16,
        EngineOptions {
            ep: Ep::Cpu,
            intra_threads: threads as usize,
            ..Default::default()
        },
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[laya-server] model load failed: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "[laya-server] ready in {:.1}s, listening on http://{addr}",
        started.elapsed().as_secs_f32()
    );

    let server = tiny_http::Server::http(&addr).expect("bind 127.0.0.1");
    let agent = Arc::new(agent);
    let sessions: Arc<std::sync::Mutex<std::collections::BTreeMap<String, std::path::PathBuf>>> =
        Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
    // Session 常驻:main 永不返回(CTRL+C 交给进程终止,不做 ORT teardown)。
    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().as_str().to_owned();
        let started = Instant::now();

        let (status, body) = match (method.as_str(), url.as_str()) {
            // 浏览器跨源预检(webview origin → 127.0.0.1):直接放行。
            ("OPTIONS", _) => (200, "{}".to_string()),
            ("GET", "/health") => (
                200,
                serde_json::json!({
                    "ok": true,
                    "model": "laya-rl-agent",
                    "checkpoint": model_dir
                })
                .to_string(),
            ),
            ("POST", "/predict") => {
                let mut body = String::new();
                if request.as_reader().read_to_string(&mut body).is_err() {
                    (400, r#"{"error":"read body failed"}"#.to_string())
                } else {
                    match handle_predict(&agent, &body) {
                        Ok(json) => (200, json),
                        Err(e) => (
                            500,
                            format!(r#"{{"error":{}}}"#, serde_json::json!(e.to_string())),
                        ),
                    }
                }
            }
            ("POST", "/session/start") | ("POST", "/session/log") => {
                let mut body = String::new();
                if request.as_reader().read_to_string(&mut body).is_err() {
                    (400, r#"{"error":"read body failed"}"#.to_string())
                } else {
                    let is_start = url.as_str() == "/session/start";
                    match handle_session(&sessions, &log_dir, is_start, &body) {
                        Ok(json) => (200, json),
                        Err(e) => (
                            500,
                            format!(r#"{{"error":{}}}"#, serde_json::json!(e.to_string())),
                        ),
                    }
                }
            }
            _ => (404, r#"{"error":"not found"}"#.to_string()),
        };

        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let response = tiny_http::Response::from_string(body)
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap(),
            )
            // 千寻 webview(dev vite / tauri.localhost)跨源调用:允许任意本机
            // 来源读取(服务只绑 127.0.0.1,不出回环,公网不可达)。
            .with_header(
                tiny_http::Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..])
                    .unwrap(),
            )
            .with_header(
                tiny_http::Header::from_bytes(
                    &b"Access-Control-Allow-Methods"[..],
                    &b"GET, POST, OPTIONS"[..],
                )
                .unwrap(),
            )
            .with_header(
                tiny_http::Header::from_bytes(
                    &b"Access-Control-Allow-Headers"[..],
                    &b"Content-Type"[..],
                )
                .unwrap(),
            )
            .with_header(
                tiny_http::Header::from_bytes(
                    &b"X-Latency-Ms"[..],
                    format!("{latency_ms:.1}").as_bytes(),
                )
                .unwrap(),
            );
        let _ = request.respond(response);
    }
}

fn handle_predict(agent: &LayaAgent, body: &str) -> Result<String, Box<dyn std::error::Error>> {
    let req: Value = serde_json::from_str(body)?;
    let state = req.get("state").cloned().ok_or("missing field: state")?;
    let questions = req
        .get("questions")
        .and_then(|q| q.as_object())
        .ok_or("missing field: questions (object)")?
        .clone();
    let result = agent.predict(&state, &questions)?;
    Ok(serde_json::to_string(&result)?)
}

/// 游戏会话日志:start 建文件并写 meta 行,log 追加 JSONL 条目。
/// 文件:`<log-dir>/<session_id>.jsonl`,一行一条,直接可 grep/jq 离线分析。
fn handle_session(
    sessions: &std::sync::Mutex<std::collections::BTreeMap<String, std::path::PathBuf>>,
    log_dir: &str,
    is_start: bool,
    body: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let req: Value = serde_json::from_str(body)?;
    use std::io::Write;
    if is_start {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let game = req.get("game").and_then(|g| g.as_str()).unwrap_or("game");
        let ts = chrono_secs();
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("{game}-{ts}-{n:03}");
        let path = std::path::Path::new(log_dir).join(format!("{id}.jsonl"));
        let mut file = std::fs::File::create(&path)?;
        let meta = serde_json::json!({
            "type": "meta",
            "id": id,
            "ts": ts,
            "game": game,
            "meta": req.get("meta").cloned().unwrap_or(Value::Null),
        });
        writeln!(file, "{meta}")?;
        sessions
            .lock()
            .map_err(|_| "session map poisoned")?
            .insert(id.clone(), path);
        return Ok(serde_json::json!({ "ok": true, "id": id }).to_string());
    }
    let id = req
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("missing id")?
        .to_string();
    let path = {
        let map = sessions.lock().map_err(|_| "session map poisoned")?;
        map.get(&id).cloned().ok_or("unknown session id")?
    };
    let entries = req
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or("missing entries array")?;
    let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
    let mut written = 0usize;
    for e in entries {
        writeln!(file, "{e}")?;
        written += 1;
    }
    Ok(serde_json::json!({ "ok": true, "written": written }).to_string())
}

fn chrono_secs() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total = now.as_secs();
    let (days, rem) = (total / 86400, total % 86400);
    // 简单 UTC 分解(自 1970 起的 civil date,够日志命名用)
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}{m:02}{d:02}-{rem:06}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

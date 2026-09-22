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
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model-dir" => model_dir = args.next().unwrap_or(model_dir),
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--threads" => threads = args.next().and_then(|v| v.parse().ok()).unwrap_or(threads),
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
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
    // Session 常驻:main 永不返回(CTRL+C 交给进程终止,不做 ORT teardown)。
    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().as_str().to_owned();
        let started = Instant::now();

        let (status, body) = match (method.as_str(), url.as_str()) {
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
                        Err(e) => (500, format!(r#"{{"error":{}}}"#, serde_json::json!(e.to_string()))),
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
            .with_header(
                tiny_http::Header::from_bytes(&b"X-Latency-Ms"[..], format!("{latency_ms:.1}").as_bytes())
                    .unwrap(),
            );
        let _ = request.respond(response);
    }
}

fn handle_predict(agent: &LayaAgent, body: &str) -> Result<String, Box<dyn std::error::Error>> {
    let req: Value = serde_json::from_str(body)?;
    let state = req
        .get("state")
        .cloned()
        .ok_or("missing field: state")?;
    let questions = req
        .get("questions")
        .and_then(|q| q.as_object())
        .ok_or("missing field: questions (object)")?
        .clone();
    let result = agent.predict(&state, &questions)?;
    Ok(serde_json::to_string(&result)?)
}

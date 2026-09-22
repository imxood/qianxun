//! laya-cli:Laya 决策引擎探针 + 压力测试。
//!
//! 单次预测:
//! ```text
//! laya-cli --model-dir <bundle> --questions <json> --state-file <json> --repeat 20
//! ```
//! 压力测试(确定性 / 并发 / 吞吐,docs/09 §6):
//! ```text
//! laya-cli stress --model-dir <bundle> [--iterations 200] [--concurrency 4]
//! ```

mod stress;

use anyhow::{bail, Context, Result};
use clap::Parser;
use laya_engine::LayaAgent;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "laya-cli", about = "Laya System-1 决策模型本地探针与压测")]
struct Args {
    /// ONNX bundle 目录(含 laya.onnx / laya.onnx.data / laya_config.json / tokenizer/)
    #[arg(long)]
    model_dir: PathBuf,

    /// 问题定义 JSON(type/choice/score/noul,同 laya-mlx 形状)
    #[arg(long)]
    questions: PathBuf,

    /// 内联状态:字符串或 JSON 文本
    #[arg(long, conflicts_with = "state_file")]
    state: Option<String>,

    /// 状态 JSON 文件(推荐,例如示例邮件)
    #[arg(long)]
    state_file: Option<PathBuf>,

    /// 每批问题数
    #[arg(long, default_value_t = 16)]
    batch_size: usize,

    /// 重复次数(延迟测量;>=2 时输出 P50/P95)
    #[arg(long, default_value_t = 1)]
    repeat: usize,

    /// ORT 线程数
    #[arg(long, default_value_t = 4)]
    threads: u16,
}

fn main() -> Result<()> {
    // 子命令手动分发:第一个位置参数 = stress
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("stress") {
        let get = |name: &str| -> Option<String> {
            raw.iter()
                .position(|a| a == name)
                .and_then(|i| raw.get(i + 1))
                .cloned()
        };
        let model_dir = get("--model-dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("models/laya-multilingual-onnx"));
        let iterations = get("--iterations").and_then(|v| v.parse().ok()).unwrap_or(200);
        let concurrency = get("--concurrency").and_then(|v| v.parse().ok()).unwrap_or(4);
        let threads = get("--threads").and_then(|v| v.parse().ok()).unwrap_or(4);
        return stress::run(&model_dir, iterations, concurrency, threads);
    }

    let args = Args::parse();
    run_single(args)
}

fn run_single(args: Args) -> Result<()> {

    let state: Value = if let Some(path) = &args.state_file {
        serde_json::from_str(
            &std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?,
        )
        .with_context(|| "parse state json".to_string())?
    } else if let Some(text) = &args.state {
        serde_json::from_str(text).unwrap_or(Value::String(text.clone()))
    } else {
        bail!("需要 --state 或 --state-file 之一")
    };

    let questions_text = std::fs::read_to_string(&args.questions)
        .with_context(|| format!("read {}", args.questions.display()))?;
    let questions: laya_engine::Questions =
        serde_json::from_str(&questions_text).with_context(|| "parse questions json".to_string())?;

    eprintln!(
        "loading model from {} (first load pays ORT init)...",
        args.model_dir.display()
    );
    let load_started = Instant::now();
    let agent = LayaAgent::load(
        &args.model_dir,
        args.batch_size,
        laya_engine::EngineOptions {
            ep: laya_engine::Ep::Cpu,
            intra_threads: args.threads as usize,
            ..Default::default()
        },
    )?;
    eprintln!("model + tokenizer ready in {:?}", load_started.elapsed());

    if !agent.config.clamped.is_empty() {
        eprintln!(
            "warning: checkpoint temperatures clamped (uncalibrated confidence): {:?}",
            agent.config.clamped
        );
    }

    let mut latencies_ms = Vec::with_capacity(args.repeat);
    let mut first_result: Option<Value> = None;
    for i in 0..args.repeat {
        let started = Instant::now();
        let result = agent.predict(&state, &questions)?;
        let elapsed = started.elapsed();
        if i == 0 {
            first_result = Some(result);
        }
        latencies_ms.push(elapsed.as_secs_f64() * 1000.0);
    }

    println!(
        "{}",
        serde_json::to_string_pretty(first_result.as_ref().expect("at least one run"))?
    );

    if args.repeat >= 2 {
        latencies_ms.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        let p = |q: f64| latencies_ms[((latencies_ms.len() as f64 - 1.0) * q).round() as usize];
        eprintln!(
            "latency over {} runs: P50 {:.1} ms | P95 {:.1} ms | min {:.1} ms",
            args.repeat,
            p(0.50),
            p(0.95),
            latencies_ms[0]
        );
    } else {
        eprintln!("latency: {:.1} ms", latencies_ms[0]);
    }
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    // onnxruntime 在进程退出销毁全局线程池时会偶发 fail-fast(0xC0000409);
    // 结果已输出完毕,直接退出跳过 teardown。
    std::process::exit(0);
}

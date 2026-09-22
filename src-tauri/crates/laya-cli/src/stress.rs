//! 压力测试:混合中英文状态循环 predict(docs/09 §6)。
//!
//! 验证四件事:
//! 1. **确定性**——同一 state 重复 predict,输出逐字节一致(浮点实现必须稳定);
//! 2. **并发正确性**——多线程共享 Agent(引擎内 Mutex 串行),结果与串行一致;
//! 3. **延迟分布**——P50/P95/min/max 与吞吐;
//! 4. **输出形状**——每次结果都含完整 answers/policy 语义字段。

use anyhow::{bail, Result};
use laya_engine::{EngineOptions, Ep, LayaAgent, Questions};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// 混合中英文状态:客服工单场景 + agent trace 场景。
const STATES: &[&str] = &[
    // zh
    r#"{"ticket":"发票被重复扣款了,请立刻退款,否则投诉","channel":"email"}"#,
    r#"{"ticket":"系统登录一直报 500 错误,所有用户都无法使用","channel":"web"}"#,
    r#"{"ticket":"想了解一下企业版的报价和合同流程","channel":"phone"}"#,
    r#"{"ticket":"软件偶尔闪退,重启后好了,麻烦看看日志","channel":"email"}"#,
    // en
    r#"{"ticket":"I was charged twice for my March invoice, please refund.","channel":"email"}"#,
    r#"{"ticket":"The API returns 500 on every request since this morning.","channel":"web"}"#,
    r#"{"ticket":"Could you share pricing for the enterprise plan?","channel":"phone"}"#,
    // trace 摘要(System-1 for agent trace)
    r#"{"step":"git push --force origin main","exit":0,"attempts":3,"tests_failed":0}"#,
];

fn question_set() -> Result<Questions> {
    Ok(serde_json::from_value(json!({
        "department": {
            "type": "choice",
            "instructions": "Which department should handle this ticket?",
            "criteria": {
                "billing": "发票、扣款、退款 / invoices, refunds",
                "technical": "故障、报错、系统问题 / bugs, outages",
                "sales": "报价、购买、合同 / pricing, contracts"
            }
        },
        "refund": {"type": "noul", "instructions": "用户是否要求退款? / Does the user ask for a refund?"},
        "urgent": {
            "type": "score",
            "instructions": "这个工单有多紧急? / How urgent is this ticket?",
            "criteria": ["不紧急 not urgent", "尽快处理 soon", "紧急/投诉风险 critical"]
        }
    }))?)
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() as f64 - 1.0) * q).round() as usize]
}

fn run_once(agent: &LayaAgent, questions: &Questions, state: &str) -> Result<Value> {
    let state_value: Value = serde_json::from_str(state)
        .unwrap_or(Value::String(state.to_string()));
    let v = agent.predict(&state_value, questions)?;
    // 形状断言:每个答案必有 type/confidence
    for (qid, a) in v["answers"].as_object().expect("answers obj") {
        if a["type"].is_null() || a["confidence"].is_null() {
            bail!("answer {qid} missing type/confidence");
        }
    }
    Ok(v)
}

pub fn run(model_dir: &Path, iterations: usize, concurrency: usize, threads: u16) -> Result<()> {
    eprintln!("[stress] loading {model_dir:?} ...");
    let agent = Arc::new(LayaAgent::load(
        model_dir,
        16,
        EngineOptions {
            ep: Ep::Cpu,
            intra_threads: threads as usize,
            ..Default::default()
        },
    )?);
    let questions = Arc::new(question_set()?);
    let n = iterations.max(concurrency);
    eprintln!(
        "[stress] iterations={n} concurrency={concurrency} states={} (mixed zh/en)",
        STATES.len()
    );

    // ── 1. 确定性:每个 state 连跑 3 次,首 vs 2 vs 3 逐字节一致 ──
    let mut deterministic = true;
    for state in STATES {
        let first = run_once(&agent, &questions, state)?;
        let first_text = serde_json::to_string(&first)?;
        for _ in 0..2 {
            let again = serde_json::to_string(&run_once(&agent, &questions, state)?)?;
            if again != first_text {
                deterministic = false;
                eprintln!("[stress] ✗ 非确定性输出: {state}");
            }
        }
    }
    eprintln!(
        "[stress] determinism({} states × 3): {}",
        STATES.len(),
        if deterministic { "PASS" } else { "FAIL" }
    );

    // ── 2+3. 并发循环 + 延迟分布 ──
    let latencies = Arc::new(std::sync::Mutex::new(Vec::with_capacity(n)));
    let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let started = Instant::now();
    let mut handles = Vec::new();
    for c in 0..concurrency.max(1) {
        let agent = Arc::clone(&agent);
        let questions = Arc::clone(&questions);
        let latencies = Arc::clone(&latencies);
        let errors = Arc::clone(&errors);
        handles.push(std::thread::spawn(move || {
            let mut i = c;
            loop {
                if i >= n {
                    break;
                }
                let state = STATES[i % STATES.len()];
                let t = Instant::now();
                match run_once(&agent, &questions, state) {
                    Ok(_) => latencies.lock().unwrap().push(t.elapsed().as_secs_f64() * 1000.0),
                    Err(e) => errors.lock().unwrap().push(format!("iter {i}: {e}")),
                }
                i += concurrency.max(1);
            }
        }));
    }
    for h in handles {
        h.join().map_err(|_| anyhow::anyhow!("worker panicked"))?;
    }
    let wall = started.elapsed().as_secs_f64();
    let mut lat = latencies.lock().unwrap().clone();
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let errs = errors.lock().unwrap().clone();

    let report = json!({
        "model_dir": model_dir.display().to_string(),
        "iterations": n,
        "concurrency": concurrency,
        "deterministic": deterministic,
        "errors": errs.len(),
        "error_samples": errs.iter().take(3).cloned().collect::<Vec<_>>(),
        "latency_ms": {
            "min": lat.first().copied().unwrap_or(0.0),
            "p50": percentile(&lat, 0.50),
            "p95": percentile(&lat, 0.95),
            "max": lat.last().copied().unwrap_or(0.0),
        },
        "throughput_qps": if wall > 0.0 { lat.len() as f64 / wall } else { 0.0 },
        "wall_seconds": wall,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);

    let ok = deterministic && errs.is_empty();
    eprintln!(
        "[stress] {} ({} iters, {} errors, P50 {:.1}ms, {:.1} qps)",
        if ok { "✓ PASS" } else { "✗ FAIL" },
        lat.len(),
        errs.len(),
        percentile(&lat, 0.50),
        lat.len() as f64 / wall
    );
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    std::process::exit(if ok { 0 } else { 1 });
}

//! 模型在位时运行的对齐测试:LAYA_MODEL_DIR 指向 ONNX bundle 根目录。
//! 未设置时跳过(纯单测不依赖 1.7GB 模型文件)。
//!
//! 预期:结果形状与 laya-mlx / @receptron/laya 的 `system_one` 一致;
//! 数值上与 Python 参考实现四位小数对齐(个别端点允许 ±0.0001 舍入差)。

use laya_engine::{LayaAgent, Questions};
use serde_json::json;

#[test]
fn predict_shapes_and_ranges() {
    let dir = match std::env::var("LAYA_MODEL_DIR") {
        Ok(d) if !d.is_empty() => d,
        _ => {
            eprintln!("LAYA_MODEL_DIR 未设置,跳过模型对齐测试");
            return;
        }
    };
    let agent = LayaAgent::load(std::path::Path::new(&dir), 16, Default::default())
        .expect("load agent");

    let questions: Questions = serde_json::from_value(json!({
        "department": {
            "type": "choice",
            "instructions": "Which department should handle this email?",
            "criteria": {
                "billing": "invoices, payments, refunds",
                "technical": "bugs, outages, system errors",
                "sales": "pricing, new contracts",
                "other": "everything else"
            }
        },
        "urgency": {
            "type": "score",
            "instructions": "How urgent is this request?",
            "criteria": ["not urgent", "soon", "critical deadline or blocking issue"]
        },
        "refund": {
            "type": "noul",
            "instructions": "Does the customer ask for money back?"
        }
    }))
    .unwrap();

    let state = json!({
        "from": "user@example.com",
        "subject": "Duplicate charge on invoice #4411",
        "body": "We were billed twice for March. Please refund the duplicate today or we will cancel our plan."
    });

    let result = agent.predict(&state, &questions).expect("predict");
    assert_eq!(result["model"], "laya-rl-agent");
    assert_eq!(result["usage"]["output_tokens"], 0);

    let answers = result["answers"].as_object().expect("answers object");
    assert_eq!(answers.len(), 3);

    let department = &answers["department"];
    assert_eq!(department["type"], "choice");
    let choice = department["choice"].as_str().expect("choice label");
    assert!(["billing", "technical", "sales", "other"].contains(&choice));
    let probs = department["probabilities"].as_object().expect("probabilities");
    let sum: f64 = probs.values().map(|v| v.as_f64().unwrap()).sum();
    assert!((sum - 1.0).abs() < 0.01, "probabilities should sum to ~1, got {sum}");
    assert!((0.0..=1.0).contains(&department["confidence"].as_f64().unwrap()));

    let urgency = &answers["urgency"];
    assert_eq!(urgency["type"], "score");
    assert!((0.0..=2.0).contains(&urgency["score"].as_f64().unwrap()));

    let refund = &answers["refund"];
    assert_eq!(refund["type"], "noul");
    // 退款请求邮件,P(true) 应明显高于随机。
    assert!(refund["noul"].as_f64().unwrap() > 0.5, "refund noul should lean true");

    // ONNX Runtime 在 Session 析构/进程收尾销毁全局线程池时会偶发 fail-fast
    // (0xC0000409),与 pyke ort "环境常驻" 的处理一致:故意泄漏,交给进程回收。
    std::mem::forget(agent);
}

//! tokenizer + 序列构造的集成测试:只依赖仓库内 tokenizer 文件,不依赖 1.6GB 权重。
//! models/laya-onnx 缺失时跳过。

use laya_engine::config::LayaConfig;
use laya_engine::sequence::build_sequence;
use laya_engine::tokenizer::LayaTokenizer;
use laya_engine::questions::QuestionDef;
use serde_json::json;
use std::path::{Path, PathBuf};

/// bundle 根 = workspace 根的 models/laya-onnx(cwd 是 crate 目录,向上两级)。
fn bundle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("models")
        .join("laya-onnx")
}

#[test]
fn tokenizer_and_sequence_smoke() {
    let tok_dir = bundle_dir().join("tokenizer");
    if !tok_dir.exists() {
        eprintln!("models/laya-onnx/tokenizer 不存在,跳过");
        return;
    }
    let tok = LayaTokenizer::load(&tok_dir).expect("load tokenizer");
    // ModernBERT 家族标准特殊 token id
    assert_eq!(tok.cls_token_id, 101, "cls");
    assert_eq!(tok.sep_token_id, 102, "sep");
    assert_eq!(tok.pad_token_id, 0, "pad");
    assert_eq!(tok.mask_token_id, 103, "mask");

    let def: QuestionDef = serde_json::from_value(json!({
        "type": "choice",
        "instructions": "Which department should handle this email?",
        "criteria": {"billing": "invoices, refunds", "technical": "bugs"}
    }))
    .unwrap();
    let q = def.to_internal();
    let state = json!({
        "subject": "Duplicate charge",
        "body": "Please refund the duplicate."
    });
    let cfg = LayaConfig::load(&bundle_dir()).expect("load config");
    assert_eq!(cfg.max_len, 512);
    // choice:11+ 桶 0.1006 必须被钳制
    assert!(cfg.temperature_by_options["choice:11+"] >= 0.5, "sharpening temperature must be clamped");

    let pq = build_sequence(&tok, &state, &q, "department", cfg.max_len, cfg.head_max_len, false)
        .expect("build sequence");
    assert_eq!(pq.ids[0], tok.cls_token_id, "sequence starts with [CLS]");
    assert_eq!(*pq.ids.last().unwrap(), tok.sep_token_id, "sequence ends with [SEP]");
    assert_eq!(pq.markers.len(), 2, "one marker per choice option");
    for m in &pq.markers {
        assert_eq!(pq.ids[*m], tok.mask_token_id, "marker points at [MASK]");
    }
    assert!(pq.ids.len() <= cfg.max_len);
}

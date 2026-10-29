//! 模型图选择优先级(docs/16 §4):量化版优先于 fp32。
//!
//! 这是纯文件存在性判断,不加载 ORT,故可在无 DLL 的环境下跑。

// 仓库惯例:测试函数名用中文描述行为,不强制 snake_case。
#![allow(non_snake_case)]

use laya_engine::engine::{resolve_model_file, EngineOptions, QUANTIZED_CANDIDATES};
use std::fs;
use std::path::PathBuf;

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("laya-modelpick-{tag}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("create temp dir");
    d
}

#[test]
fn 没有图文件时报错并指出期望的名字() {
    let d = tmpdir("empty");
    let err = resolve_model_file(&d, "laya.onnx").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("laya.onnx"),
        "错误信息应包含期望文件名,实际: {msg}"
    );
}

#[test]
fn 只有_fp32_时用_fp32() {
    let d = tmpdir("fp32");
    fs::write(d.join("laya.onnx"), b"x").unwrap();
    assert_eq!(resolve_model_file(&d, "laya.onnx").unwrap(), "laya.onnx");
}

#[test]
fn 量化版存在时优先于_fp32() {
    let d = tmpdir("int8");
    fs::write(d.join("laya.onnx"), b"x").unwrap();
    fs::write(d.join("laya.int8.onnx"), b"x").unwrap();
    assert_eq!(
        resolve_model_file(&d, "laya.onnx").unwrap(),
        "laya.int8.onnx"
    );
}

#[test]
fn 调用方显式指定的图文件优先于一切() {
    let d = tmpdir("explicit");
    fs::write(d.join("laya.onnx"), b"x").unwrap();
    fs::write(d.join("laya.int8.onnx"), b"x").unwrap();
    fs::write(d.join("mine.onnx"), b"x").unwrap();
    assert_eq!(resolve_model_file(&d, "mine.onnx").unwrap(), "mine.onnx");
}

#[test]
fn 多个量化候选按声明顺序取第一个() {
    let d = tmpdir("multi");
    fs::write(d.join("laya.q8.onnx"), b"x").unwrap();
    fs::write(d.join("laya.quant.onnx"), b"x").unwrap();
    let got = resolve_model_file(&d, "laya.onnx").unwrap();
    assert_eq!(got, QUANTIZED_CANDIDATES[1]);
}

#[test]
fn 默认引擎选项_关自旋_开_denormal_近似_gelu_线程数交_ORT() {
    let o = EngineOptions::default();
    // 这三条是 CPU 提速开关,任何一条被改回保守值都会丢掉大部分收益
    assert!(
        !o.intra_op_spinning,
        "intra-op 线程池必须关自旋(否则空转吃满 CPU)"
    );
    assert!(
        o.flush_to_zero,
        "必须开 flush_to_zero(softmax 产出大量 denormal)"
    );
    assert!(o.approximate_gelu, "必须开 approximate_gelu(约 10% 且无损)");
    assert_eq!(o.intra_threads, 0, "0 = 交给 ORT 取物理核数");
    assert_eq!(o.model_file, "laya.onnx");
}

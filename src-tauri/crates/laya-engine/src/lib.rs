//! laya-engine:Laya System-1 决策模型的 Rust/ONNX 推理引擎。
//!
//! 语义对齐两个参考实现:
//! - `laya-mlx`(MLX 移植,序列构造/温度钳制/置信度公式来源)
//! - `@receptron/laya`(官方 Node/ONNX,同一 ONNX bundle 的消费方)
//!
//! 输入输出契约(receptron/laya-onnx):
//! - inputs: `input_ids [B,L] i64`, `attention_mask [B,L] i64`,
//!   `marker_pos [B,K] i64`, `marker_mask [B,K] bool`, `qtype [B] i64`
//! - outputs: `logits [B,K] f32`(未校准;masked 槽位 -1e4), `act_probs [B,2] f32`

pub mod agent;
pub mod config;
pub mod engine;
pub mod error;
pub mod questions;
pub mod sequence;
pub mod tokenizer;

pub use agent::{LayaAgent, Questions};
pub use config::LayaConfig;
pub use engine::{ensure_ort_dylib, EngineOptions, Ep, OrtEngine};
pub use error::{LayaError, Result};
pub use questions::{QuestionDef, QuestionKind};
pub use sequence::PreparedQuestion;
pub use tokenizer::LayaTokenizer;

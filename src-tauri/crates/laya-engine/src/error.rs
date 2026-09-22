//! 错误类型。

use crate::questions::QuestionKind;

#[derive(Debug, thiserror::Error)]
pub enum LayaError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("tokenizer: {0}")]
    Tokenizer(String),

    #[error("onnx runtime: {0}")]
    Ort(String),

    #[error("question {qid:?}: {message}")]
    Question { qid: String, message: String },

    #[error("{kind:?} question {qid:?}: too many options for the {budget} token budget")]
    TooManyOptions {
        qid: String,
        kind: QuestionKind,
        budget: usize,
    },

    #[error("model directory {dir:?} is not a complete Laya ONNX bundle (missing {missing:?})")]
    IncompleteBundle { dir: String, missing: String },

    #[error("non-finite model output; retry with float32 weights or a smaller input")]
    NonFiniteOutput,
}

pub type Result<T, E = LayaError> = std::result::Result<T, E>;

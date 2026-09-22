//! 加载 checkpoint 自带的 HuggingFace tokenizer(tokenizer.json)。
//!
//! 与 laya-mlx `tokenizer.py` 相同:底层就是 Rust `tokenizers` 库,这里直接原生调用。
//! 特殊 token(cls/sep/pad/mask)从 `tokenizer_config.json` 解析,兼容字符串与
//! `{"content": "..."}` 两种形式。

use crate::error::{LayaError, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct TokenizerConfig {
    #[serde(default, alias = "cls_token")]
    cls: Option<SpecialToken>,
    #[serde(default, alias = "sep_token")]
    sep: Option<SpecialToken>,
    #[serde(default, alias = "pad_token")]
    pad: Option<SpecialToken>,
    #[serde(default, alias = "mask_token")]
    mask: Option<SpecialToken>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SpecialToken {
    Text(String),
    Detailed { content: String },
}

impl SpecialToken {
    fn content(&self) -> &str {
        match self {
            SpecialToken::Text(s) => s,
            SpecialToken::Detailed { content } => content,
        }
    }
}

pub struct LayaTokenizer {
    backend: tokenizers::Tokenizer,
    pub cls_token: String,
    pub cls_token_id: u32,
    pub sep_token: String,
    pub sep_token_id: u32,
    pub pad_token: String,
    pub pad_token_id: u32,
    pub mask_token: String,
    pub mask_token_id: u32,
}

impl LayaTokenizer {
    pub fn load(dir: &Path) -> Result<Self> {
        let json_path = dir.join("tokenizer.json");
        if !json_path.exists() {
            return Err(LayaError::IncompleteBundle {
                dir: dir.display().to_string(),
                missing: "tokenizer/tokenizer.json".into(),
            });
        }
        let backend = tokenizers::Tokenizer::from_file(&json_path)
            .map_err(|e| LayaError::Tokenizer(e.to_string()))?;

        let cfg_path = dir.join("tokenizer_config.json");
        let cfg: TokenizerConfig = if cfg_path.exists() {
            serde_json::from_str(&std::fs::read_to_string(&cfg_path)?)
                .map_err(|e| LayaError::Tokenizer(format!("tokenizer_config.json: {e}")))?
        } else {
            serde_json::from_str("{}")?
        };

        fn resolve(
            backend: &tokenizers::Tokenizer,
            cfg_value: Option<SpecialToken>,
            name: &str,
        ) -> Result<(String, u32)> {
            let content = cfg_value
                .as_ref()
                .map(|t| t.content().to_string())
                .ok_or_else(|| {
                    LayaError::Tokenizer(format!("tokenizer_config.json is missing {name}"))
                })?;
            let id = backend
                .token_to_id(&content)
                .ok_or_else(|| {
                    LayaError::Tokenizer(format!(
                        "token {content:?} ({name}) not found in tokenizer.json"
                    ))
                })?;
            Ok((content, id))
        }

        let (cls_token, cls_token_id) = resolve(&backend, cfg.cls, "cls_token")?;
        let (sep_token, sep_token_id) = resolve(&backend, cfg.sep, "sep_token")?;
        let (pad_token, pad_token_id) = resolve(&backend, cfg.pad, "pad_token")?;
        let (mask_token, mask_token_id) = resolve(&backend, cfg.mask, "mask_token")?;

        Ok(Self {
            backend,
            cls_token,
            cls_token_id,
            sep_token,
            sep_token_id,
            pad_token,
            pad_token_id,
            mask_token,
            mask_token_id,
        })
    }

    /// `tok(text, add_special_tokens=False)["input_ids"]`。
    pub fn encode_no_specials(&self, text: &str) -> Result<Vec<u32>> {
        let encoding = self
            .backend
            .encode(text, false)
            .map_err(|e| LayaError::Tokenizer(e.to_string()))?;
        Ok(encoding.get_ids().to_vec())
    }
}

//! `system_one` 编排:构造序列 → collate → forward → 温度校准 → 概率/置信度,
//! 结果形状与 laya-mlx `Agent.predict`(即上游 `RLAgent.system_one`)一致。

use crate::config::{LayaConfig, QuestionKindAsInt};
use crate::engine::{BatchInput, EngineOptions, OrtEngine};
use crate::error::{LayaError, Result};
use crate::questions::{InternalQuestion, QuestionDef, QuestionKind};
use crate::sequence::{build_sequence, PreparedQuestion};
use crate::tokenizer::LayaTokenizer;
use serde_json::{json, Map, Value};
use std::path::Path;

/// 问题表:`qid → 问题定义 JSON`。用 serde_json::Map(preserve_order)保持插入顺序。
pub type Questions = Map<String, Value>;

pub struct LayaAgent {
    pub config: LayaConfig,
    pub tokenizer: LayaTokenizer,
    pub engine: OrtEngine,
    pub batch_size: usize,
}

impl LayaAgent {
    /// `model_dir` 是 ONNX bundle 根目录(receptron/laya-onnx 布局,或其中任一子目录)。
    pub fn load(model_dir: &Path, batch_size: usize, options: EngineOptions) -> Result<Self> {
        if !model_dir.exists() {
            return Err(LayaError::IncompleteBundle {
                dir: model_dir.display().to_string(),
                missing: "(directory itself)".into(),
            });
        }
        let config = LayaConfig::load(model_dir)?;
        // receptron/laya-onnx 布局 tokenizer 在子目录;兼容平铺布局。
        let tok_dir = if model_dir.join("tokenizer").is_dir() {
            model_dir.join("tokenizer")
        } else {
            model_dir.to_path_buf()
        };
        let tokenizer = LayaTokenizer::load(&tok_dir)?;
        let engine = OrtEngine::new(model_dir, options)?;
        Ok(Self {
            config,
            tokenizer,
            engine,
            batch_size: batch_size.max(1),
        })
    }

    fn prepare(
        &self,
        state: &Value,
        questions: &Questions,
    ) -> Result<Vec<(String, PreparedQuestion)>> {
        let mut prepared = Vec::with_capacity(questions.len());
        for (qid, def_value) in questions {
            let def: QuestionDef = serde_json::from_value(def_value.clone()).map_err(|e| {
                LayaError::Question {
                    qid: qid.clone(),
                    message: e.to_string(),
                }
            })?;
            let internal: InternalQuestion = def.to_internal();
            let pq = build_sequence(
                &self.tokenizer,
                state,
                &internal,
                qid,
                self.config.max_len,
                self.config.head_max_len,
                false,
            )?;
            prepared.push((qid.clone(), pq));
        }
        Ok(prepared)
    }

    /// 对齐 `Agent.system_one`,返回 `{model, answers, usage}` JSON。
    pub fn predict(&self, state: &Value, questions: &Questions) -> Result<Value> {
        if questions.is_empty() {
            return Err(LayaError::Question {
                qid: "(all)".into(),
                message: "questions must not be empty".into(),
            });
        }
        let prepared = self.prepare(state, questions)?;
        let mut answers = Map::new();
        let mut input_tokens = 0usize;

        for chunk in prepared.chunks(self.batch_size) {
            let batch = collate(chunk, self.tokenizer.pad_token_id);
            let out = self.engine.run(&batch)?;
            for (row, (qid, pq)) in chunk.iter().enumerate() {
                input_tokens += pq.ids.len();
                let k = pq.markers.len();
                let row_logits: Vec<f64> = out.logits[row * batch.max_markers..row * batch.max_markers + k]
                    .iter()
                    .map(|v| *v as f64)
                    .collect();
                let act_prob = out.act_probs[row * 2] as f64;
                answers.insert(
                    qid.clone(),
                    build_answer(pq, &row_logits, act_prob, &self.config),
                );
            }
        }

        Ok(json!({
            "model": "laya-rl-agent",
            "answers": Value::Object(answers),
            "usage": {"input_tokens": input_tokens, "output_tokens": 0}
        }))
    }
}

/// laya-mlx `collate_items`:右侧 padding,marker 槽位数取 batch 内最大(至少 2)。
fn collate(items: &[(String, PreparedQuestion)], pad_id: u32) -> BatchInput {
    let n = items.len();
    let seq_len = items.iter().map(|(_, p)| p.ids.len()).max().unwrap_or(1);
    let max_markers = std::cmp::max(2, items.iter().map(|(_, p)| p.markers.len()).max().unwrap_or(2));

    let mut input_ids = vec![pad_id as i64; n * seq_len];
    let mut attention_mask = vec![0i64; n * seq_len];
    let mut marker_pos = vec![0i64; n * max_markers];
    let mut marker_mask = vec![false; n * max_markers];
    let mut qtype = vec![0i64; n];

    for (i, (_, p)) in items.iter().enumerate() {
        for (j, id) in p.ids.iter().enumerate() {
            input_ids[i * seq_len + j] = *id as i64;
            attention_mask[i * seq_len + j] = 1;
        }
        for (j, m) in p.markers.iter().enumerate() {
            marker_pos[i * max_markers + j] = *m as i64;
            marker_mask[i * max_markers + j] = true;
        }
        qtype[i] = p.qtype as i64;
    }

    BatchInput {
        input_ids,
        attention_mask,
        marker_pos,
        marker_mask,
        qtype,
        batch: n,
        seq_len,
        max_markers,
    }
}

fn build_answer(pq: &PreparedQuestion, row_logits: &[f64], act_prob: f64, cfg: &LayaConfig) -> Value {
    let k = pq.markers.len();
    let kind = match pq.qtype {
        0 => QuestionKind::Choice,
        1 => QuestionKind::Score,
        _ => QuestionKind::Noul,
    };
    let scale = cfg.temperature_for(QuestionKindAsInt(pq.qtype), k);
    let p = softmax_scaled(row_logits, scale);

    let mut answer = json!({
        "type": format!("{kind:?}").to_lowercase(),
        "confidence": r4(entropy_confidence(&p, k)),
        "action": {"act_probability": r4(act_prob)},
    });

    match kind {
        QuestionKind::Choice => {
            let best = argmax(&p);
            let labels = &pq.choice_labels;
            let mut probabilities = Map::new();
            for (label, prob) in labels.iter().zip(p.iter()) {
                probabilities.insert(label.clone(), json!(r4(*prob)));
            }
            answer["choice"] = json!(labels[best]);
            answer["probabilities"] = Value::Object(probabilities);
        }
        QuestionKind::Score => {
            let score: f64 = p.iter().enumerate().map(|(i, v)| i as f64 * v).sum();
            let mut legend = Map::new();
            for (i, crit) in pq.score_criteria.iter().enumerate() {
                legend.insert(i.to_string(), json!(crit));
            }
            let mut probabilities = Map::new();
            for (i, prob) in p.iter().enumerate() {
                probabilities.insert(i.to_string(), json!(r4(*prob)));
            }
            answer["score"] = json!(r4(score));
            answer["legend"] = Value::Object(legend);
            answer["probabilities"] = Value::Object(probabilities);
        }
        QuestionKind::Noul => {
            let p_true = p[1];
            answer["noul"] = json!(r4(p_true));
            answer["confidence"] = json!(r4(p_true.max(1.0 - p_true)));
        }
    }
    answer
}

fn softmax_scaled(logits: &[f64], scale: f64) -> Vec<f64> {
    let z: Vec<f64> = logits.iter().map(|v| v / scale).collect();
    let max = z.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut exp: Vec<f64> = z.iter().map(|v| (v - max).exp()).collect();
    let sum: f64 = exp.iter().sum();
    for v in exp.iter_mut() {
        *v /= sum;
    }
    exp
}

/// laya-mlx `confidence_from_probs`:归一化香农熵置信度 `1 - H(p) / log(k)`。
fn entropy_confidence(p: &[f64], k: usize) -> f64 {
    if k < 2 {
        return 1.0;
    }
    let ent: f64 = p
        .iter()
        .take(k)
        .map(|v| -v * v.max(1e-12).ln())
        .sum();
    (1.0 - ent / (k as f64).ln()).clamp(0.0, 1.0)
}

fn argmax(p: &[f64]) -> usize {
    let mut best = 0usize;
    for (i, v) in p.iter().enumerate() {
        if *v > p[best] {
            best = i;
        }
    }
    best
}

/// 保留四位小数(Python `round(x, 4)` 的近似对齐)。
fn r4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_sums_to_one() {
        let p = softmax_scaled(&[1.0, 2.0, 3.0], 1.0);
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert_eq!(argmax(&p), 2);
    }

    #[test]
    fn entropy_confidence_bounds() {
        let uniform = vec![0.25, 0.25, 0.25, 0.25];
        assert!(entropy_confidence(&uniform, 4) < 1e-9);
        let peaked = vec![0.999, 0.001];
        assert!(entropy_confidence(&peaked, 2) > 0.98);
    }

    #[test]
    fn r4_matches() {
        assert_eq!(r4(0.123456), 0.1235);
        assert_eq!(r4(0.8921), 0.8921);
    }
}

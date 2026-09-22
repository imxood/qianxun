//! Typed questions:choice / score / noul。
//!
//! JSON 形状与 laya-mlx / @receptron/laya 完全一致:
//! ```json
//! {
//!   "department": {"type": "choice", "instructions": "...", "criteria": {"billing": "..."}},
//!   "urgency":    {"type": "score",  "instructions": "...", "criteria": ["not", "soon", "critical"]},
//!   "refund":     {"type": "noul",   "instructions": "..."}
//! }
//! ```

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    Choice,
    Score,
    Noul,
}

impl QuestionKind {
    pub fn as_int(self) -> u8 {
        match self {
            QuestionKind::Choice => 0,
            QuestionKind::Score => 1,
            QuestionKind::Noul => 2,
        }
    }
}

/// noul 的 criteria:{"false": "描述", "true": "描述"},两侧均可省略。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NoulCriteria {
    #[serde(default)]
    pub r#false: Option<Value>,
    #[serde(default)]
    pub r#true: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum QuestionDef {
    Choice {
        instructions: Value,
        /// 用 serde_json::Map(preserve_order feature)保持选项顺序 = 标签顺序。
        criteria: serde_json::Map<String, Value>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
    Noul {
        instructions: Value,
        #[serde(default)]
        criteria: Option<NoulCriteria>,
    },
}

/// 内部归一化形式(laya-mlx `_to_internal` 的 Rust 版)。
#[derive(Debug, Clone)]
pub struct InternalQuestion {
    pub kind: QuestionKind,
    pub instructions: String,
    pub choice_labels: Vec<String>,
    pub choice_values: Vec<Option<String>>,
    pub score_criteria: Vec<String>,
    pub noul: Option<NoulCriteria>,
}

fn instructions_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        // 与 Python 侧一致:非字符串 instructions 用 JSON 序列化。
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// laya-mlx `common.render_criterion`:字符串直传,结构化值转紧凑 JSON。
pub fn render_criterion(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => to_spaced_json(other),
    }
}

/// `json.dumps(v, ensure_ascii=False)` 的默认分隔符是 `(", ", ": ")`。
/// serde_json 紧凑输出不含空格;这里按 Python 默认风格递归补齐,保证提示序列与参考实现逐字节对齐。
pub fn to_spaced_json(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(to_spaced_json).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", serde_json::to_string(k).unwrap_or_default(), to_spaced_json(v)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

impl QuestionDef {
    pub fn to_internal(&self) -> InternalQuestion {
        match self {
            QuestionDef::Choice {
                instructions,
                criteria,
            } => InternalQuestion {
                kind: QuestionKind::Choice,
                instructions: instructions_to_string(instructions),
                choice_labels: criteria.keys().cloned().collect(),
                choice_values: Some(
                    criteria
                        .values()
                        .map(|v| match v {
                            Value::Null => None,
                            Value::String(s) if s.is_empty() => None,
                            other => Some(render_criterion(other)),
                        })
                        .collect(),
                )
                .unwrap_or_default(),
                score_criteria: Vec::new(),
                noul: None,
            },
            QuestionDef::Score {
                instructions,
                criteria,
            } => InternalQuestion {
                kind: QuestionKind::Score,
                instructions: instructions_to_string(instructions),
                choice_labels: Vec::new(),
                choice_values: Vec::new(),
                score_criteria: criteria.iter().map(render_criterion).collect(),
                noul: None,
            },
            QuestionDef::Noul {
                instructions,
                criteria,
            } => InternalQuestion {
                kind: QuestionKind::Noul,
                instructions: instructions_to_string(instructions),
                choice_labels: Vec::new(),
                choice_values: Vec::new(),
                score_criteria: Vec::new(),
                noul: criteria.clone(),
            },
        }
    }
}

/// laya-mlx `common.render_options`:选项文本按标签顺序;noul 恒为 [false, true]。
pub fn render_options(q: &InternalQuestion) -> Vec<String> {
    match q.kind {
        QuestionKind::Choice => q
            .choice_labels
            .iter()
            .zip(q.choice_values.iter())
            .map(|(k, v)| match v {
                None => k.clone(),
                Some(desc) => format!("{k}: {desc}"),
            })
            .collect(),
        QuestionKind::Score => q
            .score_criteria
            .iter()
            .enumerate()
            .map(|(i, c)| format!("level {i}: {c}"))
            .collect(),
        QuestionKind::Noul => {
            let crit = q.noul.clone().unwrap_or_default();
            let is_blank = |v: &Option<Value>| match v {
                None | Some(Value::Null) => true,
                Some(Value::String(s)) => s.is_empty(),
                Some(_) => false,
            };
            let false_desc = if is_blank(&crit.r#false) {
                "no, the statement does not hold".to_string()
            } else {
                render_criterion(crit.r#false.as_ref().expect("checked non-blank"))
            };
            let true_desc = if is_blank(&crit.r#true) {
                "yes, the statement holds".to_string()
            } else {
                render_criterion(crit.r#true.as_ref().expect("checked non-blank"))
            };
            vec![format!("false: {false_desc}"), format!("true: {true_desc}")]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_choice_options() {
        let def: QuestionDef = serde_json::from_value(serde_json::json!({
            "type": "choice",
            "instructions": "Which department?",
            "criteria": {"billing": "invoices, refunds", "other": ""}
        }))
        .unwrap();
        let q = def.to_internal();
        assert_eq!(
            render_options(&q),
            vec!["billing: invoices, refunds", "other"]
        );
    }

    #[test]
    fn renders_score_and_noul() {
        let def: QuestionDef = serde_json::from_value(serde_json::json!({
            "type": "score", "instructions": "How urgent?",
            "criteria": ["not urgent", "soon"]
        }))
        .unwrap();
        assert_eq!(
            render_options(&def.to_internal()),
            vec!["level 0: not urgent", "level 1: soon"]
        );

        let def: QuestionDef = serde_json::from_value(serde_json::json!({
            "type": "noul", "instructions": "Money back?"
        }))
        .unwrap();
        assert_eq!(
            render_options(&def.to_internal()),
            vec![
                "false: no, the statement does not hold",
                "true: yes, the statement holds"
            ]
        );
    }

    #[test]
    fn spaced_json_matches_python_defaults() {
        let v: Value =
            serde_json::from_str(r#"{"a": 1, "b": [1, 2], "c": "x, y: z"}"#).unwrap();
        assert_eq!(to_spaced_json(&v), r#"{"a": 1, "b": [1, 2], "c": "x, y: z"}"#);
    }
}

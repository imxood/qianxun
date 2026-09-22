//! Laya 提示序列构造,逐行对齐 laya-mlx `common.py::build_prefix / build_sequence`。
//!
//! 格式:`[CLS] <type> question: <instructions> [SEP] [MASK] opt0 [MASK] opt1 ... [SEP] state [SEP]`
//! 每个 `[MASK]` 是该选项决策头的读取位置(marker),head 预算 `head_max_len`(默认 192),
//! 全序列预算 `max_len`(默认 512/1024)。

use crate::config::QuestionKindAsInt;
use crate::error::{LayaError, Result};
use crate::questions::{render_options, InternalQuestion, QuestionKind};
use crate::tokenizer::LayaTokenizer;
use serde_json::Value;

/// 每个选项文本最多保留 48 个 token(不含前置 [MASK])。
const OPTION_TOKEN_LIMIT: usize = 48;
/// head 预算的最小保底。
const HEAD_FLOOR: usize = 8;
/// 选项预算兜底阈值。
const OPT_BUDGET_FLOOR: usize = 16;

/// 状态:字符串直传;dict/list 走 Python `json.dumps(..., ensure_ascii=False)` 风格的带空格 JSON。
pub fn serialize_state(state: &Value) -> String {
    match state {
        Value::String(s) => s.clone(),
        other => crate::questions::to_spaced_json(other),
    }
}

#[derive(Debug, Clone)]
pub struct PreparedQuestion {
    pub ids: Vec<u32>,
    pub markers: Vec<usize>,
    pub qtype: u8,
    pub n_options: usize,
    /// choice 题的标签顺序,其余题为空。
    pub choice_labels: Vec<String>,
    /// score 题的分档描述,其余题为空。
    pub score_criteria: Vec<String>,
}

impl PreparedQuestion {
    pub fn kind(&self) -> QuestionKindAsInt {
        QuestionKindAsInt(self.qtype)
    }
}

/// laya-mlx `build_prefix`:先构造"仅问题"前缀,再拼 state 并做最终截断。
pub fn build_prefix(
    tok: &LayaTokenizer,
    q: &InternalQuestion,
    head_max_len: usize,
) -> Result<(Vec<u32>, Vec<usize>)> {
    let opts = render_options(q);
    let type_text = match q.kind {
        QuestionKind::Choice => "choice",
        QuestionKind::Score => "score",
        QuestionKind::Noul => "noul",
    };

    let ins = q.instructions.replace(&tok.mask_token, " ");
    let head_text = format!("{type_text} question: {ins}");
    let mut head_ids = tok.encode_no_specials(&head_text)?;

    let mut opt_ids: Vec<Vec<u32>> = Vec::with_capacity(opts.len());
    for opt in &opts {
        let text = format!(" {}", opt.replace(&tok.mask_token, " "));
        let mut ids = tok.encode_no_specials(&text)?;
        ids.truncate(OPTION_TOKEN_LIMIT);
        let mut full = vec![tok.mask_token_id];
        full.extend(ids);
        opt_ids.push(full);
    }

    let mut opt_budget: isize =
        head_max_len as isize - opt_ids.iter().map(|o| o.len() as isize).sum::<isize>();
    if opt_budget < OPT_BUDGET_FLOOR as isize {
        let per = std::cmp::max(
            4usize,
            (head_max_len.saturating_sub(OPT_BUDGET_FLOOR)) / std::cmp::max(1, opt_ids.len()),
        );
        for o in opt_ids.iter_mut() {
            o.truncate(per);
        }
        opt_budget =
            head_max_len as isize - opt_ids.iter().map(|o| o.len() as isize).sum::<isize>();
    }

    let head_keep = std::cmp::max(HEAD_FLOOR as isize, opt_budget).max(0) as usize;
    head_ids.truncate(head_keep);

    let mut ids = Vec::with_capacity(head_ids.len() + opt_ids.iter().map(|o| o.len()).sum::<usize>() + 2);
    ids.push(tok.cls_token_id);
    ids.extend(head_ids);
    ids.push(tok.sep_token_id);

    let mut markers = Vec::with_capacity(opt_ids.len());
    for o in &opt_ids {
        markers.push(ids.len());
        ids.extend(o);
    }
    ids.push(tok.sep_token_id);
    Ok((ids, markers))
}

/// laya-mlx `build_sequence`。`truncate_left=false`(默认)保留 state 开头,`true` 保留结尾。
pub fn build_sequence(
    tok: &LayaTokenizer,
    state: &Value,
    q: &InternalQuestion,
    qid: &str,
    max_len: usize,
    head_max_len: usize,
    truncate_left: bool,
) -> Result<PreparedQuestion> {
    let kind = q.kind;
    let (mut ids, markers) = build_prefix(tok, q, head_max_len)?;

    let room = max_len.saturating_sub(ids.len() + 1);
    let state_text = serialize_state(state).replace(&tok.mask_token, " ");
    let mut st = tok.encode_no_specials(&state_text)?;
    if st.len() > room {
        st = if truncate_left {
            st[st.len() - room..].to_vec()
        } else {
            st[..room].to_vec()
        };
    }
    ids.extend(st);
    ids.push(tok.sep_token_id);
    ids.truncate(max_len);

    let kept_markers: Vec<usize> = markers.into_iter().filter(|m| *m < max_len).collect();
    let n_options = match kind {
        QuestionKind::Noul => 2,
        _ => kept_markers.len(),
    };
    if kept_markers.is_empty() {
        return Err(LayaError::TooManyOptions {
            qid: qid.to_string(),
            kind,
            budget: max_len,
        });
    }
    Ok(PreparedQuestion {
        ids,
        markers: kept_markers,
        qtype: kind.as_int(),
        n_options,
        choice_labels: q.choice_labels.clone(),
        score_criteria: q.score_criteria.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_state_matches_python() {
        let v: Value = serde_json::from_str(
            r#"{"a": 1, "b": "文本", "list": [1, 2]}"#,
        )
        .unwrap();
        assert_eq!(serialize_state(&v), r#"{"a": 1, "b": "文本", "list": [1, 2]}"#);
        assert_eq!(serialize_state(&Value::String("plain".into())), "plain");
    }
}

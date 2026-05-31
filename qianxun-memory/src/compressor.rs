use crate::types::{Observation, ObservationType};
use chrono::Utc;
use serde_json::Value;

/// 合成压缩 —— 根据工具类型启发式提取结构化记忆体。
///
/// 不调用 LLM，0 token 消耗，< 0.1ms。
pub fn build_synthetic(
    obs_id: String,
    session_id: String,
    hook_type: &str,
    tool_name: &str,
    tool_input: Option<&Value>,
    tool_output: Option<&str>,
) -> Observation {
    let timestamp = Utc::now();
    let is_post = hook_type == "PostToolUse";

    match tool_name {
        "read_file" => compress_read(obs_id, session_id, timestamp, tool_input, is_post),
        "write_file" => compress_write(obs_id, session_id, timestamp, tool_input, is_post),
        "edit_file" => compress_edit(obs_id, session_id, timestamp, tool_input, is_post),
        "execute_command" | "terminal" => {
            compress_terminal(obs_id, session_id, timestamp, tool_input, tool_output, is_post)
        }
        "grep" | "search" | "glob" => {
            compress_search(obs_id, session_id, timestamp, tool_input, is_post)
        }
        _ => compress_default(obs_id, session_id, timestamp, tool_name, tool_input),
    }
}

fn compress_read(
    id: String,
    session_id: String,
    ts: chrono::DateTime<chrono::Utc>,
    input: Option<&Value>,
    _is_post: bool,
) -> Observation {
    let path = extract_path(input).unwrap_or_else(|| "未知".to_string());
    Observation {
        id,
        session_id,
        timestamp: ts,
        obs_type: ObservationType::FileRead,
        title: format!("读取文件: {path}"),
        subtitle: None,
        facts: vec![],
        narrative: format!("读取了文件 {path}"),
        concepts: extract_concepts_from_path(&path),
        files: vec![path.to_string()],
        importance: 3,
        confidence: None,
    }
}

fn compress_write(
    id: String,
    session_id: String,
    ts: chrono::DateTime<chrono::Utc>,
    input: Option<&Value>,
    _is_post: bool,
) -> Observation {
    let path = extract_path(input).unwrap_or_else(|| "未知".to_string());
    Observation {
        id,
        session_id,
        timestamp: ts,
        obs_type: ObservationType::FileWrite,
        title: format!("写入文件: {path}"),
        subtitle: None,
        facts: vec![],
        narrative: format!("写入了文件 {path}"),
        concepts: extract_concepts_from_path(&path),
        files: vec![path.to_string()],
        importance: 5,
        confidence: None,
    }
}

fn compress_edit(
    id: String,
    session_id: String,
    ts: chrono::DateTime<chrono::Utc>,
    input: Option<&Value>,
    _is_post: bool,
) -> Observation {
    let path = extract_path(input).unwrap_or_else(|| "未知".to_string());
    // 从 input 中提取 diff 摘要（仅限 old/new 片段首行）
    let summary = input
        .and_then(|v| v.get("old"))
        .and_then(|v| v.as_str())
        .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    let title = if summary.is_empty() {
        format!("编辑文件: {path}")
    } else {
        format!("编辑 {path}: {summary}")
    };

    Observation {
        id,
        session_id,
        timestamp: ts,
        obs_type: ObservationType::FileEdit,
        title,
        subtitle: None,
        facts: vec![],
        narrative: format!("编辑了文件 {path}"),
        concepts: extract_concepts_from_path(&path),
        files: vec![path.to_string()],
        importance: 6,
        confidence: None,
    }
}

fn compress_terminal(
    id: String,
    session_id: String,
    ts: chrono::DateTime<chrono::Utc>,
    input: Option<&Value>,
    output: Option<&str>,
    _is_post: bool,
) -> Observation {
    let cmd = input
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .unwrap_or("未知命令")
        .to_string();

    let has_error = output.map(|o| {
        let lower = o.to_lowercase();
        lower.contains("error")
            || lower.contains("failed")
            || lower.contains("panic")
            || lower.contains("traceback")
            || o.contains("exit code: 1")
    }).unwrap_or(false);

    let importance = if has_error { 8 } else { 4 };
    let obs_type = if has_error {
        ObservationType::Error
    } else {
        ObservationType::CommandRun
    };

    // 只保留输出首尾各 3 行
    let output_summary = output
        .map(|o| {
            let lines: Vec<&str> = o.lines().collect();
            if lines.len() <= 6 {
                o.to_string()
            } else {
                let head = lines[..3].join("\n");
                let tail = lines[lines.len() - 3..].join("\n");
                format!("{head}\n...\n{tail}")
            }
        })
        .unwrap_or_default();

    Observation {
        id,
        session_id,
        timestamp: ts,
        obs_type,
        title: format!("{}: {cmd}", if has_error { "报错" } else { "执行" }),
        subtitle: None,
        facts: vec![],
        narrative: format!("{cmd}\n输出摘要:\n{output_summary}"),
        concepts: vec![],
        files: vec![],
        importance,
        confidence: None,
    }
}

fn compress_search(
    id: String,
    session_id: String,
    ts: chrono::DateTime<chrono::Utc>,
    input: Option<&Value>,
    _is_post: bool,
) -> Observation {
    let query = input
        .and_then(|v| v.get("query").or_else(|| v.get("pattern")))
        .and_then(|v| v.as_str())
        .unwrap_or("未知搜索")
        .to_string();

    Observation {
        id,
        session_id,
        timestamp: ts,
        obs_type: ObservationType::Search,
        title: format!("搜索: {query}"),
        subtitle: None,
        facts: vec![],
        narrative: format!("执行了搜索: {query}"),
        concepts: vec![query.to_string()],
        files: vec![],
        importance: 2,
        confidence: None,
    }
}

fn compress_default(
    id: String,
    session_id: String,
    ts: chrono::DateTime<chrono::Utc>,
    tool_name: &str,
    _input: Option<&Value>,
) -> Observation {
    Observation {
        id,
        session_id,
        timestamp: ts,
        obs_type: ObservationType::Other,
        title: format!("调用工具: {tool_name}"),
        subtitle: None,
        facts: vec![],
        narrative: format!("调用了 {tool_name}"),
        concepts: vec![tool_name.to_string()],
        files: vec![],
        importance: 2,
        confidence: None,
    }
}

fn extract_path(input: Option<&Value>) -> Option<String> {
    input
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn extract_concepts_from_path(path: &str) -> Vec<String> {
    use std::path::Path;
    let p = Path::new(path);
    let mut concepts = Vec::new();
    if let Some(ext) = p.extension() {
        concepts.push(ext.to_string_lossy().to_string());
    }
    if let Some(name) = p.file_stem() {
        concepts.push(name.to_string_lossy().to_string());
    }
    concepts
}

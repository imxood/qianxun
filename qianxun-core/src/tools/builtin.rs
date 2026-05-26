use super::{AgentTool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct ReadTextFileTool;

#[async_trait]
impl AgentTool for ReadTextFileTool {
    fn name(&self) -> &str {
        "read_text_file"
    }

    fn description(&self) -> &str {
        "读取指定文件的内容"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件路径"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing path".into()))?;

        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                let truncated = if content.len() > 100_000 {
                    let head = &content[..50_000];
                    let tail = &content[content.len() - 50_000..];
                    format!("{head}\n... [truncated, total {} bytes]\n{tail}", content.len())
                } else {
                    content
                };
                Ok(ToolOutput {
                    content: truncated,
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolOutput {
                content: format!("Error reading file: {e}"),
                is_error: true,
            }),
        }
    }
}

pub struct WriteTextFileTool;

#[async_trait]
impl AgentTool for WriteTextFileTool {
    fn name(&self) -> &str {
        "write_text_file"
    }

    fn description(&self) -> &str {
        "写入内容到指定文件"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing path".into()))?;
        let content = arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing content".into()))?;

        // Create parent dir if needed
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }

        match tokio::fs::write(path, content).await {
            Ok(_) => Ok(ToolOutput {
                content: format!("Successfully wrote {} bytes to {path}", content.len()),
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: format!("Error writing file: {e}"),
                is_error: true,
            }),
        }
    }
}

pub struct SearchTool;

#[async_trait]
impl AgentTool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "搜索文件名（递归），支持 glob 模式匹配"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "文件 glob 模式，例如 *.rs 或 **/Cargo.toml"
                },
                "path": {
                    "type": "string",
                    "description": "搜索起始目录，默认当前目录"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let pattern = arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing pattern".into()))?;
        let root = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let mut results = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(root)];
        let max_results = 200;

        while let Some(dir) = dirs.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(r) => r,
                Err(_) => continue,
            };

            loop {
                let entry = match entries.next_entry().await {
                    Ok(Some(e)) => e,
                    Ok(None) => break,
                    Err(_) => continue,
                };
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if glob_match(pattern, name) {
                        results.push(path.to_string_lossy().to_string());
                        if results.len() >= max_results {
                            break;
                        }
                    }
                }
            }
            if results.len() >= max_results {
                break;
            }
        }

        if results.is_empty() {
            Ok(ToolOutput {
                content: format!("No files matching '{pattern}' found under {root}"),
                is_error: false,
            })
        } else {
            Ok(ToolOutput {
                content: results.join("\n"),
                is_error: false,
            })
        }
    }
}

/// 简单的 glob 模式匹配（支持 `*`, `?`, `[...]`）
fn glob_match(pattern: &str, name: &str) -> bool {
    let pat_chars: Vec<char> = pattern.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();
    glob_match_rec(&pat_chars, &name_chars, 0, 0)
}

fn glob_match_rec(p: &[char], s: &[char], pi: usize, si: usize) -> bool {
    if pi == p.len() {
        return si == s.len();
    }
    match p[pi] {
        '*' => {
            // '*' matches any sequence (including empty)
            if pi + 1 < p.len() && p[pi + 1] == '*' {
                // '**' — skip; handled as single * here for simplicity
                glob_match_rec(p, s, pi + 2, si)
                    || (si < s.len() && glob_match_rec(p, s, pi, si + 1))
            } else {
                glob_match_rec(p, s, pi + 1, si)
                    || (si < s.len() && glob_match_rec(p, s, pi, si + 1))
            }
        }
        '?' => {
            if si < s.len() {
                glob_match_rec(p, s, pi + 1, si + 1)
            } else {
                false
            }
        }
        '[' => {
            // Simple bracket expression: [abc] or [a-z]
            if let Some(end) = p[pi..].iter().position(|&c| c == ']') {
                let alt_end = pi + end;
                let mut matched = false;
                let mut ai = pi + 1;
                while ai < alt_end {
                    if ai + 2 < alt_end && p[ai + 1] == '-' {
                        if s[si] >= p[ai] && s[si] <= p[ai + 2] {
                            matched = true;
                            break;
                        }
                        ai += 3;
                    } else {
                        if s[si] == p[ai] {
                            matched = true;
                            break;
                        }
                        ai += 1;
                    }
                }
                if matched {
                    glob_match_rec(p, s, alt_end + 1, si + 1)
                } else {
                    false
                }
            } else {
                // Unclosed bracket — treat as literal
                s.get(si) == Some(&p[pi]) && glob_match_rec(p, s, pi + 1, si + 1)
            }
        }
        c => {
            if si < s.len() && (c == s[si] || (c == '\\' && pi + 1 < p.len() && p[pi + 1] == s[si]))
            {
                glob_match_rec(p, s, pi + 1, si + 1)
            } else {
                false
            }
        }
    }
}

/// 注册所有内置工具到 ToolRegistry
pub fn register_all(registry: &mut super::ToolRegistry) {
    use std::sync::Arc;
    registry.register_builtin(Arc::new(ReadTextFileTool));
    registry.register_builtin(Arc::new(WriteTextFileTool));
    registry.register_builtin(Arc::new(SearchTool));
}

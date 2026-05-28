use super::{AgentTool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::OnceLock;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

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

        let meta = match tokio::fs::metadata(path).await {
            Ok(m) => m,
            Err(e) => return Ok(ToolOutput {
                content: format!("Error reading file: {e}"),
                is_error: true,
            }),
        };

        if meta.len() > 100_000 {
            // 大文件：只读头尾各 50K，避免完整加载
            let mut file = match tokio::fs::File::open(path).await {
                Ok(f) => f,
                Err(e) => return Ok(ToolOutput {
                    content: format!("Error opening file: {e}"),
                    is_error: true,
                }),
            };

            let mut head = vec![0u8; 50_000.min(meta.len() as usize)];
            let n = file.read(&mut head).await.unwrap_or(0);
            head.truncate(n);

            let tail_offset = meta.len().saturating_sub(50_000);
            let tail_len = (meta.len() - tail_offset) as usize;
            let mut tail = vec![0u8; tail_len];
            if let Err(e) = file.seek(std::io::SeekFrom::Start(tail_offset)).await {
                return Ok(ToolOutput {
                    content: format!("Error seeking file: {e}"),
                    is_error: true,
                });
            }
            file.read_exact(&mut tail).await.unwrap_or_default();

            let head_str = String::from_utf8_lossy(&head);
            let tail_str = String::from_utf8_lossy(&tail);
            Ok(ToolOutput {
                content: format!("{head_str}\n... [truncated, total {} bytes]\n{tail_str}", meta.len()),
                is_error: false,
            })
        } else {
            match tokio::fs::read_to_string(path).await {
                Ok(content) => Ok(ToolOutput { content, is_error: false }),
                Err(e) => Ok(ToolOutput {
                    content: format!("Error reading file: {e}"),
                    is_error: true,
                }),
            }
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
        let max_results = 200;
        let mut pending = vec![std::path::PathBuf::from(root)];

        while !pending.is_empty() && results.len() < max_results {
            let batch: Vec<_> = pending.drain(..pending.len().min(8)).collect();
            let mut handles = Vec::new();

            for dir in batch {
                let pat = pattern.to_string();
                let max = max_results;
                handles.push(tokio::spawn(async move {
                    let mut files = Vec::new();
                    let mut subdirs = Vec::new();
                    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            let path = entry.path();
                            if files.len() >= max { break; }
                            if path.is_dir() {
                                subdirs.push(path);
                            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                if glob_match(&pat, name) {
                                    files.push(path.to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                    (files, subdirs)
                }));
            }

            for handle in handles {
                if let Ok((files, subdirs)) = handle.await {
                    let remaining = max_results.saturating_sub(results.len());
                    results.extend(files.into_iter().take(remaining));
                    pending.extend(subdirs);
                }
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

pub struct GrepTool;

#[async_trait]
impl AgentTool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "在文件中搜索文本内容，返回匹配行及行号"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "搜索模式（区分大小写子串匹配）"
                },
                "path": {
                    "type": "string",
                    "description": "搜索起始目录，默认当前目录"
                },
                "include": {
                    "type": "string",
                    "description": "文件 glob 模式，例如 *.rs"
                },
                "max_results": {
                    "type": "number",
                    "description": "最大结果数，默认 200"
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
        let include = arguments.get("include").and_then(|v| v.as_str());
        let max_results = arguments
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(200) as usize;

        let mut results = Vec::new();
        let mut pending = vec![std::path::PathBuf::from(root)];

        while !pending.is_empty() && results.len() < max_results {
            let batch: Vec<_> = pending.drain(..pending.len().min(8)).collect();
            let mut handles = Vec::new();

            for dir in batch {
                let pat = pattern.to_string();
                let glob_filter = include.map(|s| s.to_string());
                handles.push(tokio::spawn(async move {
                    let mut lines = Vec::new();
                    let mut subdirs = Vec::new();
                    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            if lines.len() >= max_results { break; }
                            let path = entry.path();
                            if path.is_dir() {
                                subdirs.push(path);
                            } else {
                                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                if let Some(ref glob) = glob_filter {
                                    if !glob_match(glob, name) {
                                        continue;
                                    }
                                }
                                // 跳过过大的文件
                                const MAX_GREP_FILE_SIZE: u64 = 10 * 1024 * 1024;
                                if let Ok(meta) = tokio::fs::metadata(&path).await {
                                    if meta.len() > MAX_GREP_FILE_SIZE {
                                        continue;
                                    }
                                }
                                // 读取文件并搜索
                                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                                    for (i, line) in content.lines().enumerate() {
                                        if line.contains(&pat) {
                                            lines.push(format!(
                                                "{}:{}: {}",
                                                path.display(),
                                                i + 1,
                                                line
                                            ));
                                            if lines.len() >= max_results { break; }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    (lines, subdirs)
                }));
            }

            for handle in handles {
                if let Ok((lines, subdirs)) = handle.await {
                    let remaining = max_results.saturating_sub(results.len());
                    results.extend(lines.into_iter().take(remaining));
                    pending.extend(subdirs);
                }
            }
        }

        if results.is_empty() {
            Ok(ToolOutput {
                content: format!("No matches for '{pattern}'"),
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

pub struct ListDirectoryTool;

#[async_trait]
impl AgentTool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "列出目录内容（树状结构，可控制深度）"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "目录路径"
                },
                "depth": {
                    "type": "number",
                    "description": "递归深度，默认 1，最大 3"
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
        let depth = arguments
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .min(3) as usize;

        let path = std::path::PathBuf::from(path);
        if !path.exists() {
            return Ok(ToolOutput {
                content: format!("Path does not exist: {}", path.display()),
                is_error: true,
            });
        }
        if !path.is_dir() {
            return Ok(ToolOutput {
                content: format!("Not a directory: {}", path.display()),
                is_error: true,
            });
        }

        let lines = collect_tree_async(&path, depth).await;
        Ok(ToolOutput {
            content: lines.join("\n"),
            is_error: false,
        })
    }
}

/// 异步迭代式目录树收集（避免 sync I/O 阻塞和 async 递归限制）
async fn collect_tree_async(dir: &std::path::Path, max_depth: usize) -> Vec<String> {
    struct WorkItem {
        path: std::path::PathBuf,
        prefix: String,
        depth: usize,
    }

    let mut lines = Vec::new();
    let mut stack = vec![WorkItem {
        path: dir.to_path_buf(),
        prefix: String::new(),
        depth: max_depth,
    }];

    while let Some(item) = stack.pop() {
        if item.depth == 0 {
            continue;
        }

        let mut entries: Vec<_> = match tokio::fs::read_dir(&item.path).await {
            Ok(r) => {
                let mut v = Vec::new();
                let mut stream = r;
                while let Ok(Some(entry)) = stream.next_entry().await {
                    let p = entry.path();
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        if !name.starts_with('.') {
                            v.push(p);
                        }
                    }
                }
                v
            }
            Err(_) => continue,
        };
        entries.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        for (i, entry_path) in entries.iter().enumerate() {
            let is_last = i == entries.len() - 1;
            let connector = if is_last { "└── " } else { "├── " };
            let child_prefix = if is_last { "    " } else { "│   " };

            let name = entry_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if entry_path.is_dir() {
                let size = dir_size_limited(entry_path, 500).await;
                let size_str = format_size(size);
                if item.depth > 1 {
                    stack.push(WorkItem {
                        path: entry_path.clone(),
                        prefix: format!("{}{}", item.prefix, child_prefix),
                        depth: item.depth - 1,
                    });
                }
                lines.push(format!("{}{}{}/{}", item.prefix, connector, name, size_str));
            } else {
                let size = tokio::fs::metadata(entry_path)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);
                let size_str = format_size(size);
                lines.push(format!("{}{}{}{}", item.prefix, connector, name, size_str));
            }
        }
    }

    lines
}

/// 扫描目录大小，最多遍历 500 个条目（防止 node_modules 等大目录 O(n²)）
async fn dir_size_limited(dir: &std::path::Path, max_entries: u64) -> u64 {
    let mut total = 0u64;
    let mut count = 0u64;
    let mut dirs = vec![dir.to_path_buf()];

    while let Some(d) = dirs.pop() {
        if count >= max_entries {
            break;
        }
        if let Ok(mut entries) = tokio::fs::read_dir(&d).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                count += 1;
                if count > max_entries {
                    break;
                }
                let p = entry.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if let Ok(meta) = tokio::fs::metadata(&p).await {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn format_size(size: u64) -> String {
    if size > 1024 * 1024 {
        format!(" ({:.1} MB)", size as f64 / (1024.0 * 1024.0))
    } else if size > 1024 {
        format!(" ({:.1} KB)", size as f64 / 1024.0)
    } else {
        format!(" ({size} B)")
    }
}

// ─── ExecuteCommandTool ─────────────────────────────────────

pub struct ExecuteCommandTool;

#[async_trait]
impl AgentTool for ExecuteCommandTool {
    fn name(&self) -> &str {
        "execute_command"
    }

    fn description(&self) -> &str {
        "执行 shell 命令并返回输出。会使用系统 shell 运行（Unix: sh -c, Windows: cmd /C）。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "要执行的命令"
                },
                "working_dir": {
                    "type": "string",
                    "description": "工作目录，默认当前目录"
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "超时时间（毫秒），默认 60000"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let command = arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing command".into()))?;
        let working_dir = arguments.get("working_dir").and_then(|v| v.as_str());
        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(60_000);

        use tokio::process::Command;
        use tokio::time::Duration;

        let mut child = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.args(["/C", command]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", command]);
            c
        };

        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        child.kill_on_drop(true);

        if let Some(dir) = working_dir {
            child.current_dir(dir);
        }

        let child = match child.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("Failed to spawn command: {e}"),
                    is_error: true,
                });
            }
        };

        let output = match tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                return Ok(ToolOutput {
                    content: format!("Failed to read command output: {e}"),
                    is_error: true,
                });
            }
            Err(_) => {
                return Ok(ToolOutput {
                    content: format!("Command timed out after {timeout_ms}ms"),
                    is_error: true,
                });
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // 截断总输出到 100KB
        const MAX_OUTPUT: usize = 100_000;
        let mut result = format!("Exit code: {exit_code}\n");
        if !stdout.is_empty() {
            let label = "\n--- stdout ---\n";
            result.push_str(label);
            let remaining = MAX_OUTPUT.saturating_sub(result.len());
            if stdout.len() > remaining {
                result.push_str(&stdout[..remaining]);
                result.push_str(&format!("\n... [truncated, total stdout {} bytes]", stdout.len()));
            } else {
                result.push_str(&stdout);
            }
        }
        if !stderr.is_empty() {
            let label = "\n--- stderr ---\n";
            let remaining = MAX_OUTPUT.saturating_sub(result.len());
            if remaining > label.len() {
                result.push_str(label);
                let remaining = MAX_OUTPUT.saturating_sub(result.len());
                if stderr.len() > remaining {
                    result.push_str(&stderr[..remaining]);
                    result.push_str(&format!("\n... [truncated, total stderr {} bytes]", stderr.len()));
                } else {
                    result.push_str(&stderr);
                }
            }
        }

        Ok(ToolOutput {
            content: result,
            is_error: !output.status.success(),
        })
    }
}

// ─── EditFileTool ────────────────────────────────────────────

pub struct EditFileTool;

#[async_trait]
impl AgentTool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "精确编辑文件：搜索 old_string 并用 new_string 替换。old_string 必须在文件中唯一匹配。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "文件路径"
                },
                "old_string": {
                    "type": "string",
                    "description": "要被替换的精确文本（必须唯一匹配）"
                },
                "new_string": {
                    "type": "string",
                    "description": "替换后的新文本"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let file_path = arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing file_path".into()))?;
        let old_string = arguments
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing old_string".into()))?;
        let new_string = arguments
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing new_string".into()))?;

        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("Error reading file: {e}"),
                    is_error: true,
                });
            }
        };

        // 单次遍历：同时计数 + 定位匹配位置
        let mut count = 0usize;
        let mut match_start = 0;
        let mut search_from = 0;
        while let Some(pos) = content[search_from..].find(old_string) {
            count += 1;
            if count == 1 {
                match_start = search_from + pos;
            }
            if count > 1 {
                break;
            }
            search_from += pos + 1;
        }

        if count == 0 {
            // 显示文件上下文帮助 LLM 调整
            let preview: String = content.chars().take(500).collect();
            let hint = if content.len() > 500 {
                format!("{preview}... [file truncated, total {} bytes]", content.len())
            } else {
                preview
            };
            return Ok(ToolOutput {
                content: format!(
                    "old_string not found in file. File has {} bytes.\nFirst 500 chars:\n{hint}",
                    content.len()
                ),
                is_error: true,
            });
        }
        if count > 1 {
            return Ok(ToolOutput {
                content: format!("old_string found {count} times — must match exactly once. Provide more context."),
                is_error: true,
            });
        }

        // 单次遍历已完成计数，用切片构造避免二次 replace 遍历
        let new_content = format!("{}{}{}", &content[..match_start], new_string, &content[match_start + old_string.len()..]);
        match tokio::fs::write(file_path, &new_content).await {
            Ok(_) => Ok(ToolOutput {
                content: format!(
                    "Successfully applied edit to {file_path} ({} chars replaced)",
                    old_string.len()
                ),
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: format!("Error writing file: {e}"),
                is_error: true,
            }),
        }
    }
}

// ─── GlobTool ────────────────────────────────────────────────

pub struct GlobTool;

#[async_trait]
impl AgentTool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "按 glob 模式搜索文件路径（匹配完整相对路径，支持 ** 递归）。例如 src/**/*.rs 搜索 src 下所有 Rust 文件。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "glob 模式，如 **/*.rs 或 src/**/mod.rs"
                },
                "root": {
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
            .get("root")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let root_path = std::path::PathBuf::from(root);
        let mut results = Vec::new();
        let max_results = 500;
        let mut pending = vec![root_path.clone()];

        while !pending.is_empty() && results.len() < max_results {
            let batch: Vec<_> = pending.drain(..pending.len().min(8)).collect();
            let mut handles = Vec::new();

            for dir in batch {
                let pat = pattern.to_string();
                let root_clone = root_path.clone();
                let max = max_results;
                handles.push(tokio::spawn(async move {
                    let mut files = Vec::new();
                    let mut subdirs = Vec::new();
                    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            if files.len() >= max { break; }
                            let path = entry.path();
                            if path.is_dir() {
                                subdirs.push(path);
                            } else if let Ok(rel) = path.strip_prefix(&root_clone) {
                                let rel_str = rel.to_string_lossy().replace('\\', "/");
                                if glob_match(&pat, &rel_str) {
                                    files.push(path.to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                    (files, subdirs)
                }));
            }

            for handle in handles {
                if let Ok((files, subdirs)) = handle.await {
                    let remaining = max_results.saturating_sub(results.len());
                    results.extend(files.into_iter().take(remaining));
                    pending.extend(subdirs);
                }
            }
        }

        if results.is_empty() {
            Ok(ToolOutput {
                content: format!("No files matching '{pattern}'"),
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

// ─── DeleteFileTool ──────────────────────────────────────────

pub struct DeleteFileTool;

#[async_trait]
impl AgentTool for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn description(&self) -> &str {
        "删除文件或空目录。recursive=true 时递归删除目录及其内容。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "要删除的文件或目录路径"
                },
                "recursive": {
                    "type": "boolean",
                    "description": "是否递归删除（用于目录），默认 false"
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
        let recursive = arguments
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let p = std::path::Path::new(path);
        if !p.exists() {
            return Ok(ToolOutput {
                content: format!("Path does not exist: {path}"),
                is_error: true,
            });
        }

        if p.is_dir() {
            if recursive {
                match tokio::fs::remove_dir_all(p).await {
                    Ok(_) => Ok(ToolOutput {
                        content: format!("Successfully deleted directory (recursive): {path}"),
                        is_error: false,
                    }),
                    Err(e) => Ok(ToolOutput {
                        content: format!("Error deleting directory: {e}"),
                        is_error: true,
                    }),
                }
            } else {
                // Try removing empty directory
                match tokio::fs::remove_dir(p).await {
                    Ok(_) => Ok(ToolOutput {
                        content: format!("Successfully deleted empty directory: {path}"),
                        is_error: false,
                    }),
                    Err(e) => Ok(ToolOutput {
                        content: format!("Directory not empty (use recursive=true): {e}"),
                        is_error: true,
                    }),
                }
            }
        } else {
            match tokio::fs::remove_file(p).await {
                Ok(_) => Ok(ToolOutput {
                    content: format!("Successfully deleted file: {path}"),
                    is_error: false,
                }),
                Err(e) => Ok(ToolOutput {
                    content: format!("Error deleting file: {e}"),
                    is_error: true,
                }),
            }
        }
    }
}

// ─── CreateDirectoryTool ─────────────────────────────────────

pub struct CreateDirectoryTool;

#[async_trait]
impl AgentTool for CreateDirectoryTool {
    fn name(&self) -> &str {
        "create_directory"
    }

    fn description(&self) -> &str {
        "递归创建目录（类似 mkdir -p）。如果目录已存在则返回成功。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "要创建的目录路径"
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

        match tokio::fs::create_dir_all(path).await {
            Ok(_) => Ok(ToolOutput {
                content: format!("Directory ready: {path}"),
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: format!("Error creating directory: {e}"),
                is_error: true,
            }),
        }
    }
}

// ─── FetchUrlTool ────────────────────────────────────────────

pub struct FetchUrlTool;

#[async_trait]
impl AgentTool for FetchUrlTool {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "HTTP GET 请求获取 URL 内容（文本）。适用于获取网页、API 响应等。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "要获取的 URL"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let url = arguments
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing url".into()))?;

        static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
        let client = CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest::Client")
        });

        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("HTTP request failed: {e}"),
                    is_error: true,
                });
            }
        };

        let status = resp.status();
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("Failed to read response body: {e}"),
                    is_error: true,
                });
            }
        };

        const MAX_BODY: usize = 1_000_000;
        let truncated = if body.len() > MAX_BODY {
            let mut b = body[..MAX_BODY].to_string();
            b.push_str(&format!(
                "\n\n... [response truncated, total {} bytes]",
                body.len()
            ));
            b
        } else {
            body
        };

        let result = if status.is_success() {
            format!("HTTP {status}\n\n{truncated}")
        } else {
            format!("HTTP {status} (error)\n\n{truncated}")
        };

        Ok(ToolOutput {
            content: result,
            is_error: !status.is_success(),
        })
    }
}

/// 注册所有内置工具到 ToolRegistry
pub fn register_all(registry: &mut super::ToolRegistry) {
    use std::sync::Arc;
    registry.register_builtin(Arc::new(ReadTextFileTool));
    registry.register_builtin(Arc::new(WriteTextFileTool));
    registry.register_builtin(Arc::new(SearchTool));
    registry.register_builtin(Arc::new(GrepTool));
    registry.register_builtin(Arc::new(ListDirectoryTool));
    registry.register_builtin(Arc::new(ExecuteCommandTool));
    registry.register_builtin(Arc::new(EditFileTool));
    registry.register_builtin(Arc::new(GlobTool));
    registry.register_builtin(Arc::new(DeleteFileTool));
    registry.register_builtin(Arc::new(CreateDirectoryTool));
    registry.register_builtin(Arc::new(FetchUrlTool));
}

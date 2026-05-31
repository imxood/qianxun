use crate::transport::AcpTransport;
use qianxun_core::tools::{AgentTool, ToolError, ToolOutput, ToolRegistry};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing;

// ─── 转发工具（替代内置文件工具，通过 ACP 向客户端请求） ──

/// 通过 ACP 双向请求转发 read_text_file
pub struct ForwardingReadFileTool {
    transport: Arc<AcpTransport>,
}

impl ForwardingReadFileTool {
    pub fn new(transport: Arc<AcpTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl AgentTool for ForwardingReadFileTool {
    fn name(&self) -> &str {
        "read_text_file"
    }

    fn description(&self) -> &str {
        "读取指定文件的内容（通过编辑器转发）"
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

        let params = serde_json::json!({ "path": path });
        match self
            .transport
            .send_request("fs/read_text_file", params, std::time::Duration::from_secs(30))
            .await
        {
            Ok(resp) => {
                if let Some(result) = resp.result {
                    if let Some(content) = result.get("content").and_then(|v| v.as_str()) {
                        return Ok(ToolOutput {
                            content: content.to_string(),
                            is_error: false,
                        });
                    }
                }
                tracing::warn!("Client returned invalid read_text_file response, falling back to local");
                fallback_read_file(path)
            }
            Err(e) => {
                tracing::warn!("ACP forward failed for read_text_file: {e}, falling back to local");
                fallback_read_file(path)
            }
        }
    }
}

fn fallback_read_file(path: &str) -> Result<ToolOutput, ToolError> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let truncated = if content.len() > 100_000 {
                let head_end = (0..=50_000).rev().find(|&i| content.is_char_boundary(i)).unwrap_or(0);
                let tail_start = (content.len() - 50_000..content.len()).find(|&i| content.is_char_boundary(i)).unwrap_or(content.len());
                let head = &content[..head_end];
                let tail = &content[tail_start..];
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

/// 通过 ACP 双向请求转发 write_text_file
pub struct ForwardingWriteFileTool {
    transport: Arc<AcpTransport>,
}

impl ForwardingWriteFileTool {
    pub fn new(transport: Arc<AcpTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl AgentTool for ForwardingWriteFileTool {
    fn name(&self) -> &str {
        "write_text_file"
    }

    fn description(&self) -> &str {
        "写入内容到指定文件（通过编辑器转发）"
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

        let params = serde_json::json!({ "path": path, "content": content });
        match self
            .transport
            .send_request("fs/write_text_file", params, std::time::Duration::from_secs(30))
            .await
        {
            Ok(resp) => {
                if resp.error.is_none() {
                    Ok(ToolOutput {
                        content: format!("Successfully wrote {} bytes to {path}", content.len()),
                        is_error: false,
                    })
                } else {
                    Ok(ToolOutput {
                        content: format!("Client rejected write: {:?}", resp.error),
                        is_error: true,
                    })
                }
            }
            Err(e) => {
                tracing::warn!("ACP forward failed for write_text_file: {e}, falling back to local");
                fallback_write_file(path, content)
            }
        }
    }
}

fn fallback_write_file(path: &str, content: &str) -> Result<ToolOutput, ToolError> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(path, content) {
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

// ─── 构建带转发工具的 ToolRegistry ─────────────────────

/// 构建 ACP 模式的 ToolRegistry，将文件工具替换为转发版本
pub fn build_acp_tool_registry(transport: Arc<AcpTransport>) -> ToolRegistry {
    let mut tools = ToolRegistry::new();

    // 先注册所有内置工具
    qianxun_core::tools::builtin::register_all(&mut tools);

    // 用转发版本覆盖文件工具
    tools.register_builtin(std::sync::Arc::new(ForwardingReadFileTool::new(
        transport.clone(),
    )));
    tools.register_builtin(std::sync::Arc::new(ForwardingWriteFileTool::new(
        transport,
    )));

    tools
}

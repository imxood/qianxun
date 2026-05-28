use crate::mcp::transport::{McpTransport, McpTransportError};
use crate::mcp::McpServerConfig;
use crate::tools::{ToolError, ToolOutput};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// MCP 服务器能力声明。
#[derive(Debug, Clone)]
pub struct McpServerCapabilities {
    pub tools: bool,
    pub resources: bool,
    pub prompts: bool,
}

/// MCP 工具元数据。
#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// MCP 客户端 —— 管理和 MCP 服务器进程的连接。
///
/// 通过 `McpTransport` 与子进程通信，提供高级工具接口。
/// 实现 `Send + Sync`，可安全用于 `Arc` 跨协程共享。
pub struct McpClient {
    transport: Arc<McpTransport>,
    server_name: String,
    capabilities: McpServerCapabilities,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
struct McpInitializeResult {
    #[serde(default)]
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Option<McpServerCapabilitiesRaw>,
    #[serde(default)]
    pub server_info: Option<McpServerInfo>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct McpServerCapabilitiesRaw {
    #[serde(default)]
    pub tools: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
struct McpServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct McpListToolsResult {
    #[serde(default)]
    pub tools: Vec<McpToolRaw>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct McpToolRaw {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct McpCallToolResult {
    #[serde(default)]
    pub content: Vec<McpToolContent>,
    #[serde(default)]
    pub is_error: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type")]
enum McpToolContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "resource")]
    Resource { resource: Value },
}

impl McpClient {
    /// 连接 MCP 服务器：生成子进程 + 初始化握手。
    pub async fn connect(config: McpServerConfig) -> anyhow::Result<Self> {
        let transport = McpTransport::spawn(&config).await?;
        let transport = Arc::new(transport);
        let server_name = config.name.clone();

        // 发送 initialize 请求
        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "qianxun",
                "version": "0.1.0"
            }
        });

        let resp = transport
            .send_request("initialize", Some(params), Duration::from_secs(10))
            .await?;

        if let Some(err) = resp.error {
            anyhow::bail!("MCP initialize failed: {err}");
        }

        let init_result: McpInitializeResult = serde_json::from_value(
            resp.result.unwrap_or(Value::Null),
        )
        .map_err(|e| anyhow::anyhow!("invalid initialize result: {e}"))?;

        let capabilities = McpServerCapabilities {
            tools: init_result.capabilities.as_ref().and_then(|c| c.tools.as_ref()).is_some(),
            resources: false,
            prompts: false,
        };

        tracing::info!(
            "[mcp:{server_name}] initialized — protocol={}, capabilities={:?}",
            init_result.protocol_version,
            capabilities,
        );

        Ok(Self {
            transport,
            server_name,
            capabilities,
        })
    }

    /// 列出服务器提供的工具。
    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        let resp = self
            .transport
            .send_request("tools/list", None, Duration::from_secs(10))
            .await?;

        if let Some(err) = resp.error {
            anyhow::bail!("MCP list_tools failed: {err}");
        }

        let result: McpListToolsResult = serde_json::from_value(
            resp.result.unwrap_or(Value::Null),
        )
        .map_err(|e| anyhow::anyhow!("invalid list_tools result: {e}"))?;

        Ok(result
            .tools
            .into_iter()
            .map(|t| McpTool {
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            })
            .collect())
    }

    /// 调用 MCP 工具，返回 ToolOutput。
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolOutput, ToolError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });

        let resp = match self
            .transport
            .send_request("tools/call", Some(params), Duration::from_secs(120))
            .await
        {
            Ok(r) => r,
            Err(McpTransportError::Timeout) => {
                return Ok(ToolOutput {
                    content: format!("[mcp:{}] tool '{name}' timed out after 120s", self.server_name),
                    is_error: true,
                });
            }
            Err(McpTransportError::ConnectionClosed) => {
                return Ok(ToolOutput {
                    content: format!(
                        "[mcp:{}] connection closed while calling '{name}'",
                        self.server_name
                    ),
                    is_error: true,
                });
            }
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!("[mcp:{}] transport error: {e}", self.server_name),
                    is_error: true,
                });
            }
        };

        if let Some(err) = resp.error {
            return Ok(ToolOutput {
                content: format!("[mcp:{}] tool '{name}' error: {err}", self.server_name),
                is_error: true,
            });
        }

        let result: McpCallToolResult = match serde_json::from_value(
            resp.result.unwrap_or(Value::Null),
        ) {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolOutput {
                    content: format!(
                        "[mcp:{}] invalid tool '{name}' response: {e}",
                        self.server_name
                    ),
                    is_error: true,
                });
            }
        };

        let text: String = result
            .content
            .into_iter()
            .filter_map(|c| match c {
                McpToolContent::Text { text } => Some(text),
                McpToolContent::Resource { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolOutput {
            content: text,
            is_error: result.is_error,
        })
    }

    /// 返回服务器名称。
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// 返回服务器能力。
    pub fn capabilities(&self) -> &McpServerCapabilities {
        &self.capabilities
    }

    /// 优雅关闭 MCP 连接。
    pub async fn shutdown(&self) {
        self.transport.shutdown().await;
    }
}

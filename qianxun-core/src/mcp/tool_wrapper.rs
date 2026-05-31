use crate::mcp::client::McpClient;
use crate::tools::{AgentTool, ToolCategory, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// 将 MCP 工具适配为 `AgentTool` trait。
///
/// 工具名格式：`{server_name}/{tool_name}`（例如 `filesystem/read_file`），
/// 通过命名空间避免与 builtin 工具冲突。
pub struct McpToolWrapper {
    pub server_name: String,
    pub tool_name: String,
    pub client: Arc<McpClient>,
    pub description: String,
    pub input_schema: Value,
}

#[async_trait]
impl AgentTool for McpToolWrapper {
    fn name(&self) -> &str {
        // 格式："{server_name}/{tool_name}"
        // 在 ToolRegistry 中以此完整名称注册
        // （实际注册时由 McpServerManager 负责，此处只提供名称格式）
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Network
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        self.client.call_tool(&self.tool_name, arguments).await
    }
}

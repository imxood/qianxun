pub mod builtin;

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

// ─── ToolError ───────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

// ─── ToolDefinition ──────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// ─── ToolOutput ──────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

// ─── AgentTool trait ─────────────────────────────────────

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError>;
}

// ─── ToolRegistry ────────────────────────────────────────

#[derive(Default)]
pub struct ToolRegistry {
    builtin: HashMap<String, Arc<dyn AgentTool>>,
    mcp_tools: HashMap<String, McpToolEntry>,
}

pub struct McpToolEntry {
    pub client_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_builtin(&mut self, tool: Arc<dyn AgentTool>) {
        self.builtin.insert(tool.name().to_string(), tool);
    }

    pub fn register_mcp_tool(&mut self, entry: McpToolEntry) {
        self.mcp_tools.insert(entry.name.clone(), entry);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = Vec::new();

        for tool in self.builtin.values() {
            defs.push(ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            });
        }

        for entry in self.mcp_tools.values() {
            defs.push(ToolDefinition {
                name: entry.name.clone(),
                description: entry.description.clone(),
                input_schema: entry.input_schema.clone(),
            });
        }

        defs
    }

    /// 异步执行工具（真正的 async，避免 block_on 在 runtime 内 panic）
    pub async fn execute_async(&self, name: &str, arguments: Value) -> Result<ToolOutput, ToolError> {
        if let Some(tool) = self.builtin.get(name) {
            tool.execute(arguments).await
        } else {
            Err(ToolError::NotFound(name.to_string()))
        }
    }

    /// 同步执行工具（通过 block_on，仅用于非 tokio 上下文）
    pub fn execute(&self, name: &str, arguments: Value) -> Result<ToolOutput, ToolError> {
        if let Some(tool) = self.builtin.get(name) {
            let rt = tokio::runtime::Handle::try_current()
                .map_err(|_| ToolError::ExecutionFailed("no tokio runtime".into()))?;

            rt.block_on(tool.execute(arguments))
        } else {
            Err(ToolError::NotFound(name.to_string()))
        }
    }

    pub fn builtin_count(&self) -> usize {
        self.builtin.len()
    }

    pub fn mcp_count(&self) -> usize {
        self.mcp_tools.len()
    }
}

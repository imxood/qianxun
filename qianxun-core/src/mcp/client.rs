use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct McpServerCapabilities {
    pub tools: bool,
    pub resources: bool,
    pub prompts: bool,
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct McpClient {}

impl McpClient {
    pub async fn connect_stdio(_config: McpServerConfig) -> anyhow::Result<Self> {
        // Phase 1: 骨架
        Ok(Self {})
    }

    pub async fn connect_http(_config: McpServerConfig) -> anyhow::Result<Self> {
        // Phase 1: 骨架
        Ok(Self {})
    }

    pub async fn initialize(&mut self) -> anyhow::Result<McpServerCapabilities> {
        Ok(McpServerCapabilities {
            tools: false,
            resources: false,
            prompts: false,
        })
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpTool>> {
        Ok(Vec::new())
    }

    pub async fn call_tool(
        &self,
        _name: &str,
        _arguments: Value,
    ) -> anyhow::Result<Value> {
        Ok(Value::Null)
    }
}

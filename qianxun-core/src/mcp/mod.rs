pub mod client;
pub mod config;
pub mod transport;
pub mod server_manager;
pub mod tool_wrapper;

use serde::Deserialize;
use std::collections::HashMap;

/// MCP 传输类型。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum McpTransportKind {
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    #[serde(rename = "http")]
    Http {
        url: String,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// MCP 服务器配置。
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl McpServerConfig {
    /// 从新格式的传输类型配置创建旧格式配置（向后兼容）。
    pub fn from_transport(name: String, transport: McpTransportKind) -> Self {
        match transport {
            McpTransportKind::Stdio { command, args, env, .. } => Self {
                name,
                command,
                args,
                env,
            },
            McpTransportKind::Http { .. } => Self {
                name,
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
            },
        }
    }

    /// 是否为 HTTP/SSE 传输。
    pub fn is_http(&self) -> bool {
        self.command.is_empty()
    }
}

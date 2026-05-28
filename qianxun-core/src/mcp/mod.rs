pub mod client;
pub mod config;
pub mod transport;

use serde::Deserialize;
use std::collections::HashMap;

/// MCP 服务器配置。
///
/// 定义如何启动一个 MCP 服务器子进程。
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

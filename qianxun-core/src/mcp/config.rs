use crate::mcp::McpServerConfig;
use std::collections::HashMap;
use std::path::Path;

/// `.claude/mcp.json` 文件顶层结构
///
/// ```json
/// {
///   "mcpServers": {
///     "my-server": {
///       "command": "npx",
///       "args": ["-y", "@anthropic/mcp-serve"],
///       "env": { "KEY": "value" }
///     }
///   }
/// }
/// ```
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfigFile {
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfigEntry>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct McpServerConfigEntry {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl McpConfigFile {
    /// 从 JSON 文件路径解析配置。
    /// 文件不存在时返回 `Ok(None)`。
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Option<Self>> {
        let path = path.as_ref();
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                anyhow::bail!("failed to read MCP config {}: {e}", path.display());
            }
        };

        if content.trim().is_empty() {
            return Ok(None);
        }

        let config: McpConfigFile = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse MCP config {}: {e}", path.display()))?;

        Ok(Some(config))
    }

    /// 在项目根目录查找 `.claude/mcp.json`。
    pub fn find_in_workspace(ws_root: &Path) -> anyhow::Result<Option<Self>> {
        let candidate_paths = [
            ws_root.join(".claude").join("mcp.json"),
            ws_root.join(".vscode").join("mcp.json"),
            ws_root.join("mcp.json"),
        ];

        for path in &candidate_paths {
            if path.exists() {
                return Self::from_file(path);
            }
        }

        Ok(None)
    }

    /// 转换为 `McpServerConfig` 列表。
    pub fn to_server_configs(self) -> Vec<McpServerConfig> {
        self.mcp_servers
            .into_iter()
            .map(|(name, entry)| McpServerConfig {
                name,
                command: entry.command,
                args: entry.args,
                env: entry.env,
            })
            .collect()
    }
}

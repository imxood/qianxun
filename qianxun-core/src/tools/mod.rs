pub mod builtin;

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ─── ToolError ───────────────────────────────────────────

/// 工具类别，用于模式驱动的权限门控。
///
/// 每个内置工具声明一个类别，Agent 模式（如 Plan-and-Execute）通过
/// ToolCategoryFilter 决定哪些工具在当前阶段可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    Read,
    Write,
    Search,
    Terminal,
    Network,
    Think,
}

/// 工具类别过滤器，表示一组允许的工具类别。
#[derive(Debug, Clone, Default)]
pub struct ToolCategoryFilter {
    allowed: std::collections::HashSet<ToolCategory>,
}

impl ToolCategoryFilter {
    /// 允许所有工具。
    pub fn all() -> Self {
        use ToolCategory::*;
        Self {
            allowed: [Read, Write, Search, Terminal, Network, Think].into(),
        }
    }

    /// 只允许读取和搜索（Plan-and-Execute 的计划阶段使用）。
    pub fn read_only() -> Self {
        use ToolCategory::*;
        Self {
            allowed: [Read, Search, Think].into(),
        }
    }

    /// 检查指定类别是否被允许。
    pub fn allows(&self, category: ToolCategory) -> bool {
        self.allowed.contains(&category)
    }

    /// 添加允许的类别。
    pub fn allow(mut self, category: ToolCategory) -> Self {
        self.allowed.insert(category);
        self
    }

    /// 判断是否允许所有类别。
    pub fn is_all(&self) -> bool {
        self.allowed.len() == 6
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    #[error("tool '{tool}' is not allowed in current mode: {mode}")]
    NotAllowedInCurrentMode { tool: String, mode: String },
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

    /// 返回此工具所属的类别，用于模式驱动的权限门控。
    /// 默认返回 `ToolCategory::Think`（无副作用）。
    fn category(&self) -> ToolCategory {
        ToolCategory::Think
    }
}

// ─── ToolRegistry ────────────────────────────────────────

/// 工具注册表，管理三层工具调度:
///
/// 1. builtin — 内置工具（如 read_text_file、grep）
/// 2. mcp — MCP 服务器注册的外部工具
/// 3. skill — 动态技能（预留）
pub struct ToolRegistry {
    builtin: HashMap<String, Arc<dyn AgentTool>>,
    mcp_tools: HashMap<String, McpToolEntry>,
    mcp_clients: Mutex<HashMap<String, Arc<crate::mcp::client::McpClient>>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            builtin: HashMap::new(),
            mcp_tools: HashMap::new(),
            mcp_clients: Mutex::new(HashMap::new()),
        }
    }
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        let clients = self.mcp_clients.lock().unwrap();
        Self {
            builtin: self.builtin.clone(),
            mcp_tools: self.mcp_tools.clone(),
            mcp_clients: Mutex::new(clients.clone()),
        }
    }
}

#[derive(Clone)]
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

    /// 获取工具类别。MCP 工具默认归为 Network，builtin 工具由各自实现决定。
    pub fn get_category(&self, name: &str) -> Option<ToolCategory> {
        if let Some(tool) = self.builtin.get(name) {
            return Some(tool.category());
        }
        if self.mcp_tools.contains_key(name) {
            return Some(ToolCategory::Network);
        }
        None
    }

    pub fn register_builtin(&mut self, tool: Arc<dyn AgentTool>) {
        self.builtin.insert(tool.name().to_string(), tool);
    }

    /// 尝试注册单个 builtin 工具，重名时返回错误而不是覆盖。
    ///
    /// 给 [`Self::register_all_builtin`] 用做"失败 fallback 不 panic"的原子单元：
    /// 构造/注册失败时跳过该工具并 warn 日志，不中断整体启动。
    pub fn try_register(&mut self, tool: Arc<dyn AgentTool>) -> Result<(), ToolError> {
        let name = tool.name().to_string();
        if self.builtin.contains_key(&name) {
            return Err(ToolError::InvalidArguments(format!(
                "duplicate builtin tool name: {name}"
            )));
        }
        self.builtin.insert(name, tool);
        Ok(())
    }

    /// MVP-0: 一次性注册所有 builtin 工具。
    ///
    /// 千寻 daemon 启动时调用一次，构造 / 注册某个工具失败时跳过 + warn，
    /// 不 panic，保证 daemon 可用（决策 D5：失败 fallback）。
    ///
    /// Returns: 成功注册的工具数（≥8：read/write/search/grep/list/exec/edit
    ///   + glob/delete/mkdir/fetch/memory-recall/memory-remember）。
    ///
    /// Note: `SkillReadTool` 需要 `Arc<SkillManager>`，由 Day 2 在
    ///   `SkillManager::load_all` 之后单独注册，不在本函数的硬编码列表内。
    pub fn register_all_builtin(&mut self) -> Result<usize, ToolError> {
        let mut count = 0;
        let mut errors: Vec<String> = Vec::new();

        // 13 个无外部依赖的 builtin 工具（SkillReadTool 单独注册）
        let candidates: Vec<(&'static str, Arc<dyn AgentTool>)> = vec![
            ("read_text_file", Arc::new(builtin::ReadTextFileTool)),
            ("write_text_file", Arc::new(builtin::WriteTextFileTool)),
            ("search", Arc::new(builtin::SearchTool)),
            ("grep", Arc::new(builtin::GrepTool)),
            ("list_directory", Arc::new(builtin::ListDirectoryTool)),
            ("execute_command", Arc::new(builtin::ExecuteCommandTool)),
            ("edit_file", Arc::new(builtin::EditFileTool)),
            ("glob", Arc::new(builtin::GlobTool)),
            ("delete_file", Arc::new(builtin::DeleteFileTool)),
            ("create_directory", Arc::new(builtin::CreateDirectoryTool)),
            ("fetch_url", Arc::new(builtin::FetchUrlTool)),
            ("memory_recall", Arc::new(builtin::MemoryRecallTool)),
            ("memory_remember", Arc::new(builtin::MemoryRememberTool)),
        ];

        for (name, tool) in candidates {
            match self.try_register(tool) {
                Ok(_) => count += 1,
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }

        if !errors.is_empty() {
            tracing::warn!(failed = ?errors, "some builtin tools failed to register");
        }
        Ok(count)
    }

    /// 返回所有已注册 builtin 工具的名字列表（按插入顺序，无序保证）。
    pub fn list_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.builtin.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn register_mcp_tool(&mut self, entry: McpToolEntry) {
        self.mcp_tools.insert(entry.name.clone(), entry);
    }

    /// 注册 MCP 客户端实例。
    pub fn register_mcp_client(&self, client: Arc<crate::mcp::client::McpClient>) {
        self.mcp_clients
            .lock()
            .unwrap()
            .insert(client.server_name().to_string(), client);
    }

    /// 移除并返回 MCP 客户端（用于关闭时清理）。
    pub fn remove_mcp_client(
        &self,
        server_name: &str,
    ) -> Option<Arc<crate::mcp::client::McpClient>> {
        self.mcp_clients.lock().unwrap().remove(server_name)
    }

    /// 返回所有注册的 MCP 客户端名称。
    pub fn mcp_client_names(&self) -> Vec<String> {
        self.mcp_clients.lock().unwrap().keys().cloned().collect()
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

    /// 异步执行工具（真正的 async，避免 block_on 在 runtime 内 panic）。
    ///
    /// 调度优先级: builtin → MCP
    pub async fn execute_async(&self, name: &str, arguments: Value) -> Result<ToolOutput, ToolError> {
        // 1. 优先检查内置工具
        if let Some(tool) = self.builtin.get(name) {
            return tool.execute(arguments).await;
        }

        // 2. 检查 MCP 工具（在 .await 前释放锁）
        if let Some(entry) = self.mcp_tools.get(name) {
            let client_opt = {
                let clients = self.mcp_clients.lock().unwrap();
                clients.get(&entry.client_id).cloned()
            };
            if let Some(client) = client_opt {
                return client.call_tool(&entry.name, arguments).await;
            }
            return Err(ToolError::ExecutionFailed(format!(
                "MCP client '{}' not available for tool '{name}'",
                entry.client_id
            )));
        }

        Err(ToolError::NotFound(name.to_string()))
    }

    /// 带权限门控的异步执行工具。
    ///
    /// 如果工具类别不被 filter 允许，返回 `ToolError::NotAllowedInCurrentMode`。
    /// 否则行为与 `execute_async()` 相同。
    pub async fn execute_async_with_filter(
        &self,
        name: &str,
        arguments: Value,
        filter: &ToolCategoryFilter,
    ) -> Result<ToolOutput, ToolError> {
        if let Some(category) = self.get_category(name) {
            if !filter.allows(category) {
                return Err(ToolError::NotAllowedInCurrentMode {
                    tool: name.to_string(),
                    mode: format!("{category:?} tools are restricted"),
                });
            }
        }
        self.execute_async(name, arguments).await
    }

    /// 同步执行工具（通过 block_on，仅用于非 tokio 上下文）。
    pub fn execute(&self, name: &str, arguments: Value) -> Result<ToolOutput, ToolError> {
        // 1. 优先检查内置工具
        if let Some(tool) = self.builtin.get(name) {
            let rt = tokio::runtime::Handle::try_current()
                .map_err(|_| ToolError::ExecutionFailed("no tokio runtime".into()))?;
            return rt.block_on(tool.execute(arguments));
        }

        // 2. 检查 MCP 工具
        if let Some(entry) = self.mcp_tools.get(name) {
            let client_opt = {
                let clients = self.mcp_clients.lock().unwrap();
                clients.get(&entry.client_id).cloned()
            };
            if let Some(client) = client_opt {
                let rt = tokio::runtime::Handle::try_current()
                    .map_err(|_| ToolError::ExecutionFailed("no tokio runtime".into()))?;
                return rt.block_on(client.call_tool(&entry.name, arguments));
            }
            return Err(ToolError::ExecutionFailed(format!(
                "MCP client '{}' not available for tool '{name}'",
                entry.client_id
            )));
        }

        Err(ToolError::NotFound(name.to_string()))
    }

    pub fn builtin_count(&self) -> usize {
        self.builtin.len()
    }

    pub fn mcp_count(&self) -> usize {
        self.mcp_tools.len()
    }

    pub fn mcp_client_count(&self) -> usize {
        self.mcp_clients.lock().unwrap().len()
    }

    /// 关闭所有连接的 MCP 客户端。
    pub async fn shutdown_all(&self) {
        let names: Vec<String> = self.mcp_client_names();
        for name in &names {
            if let Some(client) = self.remove_mcp_client(name) {
                tracing::info!("[mcp] shutting down '{name}'");
                client.shutdown().await;
            }
        }
    }

    /// 格式化工具列表（用于 CLI `/tools` 展示）。
    pub fn format_tools_list(&self) -> String {
        let mut list = String::new();

        list.push_str(&format!("内置工具 ({}):\n", self.builtin.len()));
        if self.builtin.is_empty() {
            list.push_str("  （无）\n");
        } else {
            let mut names: Vec<&str> = self.builtin.keys().map(|s| s.as_str()).collect();
            names.sort();
            for name in names {
                if let Some(tool) = self.builtin.get(name) {
                    list.push_str(&format!("  🔧 **{}** — {}\n", name, tool.description()));
                }
            }
        }

        list.push('\n');
        list.push_str(&format!("MCP 工具 ({}):\n", self.mcp_tools.len()));
        if self.mcp_tools.is_empty() {
            list.push_str("  （无）\n");
        } else {
            let mut entries: Vec<&McpToolEntry> = self.mcp_tools.values().collect();
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            for entry in entries {
                list.push_str(&format!("  🔌 **{}** [{}] — {}\n", entry.name, entry.client_id, entry.description));
            }
        }

        list
    }
}

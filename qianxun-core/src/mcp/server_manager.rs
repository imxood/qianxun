use crate::mcp::client::McpClient;
use crate::mcp::McpServerConfig;
use crate::tools::ToolRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// MCP 服务器管理器 —— 统一管理多个 MCP 服务器的生命周期。
///
/// 职责：
/// - 按配置启动/停止/重启 MCP 服务器
/// - 崩溃检测与循环保护（最多 3 次 / 5 分钟）
/// - 将 MCP 工具注册到 `ToolRegistry`
pub struct McpServerManager {
    /// 已连接的客户端（按 server_name 索引）
    clients: HashMap<String, Arc<McpClient>>,
    /// 各服务器的崩溃状态
    crash_states: HashMap<String, CrashState>,
}

/// 单个 MCP 服务器的崩溃跟踪状态。
#[derive(Debug, Clone)]
struct CrashState {
    /// 最近 crash 的时间戳
    crashes: Vec<Instant>,
}

impl CrashState {
    const MAX_CRASHES: usize = 3;
    const WINDOW: Duration = Duration::from_secs(300); // 5 分钟

    /// 记录一次 crash，返回 true 表示已超过上限。
    fn record(&mut self) -> bool {
        let now = Instant::now();
        // 移除窗口之外的旧记录
        self.crashes.retain(|t| now - *t < Self::WINDOW);
        self.crashes.push(now);
        self.crashes.len() > Self::MAX_CRASHES
    }
}

impl McpServerManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            crash_states: HashMap::new(),
        }
    }

    /// 启动一个 MCP 服务器并连接。
    pub async fn start(&mut self, config: &McpServerConfig) -> anyhow::Result<()> {
        let name = &config.name;

        // 检查是否已启动
        if self.clients.contains_key(name) {
            tracing::warn!("[mcp:{name}] already connected, skipping");
            return Ok(());
        }

        tracing::info!("[mcp:{name}] starting...");

        match McpClient::connect(config.clone()).await {
            Ok(client) => {
                let client = Arc::new(client);

                // 获取工具列表并缓存
                match client.list_tools().await {
                    Ok(tools) => {
                        tracing::info!("[mcp:{name}] connected ({} tools)", tools.len());
                    }
                    Err(e) => {
                        tracing::warn!("[mcp:{name}] connected but list_tools failed: {e}");
                    }
                }

                self.clients.insert(name.clone(), client);
                // 成功后清除崩溃记录
                self.crash_states.remove(name);
                Ok(())
            }
            Err(e) => {
                tracing::error!("[mcp:{name}] connect failed: {e}");
                Err(e)
            }
        }
    }

    /// 停止一个 MCP 服务器。
    pub async fn stop(&self, name: &str) -> anyhow::Result<()> {
        if let Some(client) = self.clients.get(name) {
            tracing::info!("[mcp:{name}] shutting down...");
            client.shutdown().await;
        }
        Ok(())
    }

    /// 重启一个 MCP 服务器（含崩溃保护）。
    pub async fn restart(&mut self, name: &str) -> anyhow::Result<()> {
        // 先停止
        let _ = self.stop(name).await;
        self.clients.remove(name);

        // 检查崩溃状态
        let state = self
            .crash_states
            .entry(name.to_string())
            .or_insert_with(|| CrashState {
                crashes: Vec::new(),
            });

        if state.record() {
            let err = anyhow::anyhow!(
                "[mcp:{name}] crashed too many times (>{}) in 5 minutes, permanently disabled",
                CrashState::MAX_CRASHES
            );
            tracing::error!("{err}");
            return Err(err);
        }

        tracing::info!("[mcp:{name}] restarting...");
        // 重新连接需要在启动流程中加载新配置
        // 此方法不持有配置信息，由调用方提供
        Ok(())
    }

    /// 从配置列表启动所有服务器。
    pub async fn start_all(&mut self, configs: &[McpServerConfig]) {
        for config in configs {
            if let Err(e) = self.start(config).await {
                tracing::error!("[mcp:{}] failed to start: {e}", config.name);
            }
        }
    }

    /// 将所有已连接 MCP 服务器的工具注册到 `ToolRegistry`。
    ///
    /// 工具名格式：`{server_name}/{tool_name}`（例如 `filesystem/read_file`）。
    pub fn register_tools(&self, registry: &mut ToolRegistry) {
        for (server_name, client) in &self.clients {
            registry.register_mcp_client(client.clone());
            // 注意：tool_wrapper 的 name() 返回的是 tool_name（无 server 前缀）
            // 为了保持命名空间隔离，这里注册时带 server 前缀
            // 但当前 ToolRegistry 的 execute_async 搜索 mcp_tools 是按完整名称查找的
            // 所以注册时不需要包装为 McpToolWrapper，直接用 McpToolEntry
            tracing::info!("[mcp:{server_name}] registered client");
        }
    }

    /// 优雅关闭所有 MCP 服务器。
    pub async fn shutdown_all(&self) {
        for (name, client) in &self.clients {
            tracing::info!("[mcp:{name}] shutting down...");
            client.shutdown().await;
        }
    }

    /// 返回当前连接的服务器数量。
    pub fn connected_count(&self) -> usize {
        self.clients.len()
    }

    /// 返回服务器名称列表。
    pub fn server_names(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_state_within_window() {
        let mut state = CrashState {
            crashes: Vec::new(),
        };
        // 3 次 crash 在窗口内 → 不应该触发熔断
        state.record();
        state.record();
        let tripped = state.record();
        assert!(!tripped, "3 crashes within window should not trip");
    }

    #[test]
    fn test_crash_state_exceeds_limit() {
        let mut state = CrashState {
            crashes: vec![
                Instant::now() - Duration::from_secs(10),
                Instant::now() - Duration::from_secs(8),
                Instant::now() - Duration::from_secs(5),
            ],
        };
        let tripped = state.record(); // 第 4 次
        assert!(tripped, "4 crashes within window should trip");
    }

    #[test]
    fn test_crash_state_expired_window() {
        let mut state = CrashState {
            crashes: vec![
                Instant::now() - Duration::from_secs(400), // 6.7 分钟前，不在窗口内
            ],
        };
        let tripped = state.record(); // 有效记录只有 1 条
        assert!(!tripped, "old crashes should be ignored");
    }
}

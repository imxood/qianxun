use crate::acp_output::AcpOutputEvent;
use crate::prompt::new_agent_loop;
use crate::session::SessionManager;
use crate::transport::AcpTransport;
use crate::types::*;
use qianxun_core::config::ResolvedCompactionConfig;
use qianxun_core::context::memory::MemoryManager;
use qianxun_core::mcp::client::McpClient;
use qianxun_core::mcp::McpServerConfig;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::{SkillManager, SkillWatcher};
use qianxun_core::tools::ToolRegistry;
use qianxun_core::types::AgentConfig;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing;

// ─── 请求处理器 ─────────────────────────────────────────

pub struct AcpRequestHandler {
    pub transport: Arc<AcpTransport>,
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub sessions: Arc<Mutex<SessionManager>>,
    pub output_tx: mpsc::UnboundedSender<AcpOutputEvent>,
    pub agent_config: AgentConfig,
    pub compact_config: Option<ResolvedCompactionConfig>,
    pub budget_input: Option<u64>,
    pub budget_output: Option<u64>,
}

impl AcpRequestHandler {
    /// 分发传入请求
    pub async fn handle_request(&self, req: JsonRpcRequest) -> anyhow::Result<()> {
        let id = req.id.clone();

        // session/prompt 使用延迟响应（等处理完成后再发 JSON-RPC 响应）
        if req.method.as_str() == "session/prompt" {
            self.handle_session_prompt_deferred(id, req.params).await;
            return Ok(());
        }

        let result = match req.method.as_str() {
            "initialize" => self.handle_initialize(req.params).await,
            "session/new" => self.handle_session_new(req.params).await,
            "session/load" => self.handle_session_load(req.params).await,
            "session/resume" => self.handle_session_resume(req.params).await,
            "session/delete" => self.handle_session_delete(req.params).await,
            "session/fork" => self.handle_session_fork(req.params).await,
            "session/close" => self.handle_session_close(req.params).await,
            "session/list" => self.handle_session_list().await,
            _ => {
                let resp = rpc_method_not_found(id.clone(), &req.method);
                self.transport.send_response(&resp).await?;
                return Ok(());
            }
        };

        match result {
            Ok(result_val) => {
                let resp = rpc_success(id, result_val);
                self.transport.send_response(&resp).await?;
            }
            Err(e) => {
                let resp = rpc_error(id, -32603, e.to_string());
                self.transport.send_response(&resp).await?;
            }
        }

        Ok(())
    }

    async fn handle_initialize(&self, params: Option<Value>) -> Result<Value, String> {
        let _params: Option<InitializeParams> = params.and_then(|p| serde_json::from_value(p).ok());
        if let Some(ref p) = _params {
            tracing::info!(
                "Client connected: {:?}",
                p.client_info.as_ref().map(|c| &c.name)
            );
        }

        Ok(serde_json::json!({
            "protocolVersion": 1,
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {},
                "mcpCapabilities": {},
                "sessionCapabilities": {
                    "resume": {},
                    "close": {}
                }
            },
            "authMethods": []
        }))
    }

    async fn handle_session_new(&self, params: Option<Value>) -> Result<Value, String> {
        let p: SessionNewParams = params
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(SessionNewParams {
                cwd: None,
                mcp_servers: None,
            });

        let session_id = uuid::Uuid::new_v4().to_string();
        let agent_loop = new_agent_loop(self.agent_config.clone(), &self.compact_config);
        let ws = p.cwd.as_ref().and_then(|cwd| {
            qianxun_core::workspace::detect_workspace(std::path::Path::new(cwd))
        });
        let global_instructions = qianxun_core::workspace::read_global_agents_md();
        let (system_prompt, skills_catalog, skill_manager): (Option<String>, String, Option<SkillManager>) = ws.as_ref().map(|w| {
            let ctx = qianxun_core::workspace::build_workspace_context(w);
            let sm = SkillManager::load_all(Some(&w.root));
            let sc = sm.build_catalog_prompt();
            (Some(qianxun_core::agent::system_prompt::build_system_prompt(&ctx, global_instructions.as_deref())), sc, Some(sm))
        }).unwrap_or((None, String::new(), None));
        let memory_manager = ws.as_ref().and_then(build_memory_manager);
        let ws_root = ws.as_ref().map(|w| w.root.clone());
        let skill_watcher = ws_root.as_ref().map(|root| SkillWatcher::new(Some(root.as_path())));

        // 构建会话级 ToolRegistry：从基础注册表克隆，添加 MCP 工具
        let mcp_servers = parse_mcp_server_configs(p.mcp_servers.as_ref());
        let session_tools = connect_mcp_servers(&mcp_servers, &self.tools, "acp params").await;

        let mut sessions = self.sessions.lock().await;
        sessions
            .create(
                session_id.clone(),
                system_prompt,
                agent_loop,
                memory_manager,
                Some(session_tools),
                skills_catalog,
                skill_manager,
                skill_watcher,
                ws_root,
            )
            .map_err(|e| e.to_string())?;

        // 应用 budget 到新创建的会话
        if let Some(s) = sessions.get(&session_id) {
            s.conversation.set_budget(self.budget_input, self.budget_output);
        }

        tracing::info!("Session created: {session_id}");

        serde_json::to_value(SessionNewResult { session_id }).map_err(|e| e.to_string())
    }

    /// 加载现有会话（如果会话不存在则创建新会话）
    async fn handle_session_load(&self, params: Option<Value>) -> Result<Value, String> {
        let p = params.ok_or_else(|| "missing params".to_string())?;
        let session_id = p["sessionId"]
            .as_str()
            .ok_or_else(|| "missing sessionId".to_string())?
            .to_string();
        let cwd = p["cwd"].as_str();

        let mut sessions = self.sessions.lock().await;
        if sessions.get(&session_id).is_some() {
            return Ok(serde_json::json!({}));
        }

        let ws = cwd.and_then(|cwd| {
            qianxun_core::workspace::detect_workspace(std::path::Path::new(cwd))
        });
        let global_instructions = qianxun_core::workspace::read_global_agents_md();
        let (system_prompt, skills_catalog, skill_manager): (Option<String>, String, Option<SkillManager>) = ws.as_ref().map(|w| {
            let ctx = qianxun_core::workspace::build_workspace_context(w);
            let sm = SkillManager::load_all(Some(&w.root));
            let sc = sm.build_catalog_prompt();
            (Some(qianxun_core::agent::system_prompt::build_system_prompt(&ctx, global_instructions.as_deref())), sc, Some(sm))
        }).unwrap_or((None, String::new(), None));
        let memory_manager = ws.as_ref().and_then(build_memory_manager);
        let ws_root = ws.as_ref().map(|w| w.root.clone());
        let skill_watcher = ws_root.as_ref().map(|root| SkillWatcher::new(Some(root.as_path())));

        let agent_loop = new_agent_loop(self.agent_config.clone(), &self.compact_config);
        let session_tools = match ws {
            Some(ref ws) => {
                match qianxun_core::mcp::config::McpConfigFile::find_in_workspace(&ws.root) {
                    Ok(Some(mcp_cfg)) => {
                        let configs = mcp_cfg.to_server_configs();
                        if configs.is_empty() {
                            None
                        } else {
                            Some(connect_mcp_servers(&configs, &self.tools, "workspace mcp.json").await)
                        }
                    }
                    _ => None,
                }
            }
            None => None,
        };
        sessions
            .create(session_id.clone(), system_prompt, agent_loop, memory_manager, session_tools, skills_catalog, skill_manager, skill_watcher, ws_root)
            .map_err(|e| e.to_string())?;

        if let Some(s) = sessions.get(&session_id) {
            s.conversation.set_budget(self.budget_input, self.budget_output);
        }

        // 尝试从磁盘恢复历史会话数据
        sessions.load_session(&session_id).await;

        Ok(serde_json::json!({}))
    }

    /// 恢复会话（同 load 逻辑，如果在同一进程内则保留上下文）
    async fn handle_session_resume(&self, params: Option<Value>) -> Result<Value, String> {
        self.handle_session_load(params).await
    }

    async fn handle_session_close(&self, params: Option<Value>) -> Result<Value, String> {
        let p: SessionCloseParams = params
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| "missing session_id".to_string())?;

        let mut sessions = self.sessions.lock().await;
        // 先持久化再关闭
        sessions.save_session(&p.session_id).await;
        if sessions.close(&p.session_id) {
            tracing::info!("Session closed: {}", p.session_id);
            Ok(serde_json::json!({}))
        } else {
            Err(format!("session not found: {}", p.session_id))
        }
    }

    async fn handle_session_fork(&self, params: Option<Value>) -> Result<Value, String> {
        let p: SessionForkParams = params
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| "missing session_id".to_string())?;

        let new_id = uuid::Uuid::new_v4().to_string();
        let mut sessions = self.sessions.lock().await;

        // fork 前先持久化源会话
        sessions.save_session(&p.session_id).await;

        sessions.fork(&new_id, &p.session_id).map_err(|e| e.to_string())?;

        // 应用 budget
        if let Some(s) = sessions.get(&new_id) {
            s.conversation.set_budget(self.budget_input, self.budget_output);
        }

        tracing::info!("Session forked: {} → {}", p.session_id, new_id);

        serde_json::to_value(SessionForkResult { session_id: new_id }).map_err(|e| e.to_string())
    }

    async fn handle_session_delete(&self, params: Option<Value>) -> Result<Value, String> {
        let p: SessionDeleteParams = params
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| "missing session_id".to_string())?;

        let mut sessions = self.sessions.lock().await;
        if sessions.delete(&p.session_id).await {
            tracing::info!("Session deleted: {}", p.session_id);
            Ok(serde_json::json!({}))
        } else {
            Err(format!("session not found: {}", p.session_id))
        }
    }

    async fn handle_session_list(&self) -> Result<Value, String> {
        let sessions = self.sessions.lock().await;
        let info = SessionListResult {
            sessions: sessions.list(),
        };
        serde_json::to_value(info).map_err(|e| e.to_string())
    }

    /// 处理 session/cancel 通知（无响应）
    pub async fn handle_cancel_notification(&self, params: Option<Value>) {
        let p: CancelParams = match params
            .and_then(|v| serde_json::from_value(v).ok())
        {
            Some(p) => p,
            None => return,
        };

        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(&p.session_id) {
            session.is_running = false;
            session.cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            tracing::info!("Session cancelled: {}", p.session_id);
        }
    }
}

/// 从工作区根路径构建 MemoryManager。
fn build_memory_manager(ws: &qianxun_core::workspace::Workspace) -> Option<MemoryManager> {
    let base_dir = qianxun_core::workspace::qianxun_dir()?.join("memory");
    Some(MemoryManager::new(base_dir, &ws.root, 5))
}

/// 解析 ACP 会话参数中的 MCP 服务器配置列表。
fn parse_mcp_server_configs(mcp_servers: Option<&Vec<Value>>) -> Vec<McpServerConfig> {
    let Some(servers) = mcp_servers else {
        return Vec::new();
    };

    servers
        .iter()
        .filter_map(|v| {
            let name = v.get("name").and_then(|n| n.as_str())?;
            let command = v.get("command").and_then(|c| c.as_str())?;
            let args: Vec<String> = v
                .get("args")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| e.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let env: std::collections::HashMap<String, String> = v
                .get("env")
                .and_then(|e| serde_json::from_value(e.clone()).ok())
                .unwrap_or_default();

            Some(McpServerConfig {
                name: name.to_string(),
                command: command.to_string(),
                args,
                env,
            })
        })
        .collect()
}

/// 连接 MCP 服务器列表，将工具和客户端注册到克隆的 ToolRegistry 中。
/// 所有连接失败仅记录警告，不阻止调用方继续。
async fn connect_mcp_servers(
    configs: &[McpServerConfig],
    base_tools: &ToolRegistry,
    source: &str,
) -> Arc<ToolRegistry> {
    if configs.is_empty() {
        return Arc::new(base_tools.clone());
    }

    let mut session_tools = base_tools.clone();
    for config in configs {
        match McpClient::connect(config.clone()).await {
            Ok(client) => {
                let server_name = client.server_name().to_string();
                match client.list_tools().await {
                    Ok(tools) => {
                        let count = tools.len();
                        for tool in tools {
                            session_tools.register_mcp_tool(
                                qianxun_core::tools::McpToolEntry {
                                    client_id: server_name.clone(),
                                    name: tool.name,
                                    description: tool.description,
                                    input_schema: tool.input_schema,
                                },
                            );
                        }
                        tracing::info!("MCP '{server_name}' connected with {count} tools ({source})");
                    }
                    Err(e) => {
                        tracing::warn!("MCP '{server_name}' list_tools failed: {e}");
                    }
                }
                session_tools.register_mcp_client(std::sync::Arc::new(client));
            }
            Err(e) => {
                tracing::warn!("MCP '{}' connect failed: {e}", config.name);
            }
        }
    }

    Arc::new(session_tools)
}

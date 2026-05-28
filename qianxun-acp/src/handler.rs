use crate::acp_output::{AcpOutputEvent, AcpOutputSink};
use crate::session::SessionManager;
use crate::transport::AcpTransport;
use crate::types::*;
use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::context::window::AutoCompactWindow;
use qianxun_core::agent::engine::processing_loop;
use qianxun_core::agent::engine::AgentLoop;
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::config::ResolvedCompactionConfig;
use qianxun_core::context::memory::MemoryManager;
use qianxun_core::mcp::client::McpClient;
use qianxun_core::mcp::McpServerConfig;
use qianxun_core::provider::LlmProvider;
use qianxun_core::skills::{SkillManager, SkillWatcher};
use qianxun_core::tools::{AgentTool, ToolError, ToolOutput, ToolRegistry};
use qianxun_core::types::AgentConfig;
use serde_json::Value;
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::path::PathBuf;
use tokio::sync::{mpsc, Mutex};
use tracing;

// ─── 转发工具（替代内置文件工具，通过 ACP 向客户端请求） ──

/// 通过 ACP 双向请求转发 read_text_file
pub struct ForwardingReadFileTool {
    transport: Arc<AcpTransport>,
}

impl ForwardingReadFileTool {
    pub fn new(transport: Arc<AcpTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl AgentTool for ForwardingReadFileTool {
    fn name(&self) -> &str {
        "read_text_file"
    }

    fn description(&self) -> &str {
        "读取指定文件的内容（通过编辑器转发）"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "文件路径"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing path".into()))?;

        let params = serde_json::json!({ "path": path });
        match self
            .transport
            .send_request("fs/read_text_file", params, std::time::Duration::from_secs(30))
            .await
        {
            Ok(resp) => {
                if let Some(result) = resp.result {
                    if let Some(content) = result.get("content").and_then(|v| v.as_str()) {
                        return Ok(ToolOutput {
                            content: content.to_string(),
                            is_error: false,
                        });
                    }
                }
                tracing::warn!("Client returned invalid read_text_file response, falling back to local");
                fallback_read_file(path)
            }
            Err(e) => {
                tracing::warn!("ACP forward failed for read_text_file: {e}, falling back to local");
                fallback_read_file(path)
            }
        }
    }
}

fn fallback_read_file(path: &str) -> Result<ToolOutput, ToolError> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let truncated = if content.len() > 100_000 {
                let head_end = (0..=50_000).rev().find(|&i| content.is_char_boundary(i)).unwrap_or(0);
                let tail_start = (content.len() - 50_000..content.len()).find(|&i| content.is_char_boundary(i)).unwrap_or(content.len());
                let head = &content[..head_end];
                let tail = &content[tail_start..];
                format!("{head}\n... [truncated, total {} bytes]\n{tail}", content.len())
            } else {
                content
            };
            Ok(ToolOutput {
                content: truncated,
                is_error: false,
            })
        }
        Err(e) => Ok(ToolOutput {
            content: format!("Error reading file: {e}"),
            is_error: true,
        }),
    }
}

/// 通过 ACP 双向请求转发 write_text_file
pub struct ForwardingWriteFileTool {
    transport: Arc<AcpTransport>,
}

impl ForwardingWriteFileTool {
    pub fn new(transport: Arc<AcpTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl AgentTool for ForwardingWriteFileTool {
    fn name(&self) -> &str {
        "write_text_file"
    }

    fn description(&self) -> &str {
        "写入内容到指定文件（通过编辑器转发）"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing path".into()))?;
        let content = arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("missing content".into()))?;

        let params = serde_json::json!({ "path": path, "content": content });
        match self
            .transport
            .send_request("fs/write_text_file", params, std::time::Duration::from_secs(30))
            .await
        {
            Ok(resp) => {
                if resp.error.is_none() {
                    Ok(ToolOutput {
                        content: format!("Successfully wrote {} bytes to {path}", content.len()),
                        is_error: false,
                    })
                } else {
                    Ok(ToolOutput {
                        content: format!("Client rejected write: {:?}", resp.error),
                        is_error: true,
                    })
                }
            }
            Err(e) => {
                tracing::warn!("ACP forward failed for write_text_file: {e}, falling back to local");
                fallback_write_file(path, content)
            }
        }
    }
}

fn fallback_write_file(path: &str, content: &str) -> Result<ToolOutput, ToolError> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(path, content) {
        Ok(_) => Ok(ToolOutput {
            content: format!("Successfully wrote {} bytes to {path}", content.len()),
            is_error: false,
        }),
        Err(e) => Ok(ToolOutput {
            content: format!("Error writing file: {e}"),
            is_error: true,
        }),
    }
}

// ─── 构建带转发工具的 ToolRegistry ─────────────────────

/// 构建 ACP 模式的 ToolRegistry，将文件工具替换为转发版本
pub fn build_acp_tool_registry(transport: Arc<AcpTransport>) -> ToolRegistry {
    let mut tools = ToolRegistry::new();

    // 先注册所有内置工具
    qianxun_core::tools::builtin::register_all(&mut tools);

    // 用转发版本覆盖文件工具
    tools.register_builtin(std::sync::Arc::new(ForwardingReadFileTool::new(
        transport.clone(),
    )));
    tools.register_builtin(std::sync::Arc::new(ForwardingWriteFileTool::new(
        transport,
    )));

    tools
}

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

/// 创建 AgentLoop 并初始化上下文压缩窗口（如有配置）。
fn new_agent_loop(agent_config: AgentConfig, compact_config: &Option<ResolvedCompactionConfig>) -> AgentLoop {
    let mut agent_loop = AgentLoop::new(agent_config);
    if let Some(cc) = compact_config {
        agent_loop.compact_config = Some(cc.clone());
        agent_loop.compact_window = Some(AutoCompactWindow::new(
            cc.model_window,
            cc.max_output_tokens,
            cc.circuit_breaker_limit,
        ));
    }
    agent_loop
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
            "session/delete" | "session/fork" => {
                Err("not implemented".to_string())
            }
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

    /// session/prompt 的延迟处理入口：先校验参数，然后异步执行
    async fn handle_session_prompt_deferred(&self, id: RequestId, params: Option<Value>) {
        match self.prepare_prompt(params).await {
            Ok((session_id, agent_loop, conversation, prompt_text, memory_context, memory_manager, tools, skills_catalog, skill_injections, cancel_flag, skill_manager)) => {
                self.run_prompt_task(id, session_id, agent_loop, conversation, prompt_text, memory_context, memory_manager, tools, skills_catalog, skill_injections, cancel_flag, skill_manager)
                    .await;
            }
            Err(e) => {
                let resp = rpc_error(id, -32603, e);
                let _ = self.transport.send_response(&resp).await;
            }
        }
    }

    /// 准备 prompt 执行的参数（校验 + 提取 session）
    #[allow(clippy::type_complexity)]
    async fn prepare_prompt(
        &self,
        params: Option<Value>,
    ) -> Result<(String, AgentLoop, Conversation, String, String, Option<MemoryManager>, Arc<ToolRegistry>, String, String, Arc<AtomicBool>, Option<SkillManager>), String> {
        let p: PromptParams = params
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| "invalid prompt params".to_string())?;

        let session_id = p.session_id.clone();
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get(&session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?;

        if session.is_running {
            return Err("session already running".to_string());
        }
        session.is_running = true;

        // 提取记忆上下文
        let memory_context = session
            .memory_manager
            .as_ref()
            .map(|mm| mm.build_context())
            .unwrap_or_default();
        let memory_manager = std::mem::take(&mut session.memory_manager);

        // 提取会话级工具注册表（如有），否则使用基础注册表
        let session_tools = session
            .tools
            .take()
            .unwrap_or_else(|| self.tools.clone());

        let empty_loop = new_agent_loop(self.agent_config.clone(), &self.compact_config);
        let empty_conv = Conversation::new(None);
        let agent_loop = std::mem::replace(&mut session.agent_loop, empty_loop);
        let conversation = std::mem::replace(&mut session.conversation, empty_conv);
        let cancel_flag = session.cancel_flag.clone();

        // 检查技能文件变更（在取出 skill_manager 之前）
        if let Some(ref mut watcher) = session.skill_watcher {
            if watcher.has_changed() {
                tracing::info!("[skill_watcher] file change detected (prepare_prompt), reloading skills");
                if let Some(ref mut sm) = session.skill_manager {
                    sm.reload(session.ws_root.as_deref());
                    session.skills_catalog = sm.build_catalog_prompt();
                }
            }
        }

        let skills_catalog = session.skills_catalog.clone();
        let skill_manager = session.skill_manager.take();

        drop(sessions);

        let user_text: String = p
            .prompt
            .iter()
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        // 构建技能注入内容（Layer 2 — 自动匹配 + 手动引用）
        // 使用会话缓存的 skill_manager，避免每个 prompt 从磁盘重载
        let loaded_fallback;
        let skills_mgr: &SkillManager = match skill_manager.as_ref() {
            Some(sm) => sm,
            None => {
                loaded_fallback = SkillManager::load_all(None::<&std::path::Path>);
                &loaded_fallback
            }
        };
        let skill_injections = {
            let user_text_for_match = if user_text.is_empty() { "..." } else { &user_text };

            // 手动引用 @技能名
            let manual_names: Vec<String> = SkillManager::extract_manual_mentions(user_text_for_match)
                .into_iter()
                .filter(|name| skills_mgr.select_by_name(name).is_some())
                .collect();

            // 自动匹配
            let exclude: Vec<&str> = manual_names.iter().map(|s| s.as_str()).collect();
            let auto_names = skills_mgr.auto_select(user_text_for_match, &exclude);

            let mut inject_names = manual_names;
            inject_names.extend(auto_names);

            skills_mgr.build_injections(&inject_names)
        };

        Ok((session_id, agent_loop, conversation, user_text, memory_context, memory_manager, session_tools, skills_catalog, skill_injections, cancel_flag, skill_manager))
    }

    /// 在后台任务中执行 prompt，完成后通过 output_tx 发送 JSON-RPC 响应
    #[allow(clippy::too_many_arguments)]
    async fn run_prompt_task(
        &self,
        id: RequestId,
        session_id: String,
        mut agent_loop: AgentLoop,
        mut conversation: Conversation,
        prompt_text: String,
        memory_context: String,
        memory_manager: Option<MemoryManager>,
        tools: Arc<ToolRegistry>,
        skills_catalog: String,
        skill_injections: String,
        cancel_flag: Arc<AtomicBool>,
        skill_manager: Option<SkillManager>,
    ) {
        conversation
            .push_user_message(vec![ContentBlock::text(&prompt_text)]);

        let sink = AcpOutputSink::new(session_id.clone(), self.output_tx.clone());
        let provider: Arc<dyn LlmProvider> = self.provider.clone();
        let tools_for_spawn = tools.clone();
        let skills_catalog_for_spawn = skills_catalog;
        let skill_injections_for_spawn = skill_injections;
        let sid = session_id.clone();

        // 第一层：处理循环（可能 panic）
        let processing_handle = tokio::spawn(async move {
            processing_loop::handle_user_message(
                &mut agent_loop,
                &mut conversation,
                provider.as_ref(),
                tools_for_spawn.as_ref(),
                &sink,
                &memory_context,
                &skills_catalog_for_spawn,
                &skill_injections_for_spawn,
                cancel_flag,
            )
            .await;
            (agent_loop, conversation) // 正常完成后返还所有权
        });

        // 第二层：监护任务，确保响应一定发送
        let output_tx = self.output_tx.clone();
        let sessions_arc2 = self.sessions.clone();
        tokio::spawn(async move {
            match processing_handle.await {
                Ok((agent_loop, conversation)) => {
                    // 处理正常完成，保存会话状态
                    // 写入记忆
                    let summary = if prompt_text.len() > 200 {
                        let end = (0..=200).rev().find(|&i| prompt_text.is_char_boundary(i)).unwrap_or(0);
                        &prompt_text[..end]
                    } else {
                        &prompt_text
                    };
                    if let Some(mm) = &memory_manager {
                        mm.write_memory(summary, &["conversation"], &prompt_text);
                    }

                    let mut sessions = sessions_arc2.lock().await;
                    if let Some(s) = sessions.get(&sid) {
                        drop(std::mem::replace(&mut s.conversation, conversation));
                        drop(std::mem::replace(&mut s.agent_loop, agent_loop));
                        s.memory_manager = memory_manager;
                        s.tools = Some(tools);
                        s.skill_manager = skill_manager;
                        s.is_running = false;
                    }
                    // 持久化到磁盘
                    sessions.save_session(&sid).await;
                }
                Err(panic_err) => {
                    // 处理过程 panic，标记会话为未运行
                    tracing::error!("Processing task panicked: {:?}", panic_err);
                    let mut sessions = sessions_arc2.lock().await;
                    if let Some(s) = sessions.get(&sid) {
                        s.memory_manager = memory_manager;
                        s.skill_manager = skill_manager;
                        s.is_running = false;
                    }
                }
            }

            // ★ 关键：无论处理成功还是失败，都必须发送响应
            let _ = output_tx.send(AcpOutputEvent::PromptResponse {
                id,
                stop_reason: "end_turn".to_string(),
            });
        });
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
    let home = if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE").ok()
    } else {
        std::env::var("HOME").ok()
    }?;
    let base_dir = PathBuf::from(home).join(".qianxun").join("memory");
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

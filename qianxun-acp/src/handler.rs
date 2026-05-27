use crate::acp_output::{AcpOutputEvent, AcpOutputSink};
use crate::session::SessionManager;
use crate::transport::AcpTransport;
use crate::types::*;
use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::processing_loop;
use qianxun_core::agent::engine::AgentLoop;
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::provider::LlmProvider;
use qianxun_core::tools::{AgentTool, ToolError, ToolOutput, ToolRegistry};
use qianxun_core::types::AgentConfig;
use serde_json::Value;
use async_trait::async_trait;
use std::sync::Arc;
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
                let head = &content[..50_000];
                let tail = &content[content.len() - 50_000..];
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
            Ok((session_id, agent_loop, conversation, prompt_text)) => {
                self.run_prompt_task(id, session_id, agent_loop, conversation, prompt_text)
                    .await;
            }
            Err(e) => {
                let resp = rpc_error(id, -32603, e);
                let _ = self.transport.send_response(&resp).await;
            }
        }
    }

    /// 准备 prompt 执行的参数（校验 + 提取 session）
    async fn prepare_prompt(
        &self,
        params: Option<Value>,
    ) -> Result<(String, AgentLoop, Conversation, String), String> {
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

        let empty_loop = AgentLoop::new(self.agent_config.clone());
        let empty_conv = Conversation::new(None);
        let agent_loop = std::mem::replace(&mut session.agent_loop, empty_loop);
        let conversation = std::mem::replace(&mut session.conversation, empty_conv);

        drop(sessions);

        let user_text: String = p
            .prompt
            .iter()
            .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        let user_text = if user_text.is_empty() { "..." } else { &user_text };

        Ok((session_id, agent_loop, conversation, user_text.to_string()))
    }

    /// 在后台任务中执行 prompt，完成后通过 output_tx 发送 JSON-RPC 响应
    async fn run_prompt_task(
        &self,
        id: RequestId,
        session_id: String,
        mut agent_loop: AgentLoop,
        mut conversation: Conversation,
        prompt_text: String,
    ) {
        conversation
            .push_user_message(vec![ContentBlock::text(&prompt_text)]);

        let sink = AcpOutputSink::new(session_id.clone(), self.output_tx.clone());
        let provider: Arc<dyn LlmProvider> = self.provider.clone();
        let tools = self.tools.clone();
        let sid = session_id.clone();

        // 第一层：处理循环（可能 panic）
        let processing_handle = tokio::spawn(async move {
            processing_loop::handle_user_message(
                &mut agent_loop,
                &mut conversation,
                provider.as_ref(),
                tools.as_ref(),
                &sink,
                "",
                "",
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
                    let mut sessions = sessions_arc2.lock().await;
                    if let Some(s) = sessions.get(&sid) {
                        drop(std::mem::replace(&mut s.conversation, conversation));
                        drop(std::mem::replace(&mut s.agent_loop, agent_loop));
                        s.is_running = false;
                    }
                }
                Err(panic_err) => {
                    // 处理过程 panic，标记会话为未运行
                    tracing::error!("Processing task panicked: {:?}", panic_err);
                    let mut sessions = sessions_arc2.lock().await;
                    if let Some(s) = sessions.get(&sid) {
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
        let agent_loop = AgentLoop::new(self.agent_config.clone());

        // 从 cwd 检测工作区并构建系统提示词
        let system_prompt = p.cwd.as_ref().and_then(|cwd| {
            let path = std::path::Path::new(cwd);
            let ws = qianxun_core::workspace::detect_workspace(path);
            ws.as_ref().map(|w| {
                let ctx = qianxun_core::workspace::build_workspace_context(w);
                qianxun_core::agent::system_prompt::build_system_prompt(&ctx, "", None)
            })
        });

        let mut sessions = self.sessions.lock().await;
        sessions
            .create(session_id.clone(), system_prompt, agent_loop)
            .map_err(|e| e.to_string())?;

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

        let system_prompt = cwd.and_then(|cwd| {
            let path = std::path::Path::new(cwd);
            let ws = qianxun_core::workspace::detect_workspace(path);
            ws.as_ref().map(|w| {
                let ctx = qianxun_core::workspace::build_workspace_context(w);
                qianxun_core::agent::system_prompt::build_system_prompt(&ctx, "", None)
            })
        });

        let agent_loop = AgentLoop::new(self.agent_config.clone());
        sessions
            .create(session_id, system_prompt, agent_loop)
            .map_err(|e| e.to_string())?;

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
            tracing::info!("Session cancelled: {}", p.session_id);
        }
    }
}

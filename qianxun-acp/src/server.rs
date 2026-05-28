use crate::acp_output::AcpOutputEvent;
use crate::handler::{build_acp_tool_registry, AcpRequestHandler};
use crate::session::SessionManager;
use crate::transport::AcpTransport;
use crate::types::{rpc_success, IncomingMessage};
use qianxun_core::provider::LlmProvider;
use qianxun_core::config::ResolvedCompactionConfig;
use qianxun_core::types::AgentConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing;

/// 运行 ACP 服务器主循环
///
/// 通过 stdio JSON-RPC 2.0 与编辑器通信。
/// 生命周期: initialize → sessions/new → session/prompt (流式) → sessions/close
pub async fn run_acp_server(
    provider: Box<dyn LlmProvider>,
    agent_config: AgentConfig,
    compact_config: Option<ResolvedCompactionConfig>,
    budget_input: Option<u64>,
    budget_output: Option<u64>,
) -> anyhow::Result<()> {
    let (transport, mut inbox) = AcpTransport::new();
    let transport = Arc::new(transport);
    let provider: Arc<dyn LlmProvider> = Arc::from(provider);

    // 会话持久化目录: ~/.qianxun/sessions/
    let sessions = {
        let dir = home_dir().map(|h| h.join(".qianxun").join("sessions"));
        Arc::new(Mutex::new(match dir {
            Some(d) => SessionManager::new_with_dir(10, d),
            None => SessionManager::new(10),
        }))
    };
    let tools = Arc::new(build_acp_tool_registry(transport.clone()));
    let (output_tx, mut output_rx) = mpsc::unbounded_channel();

    let handler = AcpRequestHandler {
        transport: transport.clone(),
        provider,
        tools,
        sessions,
        output_tx,
        agent_config,
        compact_config,
        budget_input,
        budget_output,
    };

    tracing::info!("ACP server started, waiting for messages...");

    loop {
        tokio::select! {
            // 处理传入的 RPC 消息
            msg = inbox.recv() => {
                match msg {
                    Some(IncomingMessage::Request(req)) => {
                        tracing::debug!("Received request: {}", req.method);
                        if let Err(e) = handler.handle_request(req).await {
                            tracing::error!("Error handling request: {e}");
                        }
                    }
                    Some(IncomingMessage::Response(_)) => {
                        // 响应由 transport 的 pending 路由处理
                    }
                    Some(IncomingMessage::Notification(notif)) => {
                        match notif.method.as_str() {
                            "session/cancel" => {
                                handler.handle_cancel_notification(notif.params).await;
                            }
                            _ => {
                                tracing::debug!("Unhandled notification: {}", notif.method);
                            }
                        }
                    }
                    None => {
                        tracing::info!("stdin closed, shutting down");
                        break;
                    }
                }
            }

            // 处理来自 AcpOutputSink 的输出事件
            Some(event) = output_rx.recv() => {
                match event {
                    AcpOutputEvent::Notification(value) => {
                        if let Err(e) = transport.send_raw(&value).await {
                            tracing::error!("Error sending notification: {e}");
                        }
                    }
                    AcpOutputEvent::ToolCall(..) => {
                        // Tool call 事件通过 processing_loop 自动处理
                        // 此分支用于额外元数据日志
                    }
                    AcpOutputEvent::PromptResponse { id, stop_reason } => {
                        let result = serde_json::json!({ "stopReason": stop_reason });
                        let resp = rpc_success(id, result);
                        if let Err(e) = transport.send_response(&resp).await {
                            tracing::error!("Error sending prompt response: {e}");
                        }
                    }
                }
            }

            else => break,
        }
    }

    tracing::info!("ACP server shutdown");
    Ok(())
}

/// 用户 home 目录
fn home_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    } else {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

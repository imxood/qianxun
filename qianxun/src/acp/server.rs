use crate::acp::output::AcpOutputEvent;
use crate::acp::forwarding_tools::build_acp_tool_registry;
use crate::acp::handler::AcpRequestHandler;
use crate::acp::session::SessionManager;
use crate::acp::transport::AcpTransport;
use crate::acp::types::{rpc_success, IncomingMessage};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing;

/// Phase 4a 收尾: ACP 入口走 qianxun-runtime 统一 RuntimeState.
///
/// 跟 desktop/daemon/TUI 共享同一份 RuntimeState 初始化逻辑 (单点维护).
/// RuntimeState 提供 provider / tools / memory / skills / config 全部统一访问.
///
/// ACP forwarding tools (文件读/写转 RPC) 仍独立持有 — 它们依赖 transport
/// 把请求发到编辑器, 不在 state.tools (state.tools 走 builtin + workspace MCP).
/// 跟原行为等价: forwarding tools 是 ACP 会话级 MCP 工具的 base registry.
pub async fn run_acp_server(
    state: Arc<qianxun_runtime::RuntimeState>,
) -> anyhow::Result<()> {
    let (transport, mut inbox) = AcpTransport::new();
    let transport = Arc::new(transport);

    // 会话持久化目录: ~/.qianxun/sessions/
    let sessions = {
        let dir = qianxun_core::workspace::home_dir().map(|h| h.join(".qianxun").join("sessions"));
        Arc::new(Mutex::new(match dir {
            Some(d) => SessionManager::new_with_dir(10, d),
            None => SessionManager::new(10),
        }))
    };
    // ACP forwarding 工具: 文件读/写通过 transport 转发到编辑器 (走 RPC).
    // 跟原行为一致: forwarding tools 是 ACP base registry, session MCP 工具叠加其上.
    let forwarding_tools = Arc::new(build_acp_tool_registry(transport.clone()));
    let (output_tx, mut output_rx) = mpsc::unbounded_channel();

    let handler = AcpRequestHandler {
        transport: transport.clone(),
        state: state.clone(),
        forwarding_tools,
        sessions,
        output_tx,
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


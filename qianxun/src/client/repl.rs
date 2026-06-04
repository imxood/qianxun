
// ─── 薄客户端 REPL (CLI 入口用) ──────────────────────────────

use std::io::{self, Write};

use anyhow;
use futures::stream::StreamExt;
use tracing::{info, warn};

use super::daemon_client::DaemonClient;
use super::types::{ClientError, PromptRequest, SseEvent};
use super::SseStream;

/// 薄客户端 REPL: 连接 daemon, 创建 session, 循环读 stdin 发送 prompt, 打印 SSE 流.
///
/// 替换 `qianxun/src/main.rs` 中旧 `run_thin_client` (那段只读 response.text,
/// 没解析 SSE, 不能流式输出).
///
/// # Stage 6b 鉴权
///
/// `token` 是 `Some(jwt)` 时构造 [`DaemonClient::with_token`], 后续所有请求
/// 自动带 `Authorization: Bearer <jwt>`; `None` 时走 [`DaemonClient::new`]
/// (向后兼容 Stage 5 旧测试, daemon 端会 401 拒受保护端点).
pub async fn run_thin_repl(daemon_url: &str, token: Option<&str>) -> anyhow::Result<()> {
    let client = match token {
        Some(t) => {
            tracing::info!("[client] thin client 携带 Bearer token ({} bytes)", t.len());
            DaemonClient::with_token(daemon_url.to_string(), t.to_string())
        }
        None => {
            tracing::warn!(
                "[client] thin client 未携带 token; 受保护端点会被 daemon 401"
            );
            DaemonClient::new(daemon_url.to_string())
        }
    };
    let health = client.health().await.map_err(|e| {
        anyhow::anyhow!("无法连接 Daemon {daemon_url}: {e}")
    })?;
    if health.status != "ok" {
        anyhow::bail!("Daemon unhealthy: {health:?}");
    }
    tracing::info!("Daemon 已连接: {daemon_url}");
    println!("已连接到 Daemon: {daemon_url}");
    println!("输入消息后按 Enter 发送（输入 /quit /exit 退出, /cancel 取消当前 prompt）\n");

    let session = client.create_session().await?;
    let session_id = session.session_id;
    println!("[session] {session_id}");

    let mut input = String::new();
    loop {
        input.clear();
        if std::io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let input = input.trim();
        match input {
            "/quit" | "/exit" => break,
            "/cancel" => {
                if let Err(e) = client.cancel(&session_id).await {
                    eprintln!("[cancel] error: {e}");
                } else {
                    println!("[cancelled]");
                }
                continue;
            }
            "/sessions" => match client.list_sessions().await {
                Ok(sessions) => {
                    for s in sessions {
                        println!("- {} ({})", s.session_id, s.status);
                    }
                }
                Err(e) => eprintln!("[sessions] error: {e}"),
            },
            "" => continue,
            _ => {}
        }

        let req = PromptRequest::text(input);
        match client.stream_prompt(&session_id, &req).await {
            Ok(stream) => {
                consume_sse_stream_print(stream).await;
            }
            Err(e) => eprintln!("[prompt] error: {e}"),
        }
    }
    Ok(())
}

/// 消费 SSE 事件流, 打印 text_delta (实时), 打印 usage/message_stop 摘要.
async fn consume_sse_stream_print(stream: SseStream) {
    tokio::pin!(stream);
    while let Some(item) = stream.next().await {
        match item {
            Ok(SseEvent::TextDelta { text, .. }) => {
                print!("{text}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            Ok(SseEvent::ThinkingDelta { text, .. }) => {
                eprint!("[thinking] {text}");
            }
            Ok(SseEvent::ToolUseComplete { name, id, .. }) => {
                println!("\n[tool_call] {name} (id={id})");
            }
            Ok(SseEvent::ToolResult { tool_use_id, content, is_error, .. }) => {
                let label = if is_error { "[tool_error]" } else { "[tool_result]" };
                println!("{label} {tool_use_id}: {content}");
            }
            Ok(SseEvent::Usage { input_tokens, output_tokens, .. }) => {
                eprintln!("\n[usage] in={input_tokens} out={output_tokens}");
            }
            Ok(SseEvent::MessageDelta { stop_reason }) => {
                eprintln!("[stop_reason] {stop_reason}");
            }
            Ok(SseEvent::MessageStop) => {
                println!();
            }
            Ok(SseEvent::Error { code, message }) => {
                eprintln!("\n[error {code}] {message}");
            }
            Ok(_) => {} // ContentBlockStart/Stop 等 UI 噪音, 默认静默
            Err(e) => eprintln!("\n[sse_error] {e}"),
        }
    }
}


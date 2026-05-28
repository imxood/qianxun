use crate::mcp::McpServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};
use tracing;

pub type RequestId = u64;

// ─── JSON-RPC 2.0 信封 ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpJsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpJsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<McpJsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpJsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

impl std::fmt::Display for McpJsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MCP error {}: {}", self.code, self.message)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct McpJsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

// ─── McpTransport ───────────────────────────────────────────

/// 子进程 JSON-RPC 2.0 传输层。
///
/// 管理 MCP 子进程的生命周期和双向通信。
/// 支持 Send + Sync，可安全用于 Arc 跨协程共享。
pub struct McpTransport {
    /// 子进程句柄（Drop 时 kill_on_drop）
    child: Mutex<Option<Child>>,
    /// stdin 写入缓冲
    stdin: Mutex<BufWriter<ChildStdin>>,
    /// 挂起的请求路由：id → oneshot::Sender
    pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<McpJsonRpcResponse>>>>,
    /// 单调递增请求 ID
    next_id: AtomicU64,
    /// 服务器名称（用于日志）
    pub server_name: String,
}

impl McpTransport {
    /// 生成子进程并建立 JSON-RPC 传输通道。
    pub async fn spawn(config: &McpServerConfig) -> anyhow::Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);
        cmd.env_clear();
        cmd.envs(&config.env);
        cmd.kill_on_drop(true);

        // 安全地添加 PATH
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }

        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn MCP server '{}': {e}", config.name))?;

        let stdin = child.stdin.take().ok_or_else(|| {
            anyhow::anyhow!("Failed to open stdin for MCP server '{}'", config.name)
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            anyhow::anyhow!("Failed to open stdout for MCP server '{}'", config.name)
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            anyhow::anyhow!("Failed to open stderr for MCP server '{}'", config.name)
        })?;

        let pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<McpJsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let server_name = config.name.clone();

        // 后台 stdout 读取器
        let reader_pending = pending.clone();
        let reader_name = server_name.clone();
        tokio::spawn(async move {
            Self::stdout_reader(stdout, reader_pending, &reader_name).await;
        });

        // 后台 stderr 读取器
        let stderr_name = server_name.clone();
        tokio::spawn(async move {
            Self::stderr_reader(stderr, &stderr_name).await;
        });

        Ok(Self {
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(BufWriter::new(stdin)),
            pending,
            next_id: AtomicU64::new(1),
            server_name,
        })
    }

    /// 发送 JSON-RPC 请求，等待响应（带超时）。
    pub async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<McpJsonRpcResponse, McpTransportError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let request = McpJsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        };

        let line = serde_json::to_string(&request)
            .map_err(|e| McpTransportError::Serialize(e.to_string()))?;

        // 发送请求
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await.map_err(|e| {
                McpTransportError::Io(format!("stdin write failed: {e}"))
            })?;
            stdin.write_all(b"\n").await.map_err(|e| {
                McpTransportError::Io(format!("stdin write newline failed: {e}"))
            })?;
            stdin.flush().await.map_err(|e| {
                McpTransportError::Io(format!("stdin flush failed: {e}"))
            })?;
        }

        tracing::debug!("[mcp:{}] → {method} (id={id})", self.server_name);

        // 注册挂起的请求
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // 等待响应或超时
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => {
                tracing::debug!("[mcp:{}] ← {method} (id={id})", self.server_name);
                Ok(response)
            }
            Ok(Err(_recv_err)) => {
                // oneshot 发送端被丢弃（读取器检测到 EOF 后清理）
                tracing::warn!("[mcp:{}] channel closed for {method} (id={id})", self.server_name);
                Err(McpTransportError::ConnectionClosed)
            }
            Err(_elapsed) => {
                // 超时，清理挂起的请求
                {
                    let mut pending = self.pending.lock().await;
                    pending.remove(&id);
                }
                tracing::warn!("[mcp:{}] timeout for {method} (id={id}, timeout={timeout:?})", self.server_name);
                Err(McpTransportError::Timeout)
            }
        }
    }

    /// 发送通知（无需响应）。
    pub async fn send_notification(&self, method: &str, params: Option<Value>) {
        let notification = McpJsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        };

        if let Ok(line) = serde_json::to_string(&notification) {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.write_all(line.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
        }
    }

    /// 关闭传输：发送退出通知，kill 子进程。
    pub async fn shutdown(&self) {
        tracing::info!("[mcp:{}] shutting down", self.server_name);

        // 尝试优雅关闭
        self.send_notification("shutdown", None).await;

        // 等待短暂时间后 kill
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut child = self.child.lock().await;
        if let Some(ref mut c) = *child {
            let _ = c.kill().await;
            let _ = c.wait().await;
        }
        *child = None;

        // 清理所有挂起的请求
        let mut pending = self.pending.lock().await;
        pending.clear();
    }

    /// 后台 stdout 读取器：读取子进程输出的 JSON-RPC 响应行。
    async fn stdout_reader(
        stdout: ChildStdout,
        pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<McpJsonRpcResponse>>>>,
        server_name: &str,
    ) {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::with_capacity(4096);
        const MAX_LINE_LEN: usize = 65536;

        loop {
            buf.clear();
            let n = match reader.read_until(b'\n', &mut buf).await {
                Ok(0) => break, // EOF
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("[mcp:{server_name}] stdout read error: {e}");
                    break;
                }
            };

            // read_until includes the delimiter; strip trailing newline
            let line = if buf.ends_with(b"\n") {
                &buf[..n - 1]
            } else if n >= 2 && buf.ends_with(b"\r\n") {
                &buf[..n - 2]
            } else {
                &buf[..n]
            };

            // 跳过超长行
            if line.len() > MAX_LINE_LEN {
                tracing::warn!("[mcp:{server_name}] skipping long line ({} bytes)", line.len());
                continue;
            }

            let line_str = match std::str::from_utf8(line) {
                Ok(s) => s,
                Err(_) => {
                    tracing::warn!("[mcp:{server_name}] skipping non-UTF-8 line ({} bytes)", line.len());
                    continue;
                }
            };

            if line_str.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<McpJsonRpcResponse>(line_str) {
                Ok(response) => {
                    let mut pending = pending.lock().await;
                    if let Some(tx) = pending.remove(&response.id) {
                        let _ = tx.send(response);
                    } else {
                        tracing::debug!(
                            "[mcp:{server_name}] orphan response id={}",
                            response.id
                        );
                    }
                }
                Err(e) => {
                    // 可能是通知消息，尝试解析为通用 JSON
                    if let Ok(val) = serde_json::from_str::<Value>(line_str) {
                        if val.get("method").is_some() {
                            tracing::debug!(
                                "[mcp:{server_name}] notification: {}",
                                &line_str.chars().take(200).collect::<String>()
                            );
                        } else {
                            tracing::warn!(
                                "[mcp:{server_name}] unparseable response: {e} — line: {}",
                                &line_str.chars().take(100).collect::<String>()
                            );
                        }
                    } else {
                        tracing::warn!(
                            "[mcp:{server_name}] invalid JSON: {e} — line: {}",
                            &line_str.chars().take(100).collect::<String>()
                        );
                    }
                }
            }
        }

        // 子进程已退出，清理所有挂起的请求（通知等待者）
        tracing::debug!("[mcp:{server_name}] stdout closed, cleaning up {} pending requests", {
            let mut p = pending.lock().await;
            let count = p.len();
            p.clear();
            count
        });
    }

    /// 后台 stderr 读取器：记录子进程的 stderr 输出。
    async fn stderr_reader(stderr: ChildStderr, server_name: &str) {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty() {
                tracing::debug!("[mcp:{server_name}:stderr] {line}");
            }
        }
    }
}

// ─── 错误类型 ───────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum McpTransportError {
    #[error("I/O error: {0}")]
    Io(String),

    #[error("serialization error: {0}")]
    Serialize(String),

    #[error("request timed out")]
    Timeout,

    #[error("connection closed")]
    ConnectionClosed,
}

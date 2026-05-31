use crate::acp::types::{
    IncomingMessage, JsonRpcNotification, JsonRpcResponse, RequestId,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{mpsc, oneshot, Mutex};

/// ACP 传输层（可共享引用）
///
/// 写操作通过内部 Mutex 同步，可安全用于多任务：
/// - 服务器主循环发送通知/响应
/// - 转发工具发送双向请求
pub struct AcpTransport {
    writer: Mutex<BufWriter<tokio::io::Stdout>>,
    pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<JsonRpcResponse>>>>,
    next_id: AtomicU64,
}

impl AcpTransport {
    /// 创建传输层，返回 (AcpTransport, 消息接收端)
    ///
    /// 后台读取任务从 stdin 读取并分类：
    /// - 请求/通知 → 发送到 inbox
    /// - 响应 → 路由到 pending 中的 oneshot
    pub fn new() -> (Self, mpsc::Receiver<IncomingMessage>) {
        let (inbox_tx, inbox_rx) = mpsc::channel(256);
        let pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let pending_clone = pending.clone();
        tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let reader = BufReader::new(stdin);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(val) => {
                        let has_id = val.get("id").is_some();
                        let has_method = val.get("method").is_some();

                        if has_id && has_method {
                            if let Ok(req) =
                                serde_json::from_value::<crate::acp::types::JsonRpcRequest>(val)
                            {
                                if inbox_tx
                                    .send(IncomingMessage::Request(req))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        } else if has_id && !has_method {
                            if let Ok(resp) =
                                serde_json::from_value::<JsonRpcResponse>(val)
                            {
                                let id = resp.id.clone();
                                let mut map = pending_clone.lock().await;
                                if let Some(tx) = map.remove(&id) {
                                    let _ = tx.send(resp);
                                }
                            }
                        } else if has_method {
                            if let Ok(notif) =
                                serde_json::from_value::<crate::acp::types::JsonRpcNotification>(
                                    val,
                                )
                            {
                                if inbox_tx
                                    .send(IncomingMessage::Notification(notif))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse stdio line: {e}");
                    }
                }
            }
        });

        let transport = Self {
            writer: Mutex::new(BufWriter::new(tokio::io::stdout())),
            pending,
            next_id: AtomicU64::new(1),
        };

        (transport, inbox_rx)
    }

    fn next_id(&self) -> RequestId {
        Value::Number(self.next_id.fetch_add(1, Ordering::SeqCst).into())
    }

    /// 写入 JSON-RPC 响应到 stdout
    pub async fn send_response(&self, response: &JsonRpcResponse) -> anyhow::Result<()> {
        let line = serde_json::to_string(response)?;
        let mut writer = self.writer.lock().await;
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    /// 写入 JSON-RPC 通知到 stdout
    pub async fn send_notification(&self, notif: &JsonRpcNotification) -> anyhow::Result<()> {
        let line = serde_json::to_string(notif)?;
        let mut writer = self.writer.lock().await;
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    /// 直接写入原始 JSON Value（用于 session/update 通知）
    pub async fn send_raw(&self, value: &Value) -> anyhow::Result<()> {
        let line = serde_json::to_string(value)?;
        let mut writer = self.writer.lock().await;
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    /// 向客户端发送双向请求并等待响应
    pub async fn send_request(
        &self,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> Result<JsonRpcResponse, anyhow::Error> {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();

        // 注册 pending 请求
        {
            let mut map = self.pending.lock().await;
            map.insert(id.clone(), tx);
        }

        // 发送请求到 stdout
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_string(&request)?;
        {
            let mut writer = self.writer.lock().await;
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        // 等待响应（不持有任何锁）
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(anyhow::anyhow!("request cancelled")),
            Err(_) => Err(anyhow::anyhow!("request timed out after {timeout:?}")),
        }
    }
}

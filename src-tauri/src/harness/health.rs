//! 探测 DSH「还答应着」，而不只是「还活着」。
//!
//! 只盯进程句柄只能发现崩溃，真正把用户晾在一边的是更安静的那种失败：
//! 进程活着、端口开着、请求却没有回应——事件循环卡死、插件死锁。
//! 所以探测要真说 HTTP：TCP 连接成功什么都证明不了（内核会替
//! 没在 accept 的进程完成握手），只有收到应答才证明它还在服务。
//! 一个请求加一行状态码、走回环、无 TLS——小到直接手写，不引 HTTP 客户端。

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 最短的 HTTP 状态行前缀：`HTTP/1.1 200`。
const STATUS_LINE_PREFIX: usize = 12;

/// 在 `budget` 内确认 `origin` 仍在应答。
///
/// 任何状态码都算健康——包括 404。问题在于进程是否还在服务，
/// 不在于它怎么看这个路径。
pub async fn probe(origin: &str, budget: Duration) -> Result<(), String> {
    tokio::time::timeout(budget, exchange(origin))
        .await
        .map_err(|_| format!("{} 秒内没有应答", budget.as_secs()))?
}

async fn exchange(origin: &str) -> Result<(), String> {
    let url = url::Url::parse(origin).map_err(|cause| format!("origin 不可用：{cause}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "origin 没有 host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "origin 没有端口".to_string())?;

    let mut socket = TcpStream::connect((host, port))
        .await
        .map_err(|cause| format!("连接被拒绝：{cause}"))?;

    // HEAD 让应答只有响应头；Connection: close 让服务器发完就挂断，
    // 于是不必理解 chunked 编码或内容长度就知道这次交换结束了。
    let request = format!(
        "HEAD / HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nUser-Agent: qianxun\r\n\r\n"
    );
    socket
        .write_all(request.as_bytes())
        .await
        .map_err(|cause| format!("请求发送失败：{cause}"))?;

    let mut opening = [0u8; STATUS_LINE_PREFIX];
    socket
        .read_exact(&mut opening)
        .await
        .map_err(|cause| format!("没有状态行：{cause}"))?;

    if opening.starts_with(b"HTTP/") {
        Ok(())
    } else {
        Err(format!(
            "应答不是 HTTP：{}",
            String::from_utf8_lossy(&opening).escape_debug()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    use super::probe;

    /// 服务一个应答，然后返回它所在的 origin。
    async fn serving(reply: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(reply).await;
                let _ = socket.shutdown().await;
            }
        });
        origin
    }

    #[tokio::test]
    async fn 活着的服务器返回任何状态码都算健康() {
        let origin = serving(b"HTTP/1.1 404 Not Found\r\n\r\n").await;
        assert!(probe(&origin, Duration::from_secs(2)).await.is_ok());
    }

    #[tokio::test]
    async fn 非http应答被拒绝() {
        let origin = serving(b"SSH-2.0-OpenSSH_9.6\r\n").await;
        assert!(probe(&origin, Duration::from_secs(2)).await.is_err());
    }

    #[tokio::test]
    async fn 没有监听时失败() {
        // 绑定后立即丢弃：端口真实存在但必然关闭。
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        drop(listener);

        assert!(probe(&origin, Duration::from_secs(2)).await.is_err());
    }

    /// 这个模块存在的意义：端口接受连接，但没人应答。
    #[tokio::test]
    async fn 对只连不应的监听者超时() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        // 持有但不 accept——内核照样完成握手。
        let _held = listener;

        assert!(probe(&origin, Duration::from_millis(300)).await.is_err());
    }
}

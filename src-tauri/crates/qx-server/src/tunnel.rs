//! 隧道数据面：yamux 会话与 HTTP 服务/客户端两侧的粘合（docs/19 §4.2、§12.2）。
//!
//! - 字节流载体 = `TunnelChannels::io`（tokio duplex），yamux 经 `.compat()` 直连
//!   （自研 BytePipe 投影在并发流下有提前 FIN 问题，已废弃——见 transport.rs 教训记录）；
//! - `serve_stream`：实例端把 axum Router serve 到 yamux 流上，**必须
//!   `serve_connection_with_upgrades`**——`/api/remote.mux` 的 WS 升级靠它（评审 #9）；
//! - `HostAssert`：实例侧 Host 断言中间层（评审 #6 双保险的另一半）；
//! - `round_trip`：边缘端在一条流上完成一个 HTTP/1.1 事务（含升级响应）。

use std::future::Future;
use std::pin::Pin;

use hyper::body::Incoming;
use hyper::Request;
use tokio_util::compat::FuturesAsyncReadCompatExt;

/// 实例端：把一个 HTTP 服务（千寻网关同一份 Router 的克隆）serve 到一条 yamux 流上。
///
/// 服务是 hyper::service::Service<Request<Incoming>>（HostAssert 即是——它把 Incoming
/// 体转成 axum Body 后调用内部 Router）。流断开视为正常结束。
pub async fn serve_stream<S>(stream: yamux::Stream, service: S)
where
    S: hyper::service::Service<
            Request<Incoming>,
            Response = hyper::Response<axum::body::Body>,
            Error = std::convert::Infallible,
        > + Send
        + Clone
        + 'static,
    S::Future: Send,
{
    let io = hyper_util::rt::TokioIo::new(stream.compat());
    let builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    let _ = builder.serve_connection_with_upgrades(io, service).await;
}

/// Host 断言中间层（docs/19 §4.3）：隧道侧结构性拒绝伪造 Host（如回环 authority），
/// 同时把 Incoming 体归一为 axum Body 以便直接服务 Router。
#[derive(Clone)]
pub struct HostAssert<S> {
    inner: S,
    expected_host: String,
}

impl<S> HostAssert<S> {
    pub fn new(inner: S, expected_host: impl Into<String>) -> Self {
        Self {
            inner,
            expected_host: expected_host.into(),
        }
    }
}

impl<S> hyper::service::Service<Request<Incoming>> for HostAssert<S>
where
    S: tower::Service<
            Request<axum::body::Body>,
            Response = hyper::Response<axum::body::Body>,
            Error = std::convert::Infallible,
        > + Send
        + Clone
        + 'static,
    S::Future: Send,
{
    type Response = hyper::Response<axum::body::Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, request: Request<Incoming>) -> Self::Future {
        let host = request
            .headers()
            .get(hyper::header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if !host.eq_ignore_ascii_case(&self.expected_host) {
            let body = axum::body::Body::from(format!(
                "Host 不合法（期望 {:?}，实得 {host:?}）",
                self.expected_host
            ));
            let response = hyper::Response::builder()
                .status(400)
                .body(body)
                .expect("构造 400");
            return Box::pin(async move { Ok(response) });
        }
        let request = request.map(axum::body::Body::new);
        let mut inner = self.inner.clone();
        Box::pin(async move { tower::Service::call(&mut inner, request).await })
    }
}

/// 边缘端：在一条 yamux 流上完成一个 HTTP/1.1 事务（docs/19 §4.2）。
///
/// 101 升级响应原样返回——升级后的 IO 桥由 web 层处理。
pub async fn round_trip(
    stream: yamux::Stream,
    request: Request<axum::body::Body>,
) -> Result<hyper::Response<Incoming>, String> {
    let io = hyper_util::rt::TokioIo::new(stream.compat());
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });
    sender
        .send_request(request)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use tokio_util::compat::TokioAsyncReadCompatExt as _;

    /// 回归探针（2026-10-09 并发事故）：40 条流并发开、各自写读回显。
    /// 当年的 BytePipe 投影在这里提前 FIN；duplex+compat 架构必须保持全绿。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn probe_裸yamux40并发流回显() {
        let total: usize = std::env::var("QX_N")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(40);
        let (a, b) = crate::transport::duplex_pair();
        let mut client =
            yamux::Connection::new(a.io.compat(), yamux::Config::default(), yamux::Mode::Client);
        let mut server =
            yamux::Connection::new(b.io.compat(), yamux::Config::default(), yamux::Mode::Server);
        // 客户端连接驱动：open_stream 的命令只有连接被轮询时才被处理。
        let control = client.control();
        tokio::spawn(async move { while let Ok(Some(_stream)) = client.next_stream().await {} });
        // 服务端：接受流并逐条回显。
        tokio::spawn(async move {
            use futures::AsyncReadExt as _;
            use futures::AsyncWriteExt as _;
            while let Ok(Some(mut stream)) = server.next_stream().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 256];
                    loop {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if stream.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
        let mut jobs = Vec::new();
        for index in 0..total {
            let mut control = control.clone();
            jobs.push(tokio::spawn(async move {
                use futures::AsyncReadExt as _;
                use futures::AsyncWriteExt as _;
                let mut stream = control.open_stream().await.expect("开流");
                let payload = format!("payload-{index:02}");
                stream.write_all(payload.as_bytes()).await.expect("写");
                let mut buf = [0u8; 64];
                let n = stream.read(&mut buf).await.expect("读");
                assert_eq!(
                    &buf[..n],
                    payload.as_bytes(),
                    "流 {index} 回显不匹配（并发提前 FIN 回归）"
                );
            }));
        }
        for job in jobs {
            job.await.expect("任务");
        }
    }
}

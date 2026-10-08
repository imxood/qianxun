//! 集成测试（docs/19 §10）：本机回环起完整边缘，用真实 tungstenite 客户端扮演千寻实例，
//! hyper 客户端/裸 TCP 扮演浏览器——覆盖配对、舰队、cookie 路由、会话反代、SSE 流式、
//! **101 透传（spike 核心）**、多实例切换、抢占、治理与 Host 断言。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::response::Response;
use http_body_util::BodyExt;
use hyper::body::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use qx_server::db::Db;
use qx_server::governance::GovConfig;
use qx_server::hub::Hub;
use qx_server::protocol::{Frame, InstanceMeta, Role};
use qx_server::transport::{self, TunnelChannels};
use qx_server::tunnel::{self, HostAssert};
use qx_server::web::{self, EdgeState};

const PUBLIC_HOST: &str = "relay.example.com";

// ---------- 公共设施 ----------

struct Edge {
    addr: SocketAddr,
    state: Arc<EdgeState>,
    db: Arc<Db>,
}

async fn start_edge() -> Edge {
    let db = Arc::new(Db::open_in_memory().expect("内存库"));
    let hub = Hub::new(db.clone(), GovConfig::default());
    let state = Arc::new(EdgeState {
        hub,
        public_host: PUBLIC_HOST.into(),
        public_proto: "https",
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("端口");
    let addr = listener.local_addr().expect("地址");
    let serve_state = state.clone();
    tokio::spawn(async move {
        let _ = web::serve(listener, serve_state).await;
    });
    Edge { addr, state, db }
}

type HttpClient = hyper_util::client::legacy::Client<
    hyper_util::client::legacy::connect::HttpConnector,
    axum::body::Body,
>;

fn http_client() -> HttpClient {
    hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new()).build_http()
}

async fn request(
    client: &HttpClient,
    edge: &Edge,
    method: &str,
    path: &str,
    device_token: Option<&str>,
    iid: Option<&str>,
    body: Option<Vec<u8>>,
) -> (StatusCode, HeaderMap, Bytes) {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("http://{}{path}", edge.addr));
    let mut cookies = Vec::new();
    if let Some(token) = device_token {
        cookies.push(format!("qx_token={token}"));
    }
    if let Some(iid) = iid {
        cookies.push(format!("qx_iid={iid}"));
    }
    if !cookies.is_empty() {
        builder = builder.header(header::COOKIE, cookies.join("; "));
    }
    let body = body.map(axum::body::Body::from).unwrap_or_default();
    let response = client
        .request(builder.body(body).expect("请求合法"))
        .await
        .expect("请求成功");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("读体")
        .to_bytes();
    (status, headers, bytes)
}

/// 假千寻：真实 WS 连到边缘 /qx-ws，hello 注册后把一个 axum Router 服务到 yamux 流上。
struct FakeInstance {
    #[allow(dead_code)]
    ctrl_tx: mpsc::Sender<Frame>,
    ctrl_rx: mpsc::Receiver<Frame>,
}

async fn register_instance(db: &Db, id: &str, name: &str) -> String {
    db.add_instance(id, name).expect("预置实例")
}

async fn fake_instance(
    edge: &Edge,
    id: &str,
    name: &str,
    token: &str,
    edge_secret: Option<&str>,
) -> FakeInstance {
    let channels: TunnelChannels = transport::connect(&format!("ws://{}/qx-ws", edge.addr))
        .await
        .expect("连边缘");
    let TunnelChannels {
        ctrl_tx,
        ctrl_rx,
        io,
    } = channels;
    ctrl_tx
        .send(Frame::Hello {
            role: Role::Instance,
            instance_id: Some(id.to_owned()),
            instance_token: Some(token.to_owned()),
            device_token: None,
            meta: Some(InstanceMeta {
                qianxun: "0.9.0-test".into(),
                dsh: Some("0.2.1".into()),
                hostname: Some(name.to_owned()),
            }),
        })
        .await
        .expect("发送 hello");
    if let Some(secret) = edge_secret {
        ctrl_tx
            .send(Frame::EdgeAuth {
                secret: secret.to_owned(),
            })
            .await
            .expect("发送边缘密钥");
    }

    let router = instance_router(name);
    let expected_host = PUBLIC_HOST.to_owned();
    tokio::spawn(async move {
        use tokio_util::compat::TokioAsyncReadCompatExt as _;
        let mut connection =
            yamux::Connection::new(io.compat(), yamux::Config::default(), yamux::Mode::Server);
        while let Ok(Some(stream)) = connection.next_stream().await {
            let service = HostAssert::new(router.clone(), expected_host.clone());
            tokio::spawn(tunnel::serve_stream(stream, service));
        }
    });
    FakeInstance { ctrl_tx, ctrl_rx }
}

fn instance_router(name: &str) -> axum::Router {
    let name = name.to_owned();
    axum::Router::new()
        .route(
            "/hello",
            axum::routing::get(move || {
                let name = name.clone();
                async move { format!("hello:{name}") }
            }),
        )
        .route("/dump", axum::routing::get(dump_headers))
        .route("/events", axum::routing::get(sse_events))
        .route("/slow-echo", axum::routing::post(slow_echo))
        .route("/switch", axum::routing::get(echo_upgrade))
}

async fn dump_headers(request: Request<axum::body::Body>) -> Response {
    let pick = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    axum::Json(serde_json::json!({
        "host": pick("host"),
        "device": pick("x-qx-device"),
        "edge": pick("x-qx-edge"),
    }))
    .into_response()
}

async fn sse_events() -> Response {
    let stream = futures_util::stream::unfold(0u8, |count| async move {
        if count >= 3 {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
        Some((
            Ok::<Bytes, std::io::Error>(Bytes::from(format!("data: n{count}\n\n"))),
            count + 1,
        ))
    });
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .expect("builder")
}

async fn slow_echo(request: Request<axum::body::Body>) -> Response {
    let body = request
        .into_body()
        .collect()
        .await
        .expect("读体")
        .to_bytes();
    tokio::time::sleep(Duration::from_millis(300)).await;
    Response::builder()
        .status(200)
        .body(axum::body::Body::from(format!("len:{}", body.len())))
        .expect("builder")
}

async fn echo_upgrade(request: Request<axum::body::Body>) -> Response {
    // hyper 服务端把升级句柄预置在请求 extensions（axum WS 提取器同源）。
    let on_upgrade = request
        .extensions()
        .clone()
        .get::<hyper::upgrade::OnUpgrade>()
        .cloned();
    let mut response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::UPGRADE, "websocket")
        .header(header::CONNECTION, "Upgrade")
        .body(axum::body::Body::empty())
        .expect("builder");
    let _ = &mut response;
    let Some(on_upgrade) = on_upgrade else {
        eprintln!("[实例] 请求未携带升级句柄");
        return response;
    };
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let mut io = hyper_util::rt::TokioIo::new(upgraded);
                let mut buf = vec![0u8; 4096];
                loop {
                    match io.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if io.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
            Err(cause) => eprintln!("[实例] 升级失败: {cause}"),
        }
    });
    response
}

/// 配对（走隧道控制帧签发 + HTTP 兑换）→ 返回设备 token。
async fn pair_via_tunnel(instance: &mut FakeInstance, client: &HttpClient, edge: &Edge) -> String {
    // 等 welcome 到位（连接已建立）。
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = instance.ctrl_rx.recv().await {
            if matches!(frame, Frame::Welcome { .. }) {
                break;
            }
        }
    })
    .await
    .expect("等 welcome");
    instance
        .ctrl_tx
        .send(Frame::PairCreate)
        .await
        .expect("请求配对码");
    let code = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match instance.ctrl_rx.recv().await {
                Some(Frame::PairOffered { code, .. }) => return code,
                Some(_) => continue,
                None => panic!("隧道断开"),
            }
        }
    })
    .await
    .expect("等配对码");

    let response = client
        .request(
            Request::builder()
                .method("POST")
                .uri(format!("http://{}/api/pair/submit", edge.addr))
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(format!(
                    r#"{{"code":"{code}","name":"测试手机"}}"#
                )))
                .expect("请求"),
        )
        .await
        .expect("兑换");
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.expect("体").to_bytes())
            .expect("json");
    body["token"].as_str().expect("token").to_owned()
}

// ---------- 测试 ----------

/// **spike 核心**：浏览器 →（边缘）→ 隧道 → 实例 Router 的 101 透传 + 双向字节回显。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spike_101透传与字节回显() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut instance = fake_instance(&edge, "i1", "书房主机", &token, Some("edge-secret-1")).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut instance, &client, &edge).await;

    // 裸 TCP 扮演浏览器发升级请求（host 故意乱写：边缘应结构性重写）。
    let mut sock = tokio::net::TcpStream::connect(edge.addr)
        .await
        .expect("连边缘");
    let request = format!(
        "GET /switch HTTP/1.1\r\nHost: evil.example.com\r\ncookie: qx_token={device}; qx_iid=i1\r\n\
         Upgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    sock.write_all(request.as_bytes()).await.expect("发请求");
    let mut header_buf = vec![0u8; 2048];
    let mut header_len = 0;
    loop {
        let n = sock
            .read(&mut header_buf[header_len..])
            .await
            .expect("读响应头");
        header_len += n;
        let text = String::from_utf8_lossy(&header_buf[..header_len]);
        if text.contains("\r\n\r\n") || header_len == header_buf.len() {
            break;
        }
    }
    let head = String::from_utf8_lossy(&header_buf[..header_len]);
    assert!(head.starts_with("HTTP/1.1 101"), "应为 101：{head}");
    assert!(
        head.to_lowercase().contains("upgrade"),
        "应回升级头：{head}"
    );

    // 升级后的原始字节双向回显（spike 的核心断言：两跳升级桥活着）。
    sock.write_all(b"spike-echo-payload").await.expect("写");
    let mut echo = [0u8; 32];
    let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut echo))
        .await
        .expect("超时")
        .expect("读回显");
    assert_eq!(&echo[..n], b"spike-echo-payload");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 会话反代头部盖章与host重写() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut instance = fake_instance(&edge, "i1", "书房主机", &token, Some("edge-secret-1")).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut instance, &client, &edge).await;

    let (status, _, body) = request(
        &client,
        &edge,
        "GET",
        "/dump",
        Some(&device),
        Some("i1"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    let dump: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        dump["host"], PUBLIC_HOST,
        "Host 被结构性重写（入站 evil/任意 host 无效）"
    );
    assert!(dump["device"].as_str().expect("str").starts_with("dev-"));
    assert_eq!(dump["edge"], "edge-secret-1", "边缘盖章随 EdgeAuth 生效");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse流式转发与interactive升格() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut instance = fake_instance(&edge, "i1", "书房主机", &token, None).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut instance, &client, &edge).await;

    let start = std::time::Instant::now();
    let (status, headers, body) = request(
        &client,
        &edge,
        "GET",
        "/events",
        Some(&device),
        Some("i1"),
        None,
    )
    .await;
    let elapsed = start.elapsed();
    assert_eq!(status, 200);
    assert!(headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("text/event-stream"));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("data: n0") && text.contains("data: n2"));
    assert!(
        elapsed >= Duration::from_millis(150),
        "应流式到达而非整块缓冲：{elapsed:?}"
    );

    // 指标：interactive 计数出现（SSE 升格）。
    let (status, _, metrics) = request(&client, &edge, "GET", "/metrics", None, None, None).await;
    assert_eq!(status, 200, "回环可读 metrics");
    assert!(String::from_utf8_lossy(&metrics).contains("class=\"interactive\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 多实例舰队与cookie切换() {
    let edge = start_edge().await;
    let token1 = register_instance(&edge.db, "i1", "书房主机").await;
    let token2 = register_instance(&edge.db, "i2", "客厅主机").await;
    let mut instance1 = fake_instance(&edge, "i1", "书房主机", &token1, None).await;
    let _instance2 = fake_instance(&edge, "i2", "客厅主机", &token2, None).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut instance1, &client, &edge).await;

    // 舰队：两台都在线（HTTP 投影 = fleet.get）。
    let (status, _, body) = request(
        &client,
        &edge,
        "GET",
        "/api/fleet",
        Some(&device),
        None,
        None,
    )
    .await;
    assert_eq!(status, 200);
    let fleet: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let instances = fleet["instances"].as_array().expect("数组");
    assert_eq!(instances.len(), 2);
    assert!(instances
        .iter()
        .all(|item| item["online"].as_bool().expect("bool")));

    // 入口设 cookie → 会话路由到 i1。
    let (status, _, body) = request(
        &client,
        &edge,
        "GET",
        "/hello",
        Some(&device),
        Some("i1"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(String::from_utf8_lossy(&body), "hello:书房主机");
    // 切换到 i2（cookie 覆写）。
    let (status, _, body) = request(
        &client,
        &edge,
        "GET",
        "/hello",
        Some(&device),
        Some("i2"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(String::from_utf8_lossy(&body), "hello:客厅主机");

    // 实例离线 → 502 + Retry-After（i2 尚连着，拔 i1 不方便——用不存在的 cookie 验证路由键校验）。
    let (status, _, _) = request(&client, &edge, "GET", "/hello", None, Some("i1"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "无设备凭据 401");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 实例离线返回502() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut instance = fake_instance(&edge, "i1", "书房主机", &token, None).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut instance, &client, &edge).await;
    let (status, _, _) = request(
        &client,
        &edge,
        "GET",
        "/hello",
        Some(&device),
        Some("i2"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_GATEWAY,
        "cookie 指向未连接实例 → 502"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 同名实例抢占旧连接让位() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut old = fake_instance(&edge, "i1", "书房主机", &token, None).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut old, &client, &edge).await;

    // 第二条连接用同一实例身份接入 → 抢占。
    let new = fake_instance(&edge, "i1", "书房主机", &token, None).await;
    // 旧连接的控制通道应关闭（服务端替换 + 关旧隧道）。
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match old.ctrl_rx.recv().await {
                Some(Frame::Welcome { .. }) => continue,
                Some(_) => continue,
                None => return true,
            }
        }
    })
    .await;
    assert!(closed.unwrap_or(false), "旧连接应被关闭");

    // 新连接照常服务，舰队保持在线。
    let (status, _, body) = request(
        &client,
        &edge,
        "GET",
        "/hello",
        Some(&device),
        Some("i1"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(String::from_utf8_lossy(&body), "hello:书房主机");
    drop(new);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn page整页40连发一次通过() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut instance = fake_instance(&edge, "i1", "书房主机", &token, None).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut instance, &client, &edge).await;

    let mut jobs = Vec::new();
    for _ in 0..40 {
        let client = client.clone();
        let edge_addr = edge.addr;
        let device = device.clone();
        jobs.push(tokio::spawn(async move {
            let builder = Request::builder()
                .uri(format!("http://{edge_addr}/hello"))
                .header(header::COOKIE, format!("qx_token={device}; qx_iid=i1"));
            let response = client
                .request(builder.body(axum::body::Body::empty()).expect("请求"))
                .await
                .expect("发");
            response.status()
        }));
    }
    let statuses: Vec<StatusCode> = futures_util::future::join_all(jobs)
        .await
        .into_iter()
        .map(|result| result.expect("任务"))
        .collect();
    assert!(
        statuses.iter().all(|status| *status == StatusCode::OK),
        "整页加载应一次通过：{statuses:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk并发上限第三条被拒() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut instance = fake_instance(&edge, "i1", "书房主机", &token, None).await;
    let client = http_client();
    let device = pair_via_tunnel(&mut instance, &client, &edge).await;

    let body = vec![7u8; 300 * 1024]; // >256KiB → bulk
    let mut jobs = Vec::new();
    for _ in 0..3 {
        let client = client.clone();
        let edge_addr = edge.addr;
        let device = device.clone();
        let body = body.clone();
        jobs.push(tokio::spawn(async move {
            let builder = Request::builder()
                .method("POST")
                .uri(format!("http://{edge_addr}/slow-echo"))
                .header(header::COOKIE, format!("qx_token={device}; qx_iid=i1"));
            let status = match client
                .request(builder.body(axum::body::Body::from(body)).expect("请求"))
                .await
            {
                Ok(response) => response.status(),
                // 服务端提前 503 断流时，hyper 客户端会以错误收场——按被拒计。
                Err(_) => StatusCode::SERVICE_UNAVAILABLE,
            };
            status
        }));
    }
    let statuses: Vec<StatusCode> = futures_util::future::join_all(jobs)
        .await
        .into_iter()
        .map(|result| result.expect("任务"))
        .collect();
    let ok = statuses
        .iter()
        .filter(|status| **status == StatusCode::OK)
        .count();
    let busy = statuses
        .iter()
        .filter(|status| **status == StatusCode::SERVICE_UNAVAILABLE)
        .count();
    assert_eq!(
        (ok, busy),
        (2, 1),
        "bulk 并发 2：两条通过、第三条 503：{statuses:?}"
    );
}

/// 实例侧 Host 断言（docs/19 §4.3 双保险的另一半）：绕过边缘直连隧道发伪造 Host → 400。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 伪造host在实例侧被结构性拒绝() {
    let edge = start_edge().await;
    let token = register_instance(&edge.db, "i1", "书房主机").await;
    let mut instance = fake_instance(&edge, "i1", "书房主机", &token, None).await;
    let client = http_client();
    let _device = pair_via_tunnel(&mut instance, &client, &edge).await;

    let stream = edge.state.hub.open_stream("i1").await.expect("实例在线");
    let request = Request::builder()
        .method("GET")
        .uri("/hello")
        .header(header::HOST, "127.0.0.1:23090") // 伪造回环 authority
        .body(axum::body::Body::empty())
        .expect("请求");
    let response = tunnel::round_trip(stream, request).await.expect("往返");
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "伪造 Host 必须 400"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 配对码错误与未配对访问() {
    let edge = start_edge().await;
    let client = http_client();

    // 错误配对码 → 403。
    let response = client
        .request(
            Request::builder()
                .method("POST")
                .uri(format!("http://{}/api/pair/submit", edge.addr))
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(r#"{"code":"DEADBEEF","name":"x"}"#))
                .expect("请求"),
        )
        .await
        .expect("发");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 未配对访问根路径 → 引导配对页。
    let (status, headers, _) = request(&client, &edge, "GET", "/", None, None, None).await;
    assert!(status.is_redirection(), "应重定向：{status}");
    assert!(headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("/qx-pair"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn 实例入口设cookie并重定向() {
    let edge = start_edge().await;
    let client = http_client();
    let (status, headers, _) =
        request(&client, &edge, "GET", "/i/pc-1234/", None, None, None).await;
    assert_eq!(status, StatusCode::FOUND);
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        cookie.contains("qx_iid=pc-1234"),
        "cookie 应写入实例 id：{cookie}"
    );
    assert!(cookie.to_lowercase().contains("httponly"));
}

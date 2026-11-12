//! 千寻端在线客户端（docs/19 v5.2 §12.3）：纯客户端——只连接、不托管任何服务。
//!
//! 生命周期：settings.server 变化 → `sync` 重建（指纹独立于网关——重建网关会杀
//! 全部会话连接，服务端连接的重建不应牵连）。连接循环：hello → edge.auth →
//! 会话（心跳 / 配对码 / 设备快照 / yamux 流服务）→ 断线退避重连。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, watch};
use tokio_util::compat::TokioAsyncReadCompatExt as _;

use crate::remote;
use qx_server::protocol::{DeviceInfo, Frame, InstanceMeta, Role};
use qx_server::tunnel::{self, HostAssert};

/// 断线退避起步与上限（docs/19 §4.1：0.5s 起步，32s 封顶 + 抖动）。
const BACKOFF_START: Duration = Duration::from_millis(500);
const BACKOFF_CAP: Duration = Duration::from_secs(32);
/// 心跳周期（服务端 welcome 约定 20s；千寻单侧发起）。
const HEARTBEAT: Duration = Duration::from_secs(20);
/// 等 hello 应答 / 配对码应答的超时。
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const PAIR_REPLY_TIMEOUT: Duration = Duration::from_secs(6);

/// 连接状态（IPC `server_status` 的数据源）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineStatus {
    /// off / connecting / online / backoff / invalid_config
    pub state: String,
    pub url: String,
    pub instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 服务端广播的设备清单副本（展示用；执法在服务端）。
    pub devices: Vec<DeviceInfo>,
    /// 最近一次配对码（远程页展示；一次性，展示完即弃）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_pair: Option<PairOffer>,
}

/// 供前端展示的配对码快照。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairOffer {
    pub code: String,
    pub expires_at: i64,
    /// 手机直接可开的配对页地址（wss→https / ws→http）。
    pub pair_url: String,
}

/// 运行句柄：sync 层按指纹重建。
pub struct OnlineHandle {
    pub fingerprint: u64,
    pub shutdown: watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
    pub status: watch::Receiver<Arc<OnlineStatus>>,
    /// 会话内可用的控制通道（None = 未连接）。
    pub ctrl: Arc<Mutex<Option<mpsc::Sender<Frame>>>>,
    /// 挂起的配对码请求（PairCreate → PairOffered 一次性兑现）。
    pub pair_reply: Arc<Mutex<Option<tokio::sync::oneshot::Sender<PairOffer>>>>,
    /// 挂起的设备清单请求（DeviceList → DeviceSnapshot）。
    pub devices_reply: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Vec<DeviceInfo>>>>>,
}

/// 会话结束原因：Reconnect 走退避；Stop 停止循环（配置无效/主动停机）。
enum SessionEnd {
    Reconnect(String),
    Stop(String),
}

// ---- 地址派生（纯函数，单测覆盖）----

/// 从 settings.url 派生 /qx-ws 的完整连接地址。
pub fn tunnel_url(url: &str) -> String {
    format!("{}/qx-ws", url.trim_end_matches('/'))
}

/// 从 settings.url 提取 Host 断言目标（scheme 之后的 authority，host[:port]）。
pub fn expected_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .or_else(|| url.strip_prefix("https://"))
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?']).next()?;
    (!authority.is_empty()).then(|| authority.to_owned())
}

/// 配对页地址（手机扫码/直开）：scheme 按 ws/wss 映射 http/https。
pub fn pair_page_url(url: &str, code: &str) -> String {
    let base = if let Some(rest) = url.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = url.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        url.to_owned()
    };
    format!("{}/qx-pair?code={code}", base.trim_end_matches('/'))
}

fn online_fingerprint(url: &str, instance_id: &str, instance_token: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    instance_id.hash(&mut hasher);
    instance_token.hash(&mut hasher);
    hasher.finish()
}

// ---- 生命周期 ----

/// 同步入口（commands::sync 调用）：指纹一致且任务存活 → 保留；否则停旧起新。
pub fn sync(app: &AppHandle) {
    let state = app.state::<crate::AppState>();
    let (url, instance_id, instance_token) = {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            settings.server.url.trim().to_owned(),
            settings.server.instance_id.trim().to_owned(),
            settings.server.instance_token.trim().to_owned(),
        )
    };
    let configured = !url.is_empty() && !instance_id.is_empty() && !instance_token.is_empty();
    let scheme_ok = url.starts_with("ws://") || url.starts_with("wss://");
    let desired = online_fingerprint(&url, &instance_id, &instance_token);

    let mut running = state
        .remote
        .online
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // 未配置 / scheme 不合法：停掉现有连接（状态由任务收尾时置 invalid_config/off）。
    if !configured || !scheme_ok {
        if let Some(handle) = running.take() {
            let _ = handle.shutdown.send(true);
            handle.task.abort();
        }
        return;
    }
    // 指纹未变且任务健在 → 保留。
    if running
        .as_ref()
        .is_some_and(|handle| !handle.task.is_finished() && handle.fingerprint == desired)
    {
        return;
    }
    if let Some(handle) = running.take() {
        let _ = handle.shutdown.send(true);
        handle.task.abort();
    }
    let (status_tx, status_rx) = watch::channel(Arc::new(OnlineStatus {
        state: "connecting".into(),
        url: url.clone(),
        instance_id: instance_id.clone(),
        detail: None,
        devices: Vec::new(),
        last_pair: None,
    }));
    let ctrl = Arc::new(Mutex::new(None));
    let pair_reply = Arc::new(Mutex::new(None));
    let devices_reply = Arc::new(Mutex::new(None));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(run(
        url.clone(),
        instance_id,
        instance_token,
        status_tx,
        ctrl.clone(),
        pair_reply.clone(),
        devices_reply.clone(),
        shutdown_rx,
    ));
    *running = Some(OnlineHandle {
        shutdown: shutdown_tx,
        task,
        status: status_rx,
        ctrl,
        pair_reply,
        devices_reply,
        fingerprint: desired,
    });
}

/// 连接循环：会话结束 → 退避重连；Stop 结束原因才退出。
#[allow(clippy::too_many_arguments)]
async fn run(
    url: String,
    instance_id: String,
    instance_token: String,
    status: watch::Sender<Arc<OnlineStatus>>,
    ctrl: Arc<Mutex<Option<mpsc::Sender<Frame>>>>,
    pair_reply: Arc<Mutex<Option<tokio::sync::oneshot::Sender<PairOffer>>>>,
    devices_reply: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Vec<DeviceInfo>>>>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut backoff = BACKOFF_START;
    loop {
        if *shutdown.borrow_and_update() {
            set_state(&status, "off", None);
            return;
        }
        set_state(&status, "connecting", None);
        match session_once(
            &url,
            &instance_id,
            &instance_token,
            &status,
            &ctrl,
            &pair_reply,
            &devices_reply,
            &mut shutdown,
        )
        .await
        {
            SessionEnd::Reconnect(cause) => {
                set_state(
                    &status,
                    "backoff",
                    Some(format!("{cause}；{backoff:?} 后重连")),
                );
                crate::logging::log(
                    "warn",
                    &format!("[服务器] 连接断开：{cause}，{backoff:?} 后重连"),
                );
            }
            SessionEnd::Stop(cause) => {
                set_state(&status, "invalid_config", Some(cause));
                crate::logging::log(
                    "warn",
                    "[服务器] 停止连接（配置无效或凭据被拒，修改设置后自动重试）",
                );
                return;
            }
        }
        tokio::select! {
            _ = shutdown.changed() => {
                set_state(&status, "off", None);
                return;
            }
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(BACKOFF_CAP);
    }
}

fn set_state(status: &watch::Sender<Arc<OnlineStatus>>, state: &str, detail: Option<String>) {
    status.send_modify(|current| {
        let mut next = (**current).clone();
        next.state = state.to_owned();
        next.detail = detail;
        *current = Arc::new(next);
    });
}

/// 单次会话：连接 → hello → edge.auth → 服务直至断开。
#[allow(clippy::too_many_arguments)]
async fn session_once(
    url: &str,
    instance_id: &str,
    instance_token: &str,
    status: &watch::Sender<Arc<OnlineStatus>>,
    ctrl_slot: &Arc<Mutex<Option<mpsc::Sender<Frame>>>>,
    pair_reply: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<PairOffer>>>>,
    devices_reply: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<Vec<DeviceInfo>>>>>,
    shutdown: &mut watch::Receiver<bool>,
) -> SessionEnd {
    let Some(host) = expected_host(url) else {
        return SessionEnd::Stop("地址缺少 host".into());
    };
    let channels = match qx_server::transport::connect(&tunnel_url(url)).await {
        Ok(channels) => channels,
        Err(cause) => return SessionEnd::Reconnect(cause),
    };
    let qx_server::transport::TunnelChannels {
        ctrl_tx,
        mut ctrl_rx,
        io,
    } = channels;

    // hello：instance 角色 + 预置凭据。
    if ctrl_tx
        .send(Frame::Hello {
            role: Role::Instance,
            instance_id: Some(instance_id.to_owned()),
            instance_token: Some(instance_token.to_owned()),
            device_token: None,
            meta: Some(InstanceMeta {
                qianxun: env!("CARGO_PKG_VERSION").to_owned(),
                dsh: crate::harness::install::PINNED_VERSION.to_owned().into(),
                hostname: hostname::get().ok().and_then(|h| h.into_string().ok()),
            }),
        })
        .await
        .is_err()
    {
        return SessionEnd::Reconnect("发送 hello 失败".into());
    }
    // 等 welcome（或注册失败错误）。
    match tokio::time::timeout(HELLO_TIMEOUT, ctrl_rx.recv()).await {
        Ok(Some(Frame::Welcome { .. })) => {}
        Ok(Some(Frame::Error { code, message })) => {
            let detail = message.unwrap_or_else(|| code.clone());
            return SessionEnd::Stop(format!("{code}：{detail}"));
        }
        Ok(Some(_)) => {}
        Ok(None) => return SessionEnd::Reconnect("等待 welcome 时连接关闭".into()),
        Err(_) => return SessionEnd::Reconnect("等待 welcome 超时".into()),
    }
    // 边缘密钥上报（服务端转发请求时盖章；docs/19 §4.3）。
    if ctrl_tx
        .send(Frame::EdgeAuth {
            secret: remote::edge_secret().to_owned(),
        })
        .await
        .is_err()
    {
        return SessionEnd::Reconnect("发送边缘密钥失败".into());
    }

    // 数据面：yamux（Server 模式——流由边缘发起，本端被动接受）。
    let mut connection =
        yamux::Connection::new(io.compat(), yamux::Config::default(), yamux::Mode::Server);

    // 注册控制通道 + 上线状态；离开作用域时清理（ctrl 清空、设备挂起请求落空）。
    *ctrl_slot.lock().unwrap() = Some(ctrl_tx.clone());
    set_state(status, "online", None);
    crate::logging::log(
        "info",
        &format!("[服务器] 已连接 {url}（实例 {instance_id}）"),
    );
    // 连接成功后退避复位。
    let end = session_loop(
        &mut connection,
        &ctrl_tx,
        &mut ctrl_rx,
        &host,
        status,
        pair_reply,
        devices_reply,
        shutdown,
    )
    .await;
    *ctrl_slot.lock().unwrap() = None;
    *pair_reply.lock().unwrap() = None;
    *devices_reply.lock().unwrap() = None;
    set_state(status, "backoff", None);
    end
}

/// 会话主体：控制帧 + 入站流 + 心跳 + 停机信号四路 select。
#[allow(clippy::too_many_arguments)]
async fn session_loop(
    connection: &mut yamux::Connection<tokio_util::compat::Compat<tokio::io::DuplexStream>>,
    ctrl_tx: &mpsc::Sender<Frame>,
    ctrl_rx: &mut mpsc::Receiver<Frame>,
    host: &str,
    status: &watch::Sender<Arc<OnlineStatus>>,
    pair_reply: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<PairOffer>>>>,
    devices_reply: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<Vec<DeviceInfo>>>>>,
    shutdown: &mut watch::Receiver<bool>,
) -> SessionEnd {
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await; // 首个立即 tick 跳过。
    loop {
        tokio::select! {
            _ = shutdown.changed() => return SessionEnd::Stop("停机".into()),
            frame = ctrl_rx.recv() => {
                let Some(frame) = frame else {
                    return SessionEnd::Reconnect("控制通道关闭".into());
                };
                match frame {
                    Frame::Ping => {
                        if ctrl_tx.send(Frame::Pong).await.is_err() {
                            return SessionEnd::Reconnect("回心跳失败".into());
                        }
                    }
                    Frame::PairOffered { code, expires_at } => {
                        let url = status.borrow().url.clone();
                        let offer = PairOffer { pair_url: pair_page_url(&url, &code), code, expires_at };
                        resolve_pair(pair_reply, offer.clone());
                        status.send_modify(|current| {
                            let mut next = (**current).clone();
                            next.last_pair = Some(offer);
                            *current = Arc::new(next);
                        });
                    }
                    Frame::DeviceSnapshot { devices } => {
                        resolve_devices(devices_reply, devices.clone());
                        status.send_modify(|current| {
                            let mut next = (**current).clone();
                            next.devices = devices;
                            *current = Arc::new(next);
                        });
                    }
                    Frame::PairGranted { device_id, name } => {
                        crate::logging::log("info", &format!("[服务器] 新设备已配对：{name}（{device_id}）"));
                    }
                    Frame::Error { code, message } => {
                        let detail = message.unwrap_or_default();
                        if code == "bad_token" || code == "unknown_instance" {
                            return SessionEnd::Stop(format!("{code}：{detail}"));
                        }
                    }
                    _ => {}
                }
            }
            stream = connection.next_stream() => {
                match stream {
                    Ok(Some(stream)) => {
                        // 每流取最新 Router（RouterHandle，docs/19 §3.3）。
                        let router = remote::router_handle().load_full();
                        let service = HostAssert::new((*router).clone(), host.to_owned());
                        tokio::spawn(async move {
                            tunnel::serve_stream(stream, service).await;
                        });
                    }
                    Ok(None) => return SessionEnd::Reconnect("数据面关闭".into()),
                    Err(cause) => return SessionEnd::Reconnect(format!("数据面错误：{cause}")),
                }
            }
            _ = heartbeat.tick() => {
                if ctrl_tx.send(Frame::Ping).await.is_err() {
                    return SessionEnd::Reconnect("发送心跳失败".into());
                }
            }
        }
    }
}

fn resolve_pair(
    slot: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<PairOffer>>>>,
    offer: PairOffer,
) {
    if let Some(sender) = slot.lock().unwrap().take() {
        let _ = sender.send(offer);
    }
}

fn resolve_devices(
    slot: &Arc<Mutex<Option<tokio::sync::oneshot::Sender<Vec<DeviceInfo>>>>>,
    devices: Vec<DeviceInfo>,
) {
    if let Some(sender) = slot.lock().unwrap().take() {
        let _ = sender.send(devices);
    }
}

// ---- IPC 侧 API ----

/// 当前状态快照（server_status 命令）。
pub fn status(app: &AppHandle) -> OnlineStatus {
    let state = app.state::<crate::AppState>();
    let running = state
        .remote
        .online
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(handle) = running.as_ref() {
        return (**handle.status.borrow()).clone();
    }
    OnlineStatus::default()
}

/// 请求服务端签发一次性配对码（server_pair_create 命令）。
pub async fn request_pair_code(app: &AppHandle) -> Result<PairOffer, String> {
    let (ctrl, pair_reply) = {
        let state = app.state::<crate::AppState>();
        let running = state
            .remote
            .online
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(handle) = running.as_ref() else {
            return Err("服务端未连接（先填地址与实例凭据）".into());
        };
        let taken = (
            handle.ctrl.lock().unwrap().clone(),
            handle.pair_reply.clone(),
        );
        taken
    };
    let Some(ctrl) = ctrl else {
        return Err("服务端连接未就绪".into());
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<PairOffer>();
    *pair_reply.lock().unwrap() = Some(tx);
    ctrl.send(Frame::PairCreate)
        .await
        .map_err(|_| "发送配对请求失败（连接可能已断开）".to_owned())?;
    match tokio::time::timeout(PAIR_REPLY_TIMEOUT, rx).await {
        Ok(Ok(offer)) => Ok(offer),
        Ok(Err(_)) => Err("配对请求已取消".into()),
        Err(_) => {
            *pair_reply.lock().unwrap() = None;
            Err("等待配对码超时".into())
        }
    }
}

/// 拉取设备清单（server_device_list 命令）：优先向服务端要新的，失败退回缓存。
pub async fn device_list(app: &AppHandle) -> Vec<DeviceInfo> {
    let (ctrl, devices_reply, cached) = {
        let state = app.state::<crate::AppState>();
        let running = state
            .remote
            .online
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(handle) = running.as_ref() else {
            return Vec::new();
        };
        let taken = (
            handle.ctrl.lock().unwrap().clone(),
            handle.devices_reply.clone(),
            handle.status.borrow().devices.clone(),
        );
        taken
    };
    let Some(ctrl) = ctrl else {
        return cached;
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<DeviceInfo>>();
    *devices_reply.lock().unwrap() = Some(tx);
    if ctrl.send(Frame::DeviceList).await.is_err() {
        return cached;
    }
    match tokio::time::timeout(PAIR_REPLY_TIMEOUT, rx).await {
        Ok(Ok(devices)) => devices,
        _ => cached,
    }
}

/// 吊销设备（server_device_revoke 命令）：发指令，清单随后经快照到达。
pub async fn revoke_device(app: &AppHandle, device_id: &str) -> Result<(), String> {
    let (ctrl, devices_reply) = {
        let state = app.state::<crate::AppState>();
        let running = state
            .remote
            .online
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(handle) = running.as_ref() else {
            return Err("服务端未连接".into());
        };
        let taken = (
            handle.ctrl.lock().unwrap().clone(),
            handle.devices_reply.clone(),
        );
        taken
    };
    let Some(ctrl) = ctrl else {
        return Err("服务端连接未就绪".into());
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<DeviceInfo>>();
    *devices_reply.lock().unwrap() = Some(tx);
    ctrl.send(Frame::DeviceRevoke {
        device_id: device_id.to_owned(),
    })
    .await
    .map_err(|_| "发送吊销指令失败".to_owned())?;
    let _ = tokio::time::timeout(PAIR_REPLY_TIMEOUT, rx).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 隧道地址与host派生() {
        assert_eq!(
            tunnel_url("ws://127.0.0.1:23100"),
            "ws://127.0.0.1:23100/qx-ws"
        );
        assert_eq!(tunnel_url("wss://a.b.c/"), "wss://a.b.c/qx-ws");
        assert_eq!(
            expected_host("ws://127.0.0.1:23100").as_deref(),
            Some("127.0.0.1:23100")
        );
        assert_eq!(
            expected_host("wss://relay.example.com").as_deref(),
            Some("relay.example.com")
        );
        assert_eq!(
            expected_host("wss://relay.example.com/path").as_deref(),
            Some("relay.example.com")
        );
        assert_eq!(expected_host("not-a-url"), None);
    }

    #[test]
    fn 配对页地址按scheme映射() {
        assert_eq!(
            pair_page_url("wss://a.b.c", "AB12"),
            "https://a.b.c/qx-pair?code=AB12"
        );
        assert_eq!(
            pair_page_url("ws://192.168.1.5:23100/", "AB12"),
            "http://192.168.1.5:23100/qx-pair?code=AB12"
        );
    }

    #[test]
    fn 在线指纹随任一字段变化() {
        let base = online_fingerprint("ws://x", "i1", "t1");
        assert_eq!(base, online_fingerprint("ws://x", "i1", "t1"));
        assert_ne!(base, online_fingerprint("ws://y", "i1", "t1"));
        assert_ne!(base, online_fingerprint("ws://x", "i2", "t1"));
        assert_ne!(base, online_fingerprint("ws://x", "i1", "t2"));
    }

    /// 端到端（docs/19 M1 验收主链路）：内存库起真边缘（qx-server web::serve），
    /// 千寻客户端 session_once 接入（hello → edge.auth → yamux Server 流服务），
    /// 设备请求经「HTTP → 边缘反代 → 隧道 → HostAssert → RouterHandle」打到
    /// 千寻网关 Router——RouterHandle 发布/读取与 Host 断言全在真实路径上。
    #[tokio::test]
    async fn 端到端_接入边缘并转发请求() {
        use qx_server::governance::GovConfig;
        use qx_server::hub::Hub;
        use qx_server::web::{self, EdgeState};

        let db = std::sync::Arc::new(qx_server::db::Db::open_in_memory().expect("内存库"));
        let instance_token = db.add_instance("dev-i1", "书房主机").expect("预置实例");
        let device_token = qx_server::crypto::random_hex(32);
        db.add_device("dev-phone", "我的手机", &device_token)
            .expect("预置设备");
        let hub = Hub::new(db, GovConfig::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑边缘端口");
        let addr = listener.local_addr().expect("边缘地址");
        let state = std::sync::Arc::new(EdgeState {
            hub,
            public_host: addr.to_string(),
            public_proto: "http",
            limiter: std::sync::Arc::new(qx_server::web::AttemptLimiter::default()),
        });
        tokio::spawn(web::serve(listener, state));

        // RouterHandle 预发布最小网关表（真实网关的替身；在线会话每流 load_full）。
        remote::router_handle().store(std::sync::Arc::new(
            axum::Router::new().fallback(|| async { "ok-from-qianxun" }),
        ));

        let (status_tx, status_rx) = watch::channel(Arc::new(OnlineStatus {
            url: format!("ws://{addr}"),
            instance_id: "dev-i1".into(),
            ..OnlineStatus::default()
        }));
        let ctrl_slot = Arc::new(Mutex::new(None));
        let pair_reply = Arc::new(Mutex::new(None));
        let devices_reply = Arc::new(Mutex::new(None));
        let session = {
            let url = format!("ws://{addr}");
            let status_tx = status_tx.clone();
            let ctrl_slot = ctrl_slot.clone();
            let pair_reply = pair_reply.clone();
            let devices_reply = devices_reply.clone();
            tokio::spawn(async move {
                let (_shutdown_tx, mut shutdown) = watch::channel(false);
                session_once(
                    &url,
                    "dev-i1",
                    &instance_token,
                    &status_tx,
                    &ctrl_slot,
                    &pair_reply,
                    &devices_reply,
                    &mut shutdown,
                )
                .await
            })
        };

        // 等上线（hello/welcome/edge.auth 全过）。
        let mut online = status_rx.clone();
        loop {
            if online.borrow().state == "online" {
                break;
            }
            online.changed().await.expect("状态通道存活");
        }
        assert!(ctrl_slot.lock().unwrap().is_some(), "控制通道应在位");

        // 设备完整链请求：cookie 鉴权 → 治理 → 开流 → Host 重写 → 千寻 Router。
        // router_handle 是**进程级全局**（生产：一进程一网关；测试：并行的
        // 网关测试 start() 会覆写它，本测试的流在请求时 load_full 就可能拿
        // 到别人的 Router，表现为 502「上游不可达」）。所以每次尝试前重新
        // 发布本测试的最小 Router，命中别人的 Router 就重试——生产路径
        // （每流 load_full）不动，只把测试对这个全局的依赖稳住。
        let client = reqwest::Client::new();
        let mut body = String::new();
        for _ in 0..5 {
            remote::router_handle().store(std::sync::Arc::new(
                axum::Router::new().fallback(|| async { "ok-from-qianxun" }),
            ));
            let response = client
                .get(format!("http://{addr}/"))
                .header("cookie", format!("qx_token={device_token}; qx_iid=dev-i1"))
                .send()
                .await
                .expect("反代请求");
            let status = response.status();
            body = response.text().await.expect("响应体");
            if status.is_success() && body == "ok-from-qianxun" {
                break;
            }
        }
        assert_eq!(body, "ok-from-qianxun");

        session.abort();
    }
}

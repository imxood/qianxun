//! 远程域 IPC：网卡枚举 / 状态 / 配对 / 吊销 / 随设置同步启停。

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::remote::{self, gateway::GatewayHandle, online::OnlineHandle, RemoteDevice};

/// 网关运行态：Some = 监听中。锁内持 JoinHandle，停机经 shutdown 通道。
/// `sync_lock`：sync 全程互斥——DSH 就绪与设置保存可能并发触发两次
/// sync，并发重建会在「旧监听器尚未释放」时抢绑同一端口（实测 10048）。
#[derive(Default)]
pub struct RemoteState {
    pub running: Mutex<Option<GatewayHandle>>,
    /// 服务端在线客户端（docs/19）：与网关生命周期分离（指纹独立），
    /// 配置变化只重连隧道、不重启网关。
    pub online: Mutex<Option<OnlineHandle>>,
    pub sync_lock: tokio::sync::Mutex<()>,
}

/// 一块网卡（一个 IP 一行；EasyTier 识别按接口名/友好名包含判定）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetInterface {
    pub name: String,
    pub ip: String,
    pub easytier: bool,
}

/// 远程域状态总览（设置页卡片）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub enabled: bool,
    pub bind_ip: String,
    pub port: u16,
    pub listening: Option<String>,
    pub device_count: usize,
    pub active_count: usize,
    /// DSH 是否在跑（网关可用性的前提）。
    pub dsh_running: bool,
    /// 本地服务反代（docs/20）：设置态 + 监听态 + 端口（网关端口 + 2）。
    pub proxy_enabled: bool,
    pub proxy_port: u16,
    pub proxy_listening: Option<String>,
}

/// 本机全部网卡地址（含 EasyTier 识别），供绑定选择。
#[tauri::command]
pub fn remote_interfaces() -> Vec<NetInterface> {
    let mut list = Vec::new();
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for interface in interfaces {
            let ip = interface.ip().to_string();
            if ip.starts_with("127.") || ip.contains(':') {
                continue; // 回环无意义；只绑 IPv4（EasyTier 即 v4）。
            }
            let name = interface.name.clone();
            let easytier = name.to_lowercase().contains("easytier");
            list.push(NetInterface { name, ip, easytier });
        }
    }
    list.sort_by(|a, b| b.easytier.cmp(&a.easytier).then(a.ip.cmp(&b.ip)));
    list
}

#[tauri::command]
pub fn remote_status(app: AppHandle) -> RemoteStatus {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap();
    let remote = &settings.remote;
    let listening = state
        .remote
        .running
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|handle| handle.lan_addr.borrow().map(|a| a.to_string()));
    let proxy_listening = state
        .remote
        .running
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|handle| handle.proxy_addr.borrow().map(|a| a.to_string()));
    let dsh_running = matches!(
        state.harness.supervisor.status(),
        crate::harness::supervisor::Status::Starting
            | crate::harness::supervisor::Status::Ready { .. }
            | crate::harness::supervisor::Status::Restarting { .. }
    );
    RemoteStatus {
        enabled: remote.enabled,
        bind_ip: remote.bind_ip.clone(),
        port: remote.port,
        listening,
        device_count: remote.devices.len(),
        active_count: remote.devices.iter().filter(|d| !d.revoked).count(),
        dsh_running,
        proxy_enabled: remote.proxy_enabled,
        proxy_port: remote::proxy::ProxyLaunch::port_for(remote.port).unwrap_or_default(),
        proxy_listening,
    }
}

/// 配对：生成设备记录（持久化）并返回 URL（前端渲染二维码）。
#[tauri::command]
pub fn remote_pair(app: AppHandle, name: String) -> Result<String> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(Error::Remote("设备名不能为空".to_owned()));
    }
    let state = app.state::<crate::AppState>();
    let path = crate::paths::settings_path(&app)?;
    let mut settings = state.settings.lock().unwrap();
    let device = RemoteDevice {
        id: remote::gateway::new_device_id(),
        name,
        token: remote::gateway::new_token(),
        created_at: chrono::Local::now().timestamp_millis(),
        revoked: false,
    };
    // 配对 URL 携带一次性短期 code（docs/18 A4），不再直接暴露长期 token：
    // code 5 分钟有效、兑换即消费，长期 token 只落在兑换后的 cookie 里。
    let code = remote::gateway::global_pair_codes().issue(&device.token);
    let url =
        remote::gateway::pair_url_with_code(&settings.remote.bind_ip, settings.remote.port, &code);
    settings.remote.devices.push(device);
    crate::settings::save(&path, &settings)?;
    drop(settings);
    // 新 token 必须进网关快照：指纹变化 → sync 停旧起新（即时可配对）。
    tauri::async_runtime::spawn(sync(app));
    Ok(url)
}

/// 吊销：设备 token 失效。正在使用的 cookie 在网关下次重启后彻底断
/// （语义：吊销后点「应用」触发网关重建即全断，见架构 §4.5）。
#[tauri::command]
pub fn remote_revoke(app: AppHandle, id: String) -> Result<()> {
    let state = app.state::<crate::AppState>();
    let path = crate::paths::settings_path(&app)?;
    let mut settings = state.settings.lock().unwrap();
    match settings.remote.devices.iter_mut().find(|d| d.id == id) {
        Some(device) => device.revoked = true,
        None => return Err(Error::Remote(format!("设备不存在：{id}"))),
    }
    crate::settings::save(&path, &settings)?;
    drop(settings);
    // 吊销即时生效：指纹变化 → sync 重建网关 → 旧 cookie 全断（R1）。
    tauri::async_runtime::spawn(sync(app));
    Ok(())
}

/// 自检结果：「能不能用」从猜测变成一眼可见（设计 §5.2）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfCheck {
    pub ok: bool,
    pub detail: String,
    pub latency_ms: u64,
}

/// 自检：本机带真实设备 token 请求网关 `/qx-gate`，302 + cookie = 健康。
#[tauri::command]
pub async fn remote_self_check(app: AppHandle) -> SelfCheck {
    let state = app.state::<crate::AppState>();
    let (addr, token) = {
        let running = state
            .remote
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            running
                .as_ref()
                .and_then(|handle| handle.lan_addr.borrow().map(|a| a.to_string())),
            settings
                .remote
                .devices
                .iter()
                .find(|d| !d.revoked)
                .map(|d| d.token.clone()),
        )
    };
    let Some(addr) = addr else {
        return SelfCheck {
            ok: false,
            detail: "网关未监听：先启用远程并选择网卡".to_owned(),
            latency_ms: 0,
        };
    };
    let Some(token) = token else {
        return SelfCheck {
            ok: false,
            detail: "无已配对设备：先配对一台设备再自检".to_owned(),
            latency_ms: 0,
        };
    };

    let url = format!("http://{addr}/qx-gate?token={token}");
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(cause) => {
            return SelfCheck {
                ok: false,
                detail: format!("HTTP 客户端构建失败：{cause}"),
                latency_ms: 0,
            }
        }
    };
    let started = std::time::Instant::now();
    let latency = || started.elapsed().as_millis() as u64;
    match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(response) => {
            let healthy = response.status() == reqwest::StatusCode::FOUND
                && response.headers().contains_key(reqwest::header::SET_COOKIE);
            if healthy {
                SelfCheck {
                    ok: true,
                    detail: format!("网关工作正常（{}ms）", latency()),
                    latency_ms: latency(),
                }
            } else {
                SelfCheck {
                    ok: false,
                    detail: format!("异常响应 {}（{}ms）", response.status(), latency()),
                    latency_ms: latency(),
                }
            }
        }
        Err(cause) => SelfCheck {
            ok: false,
            detail: format!("请求失败：{cause}"),
            latency_ms: latency(),
        },
    }
}

/// 启动配置指纹：enabled/bind/port/devices/upstream/dsh_url/反代开关/
/// 内网目标开关 任一变化都会改变它。sync 用它决定「保留运行中的网关」
/// 还是「停旧起新」——设备表是网关启动快照，不重启就进不去新配对的
/// token、也断不掉已吊销的（R1 修复）；upstream 必须参与：启动早于 DSH
/// 就绪时上游是占位 0 端口，就绪事件再 sync 时指纹必须变化才会重建
/// （否则网关永远指向上游 0——实测复现）。dsh_url（DSH 启动 token）参与：
/// DSH revive 后 token 变化时也重建网关，保证 `Upstream` 始终持有最新的
/// 换 cookie 入口（cookie 本身因签名密钥持久化多半仍有效，但重兑路径
/// 必须用新 token）。参数即全部指纹维度，哈希函数天然多参，允许超参。
#[allow(clippy::too_many_arguments)]
fn fingerprint(
    enabled: bool,
    bind_ip: &str,
    port: u16,
    devices: &[RemoteDevice],
    upstream: Option<&str>,
    dsh_url: Option<&str>,
    dsh_cookie: Option<&str>,
    proxy_enabled: bool,
    proxy_allow_lan: bool,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    enabled.hash(&mut hasher);
    bind_ip.hash(&mut hasher);
    port.hash(&mut hasher);
    devices.hash(&mut hasher);
    upstream.hash(&mut hasher);
    dsh_url.hash(&mut hasher);
    // 共享实例的 cookie 是自签的：换实例/重签都要重建网关。
    dsh_cookie.hash(&mut hasher);
    proxy_enabled.hash(&mut hasher);
    proxy_allow_lan.hash(&mut hasher);
    hasher.finish()
}

/// 设置变更后的同步入口：指纹一致且任务存活 → 保留；否则停旧起新。
/// DSH origin 由 supervisor 状态提供；DSH 未跑时网关照常监听
/// （请求会得到「上游不可达」提示），DSH 起来即通。
///
/// **回环永远在**——即使远程未启用，DSH 页 iframe 也要工作。LAN 仅在
/// `enabled && !bind_ip.is_empty()` 时才绑。两条 socket 共享同一 Router。
pub async fn sync(app: AppHandle) {
    // 全程互斥（见 RemoteState::sync_lock 注释）。
    let state = app.state::<crate::AppState>();
    let _serial = state.remote.sync_lock.lock().await;

    let (should_bind_lan, bind_ip, port, devices, upstream, dsh_url, dsh_cookie, proxy_launch) = {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // DSH 就绪后取完整 URL（gateway 替远程设备兑换签名 cookie 时整段打过去，
        // 不再单独抽 token 字段）；upstream 仍只装 host:port，供转发与 WS 桥构造 URL。
        //
        // 共享实例（复用用户自己跑着的 dsh web）没有启动 token 可兑换：cookie
        // 是自签的，dsh_url 置空，避免网关拿回环地址去「兑换」——那是网关自己。
        let (upstream, dsh_url, dsh_cookie) = match state.harness.supervisor.status() {
            crate::harness::supervisor::Status::Ready {
                origin,
                url,
                shared,
                ..
            } => {
                if shared {
                    (Some(origin), None, state.harness.supervisor.shared_cookie())
                } else {
                    (Some(origin), Some(url), None)
                }
            }
            _ => (None, None, None),
        };
        let should_bind_lan = settings.remote.enabled && !settings.remote.bind_ip.is_empty();
        let proxy_launch = remote::proxy::ProxyLaunch {
            enabled: settings.remote.proxy_enabled,
            allow_lan_targets: settings.remote.proxy_allow_lan,
        };
        (
            should_bind_lan,
            settings.remote.bind_ip.clone(),
            settings.remote.port,
            settings.remote.devices.clone(),
            upstream,
            dsh_url,
            dsh_cookie,
            proxy_launch,
        )
    };
    // 回环永远要起；LAN 只在启用时绑——start() 内部据此分支。
    // 注意：传空的 bind_ip 而不是缺省值，让 fingerprint 能反映「是否 LAN」。
    let effective_bind_ip = if should_bind_lan {
        bind_ip.clone()
    } else {
        String::new()
    };
    let desired = fingerprint(
        should_bind_lan,
        &effective_bind_ip,
        port,
        &devices,
        upstream.as_deref(),
        dsh_url.as_deref(),
        dsh_cookie.as_deref(),
        proxy_launch.enabled,
        proxy_launch.allow_lan_targets,
    );

    // 指纹没变且任务健在才保留；否则（含吊销/配对/换网卡换端口/远程开关）重建。
    // std 锁不跨 await：先在锁内取出待停句柄，锁外再 abort+等待。
    let (keep, stale) = {
        let mut running = state
            .remote
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keep = running
            .as_ref()
            .is_some_and(|handle| !handle.task.is_finished() && handle.fingerprint == desired);
        (keep, if keep { None } else { running.take() })
    };
    if let Some(handle) = stale {
        let _ = handle.shutdown.send(true);
        let task = handle.task;
        task.abort();
        // 等旧任务真正结束（监听器随之释放）再绑新端口——
        // abort 只是「请求取消」，不等它就会 10048 竞态（实测）。
        let _ = task.await;
    }
    if keep {
        return;
    }
    // 上游未知（DSH 未跑/未就绪）：网关仍然监听（等 DSH），
    // 上游地址用占位，DSH 就绪事件会再触发 sync 重建。
    let upstream = upstream.unwrap_or_else(|| "127.0.0.1:0".to_owned());
    // 移动定制层目录（ADR-009 的 DSH_HOME 下，与 dsh-mobile 的惯例同址）；
    // 定位失败只降级定制层（临时目录），不拦网关。
    let mobile_access_dir = match crate::paths::dsh_home(&app) {
        Ok(home) => home.join("mobile-access"),
        Err(cause) => {
            crate::logging::log(
                "warn",
                &format!("DSH home 定位失败，定制层退回临时目录：{cause}"),
            );
            std::env::temp_dir().join("qianxun-mobile-access")
        }
    };
    // 重建必落日志（含指纹输入）：连续两次重建对比即可定位哪个字段
    // 在翻转——网关每重建一次就杀光全部 SSE/WS 连接，抖动必须可见。
    crate::logging::log(
        "info",
        &format!(
            "网关重建：lan={should_bind_lan} bind={effective_bind_ip}:{port} 设备={} 上游={upstream} dsh_url={} 反代=开({})/内网目标({})",
            devices.len(),
            dsh_url.is_some(),
            proxy_launch.enabled,
            proxy_launch.allow_lan_targets,
        ),
    );
    match remote::gateway::start(
        &effective_bind_ip,
        port,
        upstream.clone(),
        dsh_url,
        dsh_cookie,
        devices,
        desired,
        mobile_access_dir,
        proxy_launch,
    )
    .await
    {
        Ok(handle) => {
            let log = match (*handle.lan_addr.borrow(), *handle.proxy_addr.borrow()) {
                (Some(lan), Some(proxy)) => format!(
                    "回环 {} + 局域网 {lan} + 反代 {proxy}（上游 {upstream}）",
                    handle.loopback_addr
                ),
                (Some(lan), None) => format!(
                    "回环 {} + 局域网 {lan}（上游 {upstream}）",
                    handle.loopback_addr
                ),
                (None, _) => format!("仅回环 {}（上游 {upstream}）", handle.loopback_addr),
            };
            *state
                .remote
                .running
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
            crate::logging::log("info", &format!("网关监听 {log}"));
        }
        Err(cause) => {
            crate::logging::log("warn", &format!("网关启动失败：{cause}"));
        }
    }
    // 服务端隧道客户端：独立指纹（docs/19 v5.2 §12.3）——配置变化只重连
    // 隧道，不牵连网关；未配置时内部自停。
    remote::online::sync(&app);
}

// ---- 服务端域 IPC（docs/19 v5.2 §12.3）----

/// 服务端连接状态（远程页「服务器」区块）。
#[tauri::command]
pub fn server_status(app: AppHandle) -> remote::online::OnlineStatus {
    remote::online::status(&app)
}

/// 配置服务端连接（url/实例凭据）。写入 settings 后由 sync 重连。
/// 校验：非空时 url 必须是 ws:// 或 wss:// 开头且带 host。
#[tauri::command]
pub async fn server_configure(
    app: AppHandle,
    url: String,
    instance_id: String,
    instance_token: String,
) -> Result<()> {
    let url = url.trim().to_owned();
    let instance_id = instance_id.trim().to_owned();
    let instance_token = instance_token.trim().to_owned();
    let empty = url.is_empty() && instance_id.is_empty() && instance_token.is_empty();
    if !empty {
        if !(url.starts_with("ws://") || url.starts_with("wss://")) {
            return Err(Error::Remote("地址必须以 ws:// 或 wss:// 开头".into()));
        }
        if remote::online::expected_host(&url).is_none() {
            return Err(Error::Remote("地址缺少主机名".into()));
        }
        if instance_id.is_empty() || instance_token.is_empty() {
            return Err(Error::Remote("实例 ID 与凭据需同时填写".into()));
        }
    }
    crate::settings::update(&app, |settings| {
        settings.server = remote::ServerSettings {
            url,
            instance_id,
            instance_token,
        };
    })?;
    sync(app).await;
    Ok(())
}

/// 申请一次性配对码（经隧道向服务端请求）。
#[tauri::command]
pub async fn server_pair_create(app: AppHandle) -> Result<remote::online::PairOffer> {
    remote::online::request_pair_code(&app)
        .await
        .map_err(Error::Remote)
}

/// 服务端设备清单（fresh 优先，失败退缓存）。
#[tauri::command]
pub async fn server_device_list(app: AppHandle) -> Vec<qx_server::protocol::DeviceInfo> {
    remote::online::device_list(&app).await
}

/// 吊销服务端设备。
#[tauri::command]
pub async fn server_device_revoke(app: AppHandle, device_id: String) -> Result<()> {
    remote::online::revoke_device(&app, &device_id)
        .await
        .map_err(Error::Remote)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无 token 鉴权拒绝、配对入口发 cookie：直接驱动 handler 层之上的
    /// 纯函数。完整 HTTP 栈留给真机验收（R1 清单）。
    #[test]
    fn 配对url与token形状() {
        let token = remote::gateway::new_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
        // 配对 URL 只带一次性 code，不带长期 token（docs/18 A4）。
        let code = remote::gateway::global_pair_codes().issue(&token);
        let url = remote::gateway::pair_url_with_code("10.144.3.2", 17400, &code);
        assert_eq!(url, format!("http://10.144.3.2:17400/qx-gate?code={code}"));
        assert!(!url.contains(&token), "配对 URL 不得内嵌长期 token");
        let id = remote::gateway::new_device_id();
        assert!(id.starts_with("dev-"));
    }

    #[test]
    fn 指纹随任一配置维度变化() {
        let device = RemoteDevice {
            id: "d".to_owned(),
            name: "n".to_owned(),
            token: "t".repeat(64),
            created_at: 0,
            revoked: false,
        };
        let fp = |enabled: bool,
                  bind: &str,
                  port: u16,
                  devices: &[RemoteDevice],
                  upstream: Option<&str>,
                  dsh_url: Option<&str>,
                  proxy: bool,
                  proxy_lan: bool| {
            fingerprint(
                enabled, bind, port, devices, upstream, dsh_url, None, proxy, proxy_lan,
            )
        };
        let base = fp(
            true,
            "10.0.0.1",
            17400,
            std::slice::from_ref(&device),
            None,
            None,
            true,
            false,
        );
        // 配置没变 -> 指纹稳定（keep 的前提）。
        assert_eq!(
            base,
            fp(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                None,
                None,
                true,
                false,
            )
        );
        // 新配对（设备表变化）-> 指纹变化。
        let paired = RemoteDevice {
            created_at: 1,
            ..device.clone()
        };
        assert_ne!(
            base,
            fp(true, "10.0.0.1", 17400, &[paired], None, None, true, false)
        );
        // 吊销 -> 指纹变化。
        let revoked = RemoteDevice {
            revoked: true,
            ..device.clone()
        };
        assert_ne!(
            base,
            fp(true, "10.0.0.1", 17400, &[revoked], None, None, true, false)
        );
        // 换网卡 / 换端口 / 开关 / 上游就绪 -> 指纹变化。
        assert_ne!(
            base,
            fp(
                true,
                "10.0.0.2",
                17400,
                std::slice::from_ref(&device),
                None,
                None,
                true,
                false,
            )
        );
        assert_ne!(
            base,
            fp(
                true,
                "10.0.0.1",
                17401,
                std::slice::from_ref(&device),
                None,
                None,
                true,
                false,
            )
        );
        assert_ne!(
            base,
            fp(
                false,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                None,
                None,
                true,
                false,
            )
        );
        // 上游从占位（None）变为真实端口 -> 指纹变化（DSH 就绪重建的依据）。
        assert_ne!(
            base,
            fp(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                Some("127.0.0.1:17300"),
                None,
                true,
                false,
            )
        );
        // 共享实例的自签 cookie 变化 -> 指纹变化（换实例/重签都要重建网关）。
        assert_ne!(
            fingerprint(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                Some("127.0.0.1:3080"),
                None,
                None,
                true,
                false,
            ),
            fingerprint(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                Some("127.0.0.1:3080"),
                None,
                Some("dsh-auth-x=v1.body.sig"),
                true,
                false,
            )
        );
        // DSH 启动 token（dsh_url）变化 -> 指纹变化（revive 重建的依据）。
        assert_ne!(
            base,
            fp(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                Some("127.0.0.1:17300"),
                Some("http://127.0.0.1:17300/?token=AAA"),
                true,
                false,
            )
        );
        // DSH token 变而 origin 不变 -> 指纹仍变。
        assert_ne!(
            fp(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                Some("127.0.0.1:17300"),
                Some("http://127.0.0.1:17300/?token=AAA"),
                true,
                false,
            ),
            fp(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                Some("127.0.0.1:17300"),
                Some("http://127.0.0.1:17300/?token=BBB"),
                true,
                false,
            )
        );
        // 反代开关 / 内网目标开关 -> 指纹变化（docs/20）。
        assert_ne!(
            base,
            fp(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                None,
                None,
                false,
                false,
            )
        );
        assert_ne!(
            base,
            fp(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                None,
                None,
                true,
                true,
            )
        );
    }

    /// 完整 HTTP 栈集成：上游 mock + 网关真实监听——配对 302 发 cookie、
    /// 带 cookie 转发上游、未带 401、吊销后（重建设备表）旧 cookie 401。
    #[tokio::test]
    async fn 网关配对鉴权与吊销全链路() {
        // 模拟 DSH 上游：唯一路由 / → 200。
        let upstream_app =
            axum::Router::new().route("/", axum::routing::get(|| async { "dsh-ok" }));
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("上游绑定");
        let upstream_addr = upstream_listener.local_addr().expect("上游地址");
        tokio::spawn(async move {
            let _ = axum::serve(upstream_listener, upstream_app).await;
        });

        let device = RemoteDevice {
            id: "dev-test-1".to_owned(),
            name: "测试机".to_owned(),
            token: "ab".repeat(32),
            created_at: 0,
            revoked: false,
        };

        let handle = remote::gateway::start(
            "127.0.0.2",
            0,
            upstream_addr.to_string(),
            None,
            None,
            vec![device.clone()],
            1,
            std::env::temp_dir().join("qianxun-gw-test-mobile-access"),
            remote::proxy::ProxyLaunch::default(),
        )
        .await
        .expect("网关启动");
        // 局域网端（qx_token 鉴权）走 127.0.0.2 listener；回环端（无鉴权）
        // 走 127.0.0.1 listener——本测试只覆盖 LAN 路径。
        let base = format!("http://{}", handle.lan_addr.borrow().expect("LAN 应绑定"));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client");

        // 未带 token → 401。
        let status = client
            .get(format!("{base}/"))
            .send()
            .await
            .expect("请求")
            .status();
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);

        // 配对入口：有效 token → 302 + HttpOnly cookie。
        let pair = client
            .get(format!("{base}/qx-gate?token={}", device.token))
            .send()
            .await
            .expect("配对请求");
        assert_eq!(pair.status(), reqwest::StatusCode::FOUND);
        let cookie = pair
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .expect("应发 cookie")
            .to_str()
            .expect("cookie 文本")
            .to_owned();
        assert!(cookie.starts_with("qx_token="));
        assert!(cookie.contains("HttpOnly"));

        // 带 cookie → 转发上游 200。
        let ok = client
            .get(format!("{base}/"))
            .header(reqwest::header::COOKIE, cookie.clone())
            .send()
            .await
            .expect("转发请求");
        assert_eq!(ok.status(), reqwest::StatusCode::OK);
        assert_eq!(ok.text().await.expect("正文"), "dsh-ok");

        // 无效 token 的配对 → 401。
        let bad = client
            .get(format!("{base}/qx-gate?token={}", "cd".repeat(32)))
            .send()
            .await
            .expect("无效配对")
            .status();
        assert_eq!(bad, reqwest::StatusCode::UNAUTHORIZED);

        let _ = handle.shutdown.send(true);

        // 网关二：同设备已吊销（sync 指纹变化触发重建的等价场景）→ 旧 cookie 401。
        let revoked = RemoteDevice {
            revoked: true,
            ..device
        };
        let handle2 = remote::gateway::start(
            "127.0.0.2",
            0,
            upstream_addr.to_string(),
            None,
            None,
            vec![revoked],
            2,
            std::env::temp_dir().join("qianxun-gw-test-mobile-access"),
            remote::proxy::ProxyLaunch::default(),
        )
        .await
        .expect("网关二启动");
        let base2 = format!("http://{}", handle2.lan_addr.borrow().expect("LAN 应绑定"));
        let status = client
            .get(format!("{base2}/"))
            .header(reqwest::header::COOKIE, cookie)
            .send()
            .await
            .expect("吊销后请求")
            .status();
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
        let _ = handle2.shutdown.send(true);
    }

    /// DSH 0.1.2 浏览器鉴权全链路：网关用启动 token 兑换签名 cookie，
    /// 手机请求经网关携带 cookie 到上游；手机的 Host/Origin/Sec-Fetch
    /// 头不透传（DSH 的 Host/Origin 栅栏只认上游 authority）。
    #[tokio::test]
    async fn 网关兑换cookie并注入转发() {
        use std::sync::Arc;

        const LAUNCH_TOKEN: &str = "tok-launch-0123456789";
        const DSH_COOKIE: &str = "dsh-auth-test=signed-value";

        /// 模拟 DSH：/?token=… 兑换 303+cookie；其余请求要求 cookie，
        /// 并把观察到的 Host/Origin/Sec-Fetch 记下来供断言。
        struct Observed {
            summary: String,
            cookie: String,
            is_exchange: bool,
        }

        /// 同步拆解请求为纯数据：request 在 await 前就消亡，保证 future 是
        /// Send（Body 非 Sync，任何对它的借用跨 await 都会让 Handler 失败）。
        fn observe(request: axum::extract::Request) -> Observed {
            let header = |name: &str| {
                request
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned()
            };
            let cookie = header("cookie");
            let summary = format!(
                "host={}|origin={}|site={}|cookie={}",
                header("host"),
                header("origin"),
                header("sec-fetch-site"),
                cookie,
            );
            let is_exchange = request.method() == axum::http::Method::GET
                && request
                    .uri()
                    .query()
                    .unwrap_or_default()
                    .contains("token=tok-launch");
            Observed {
                summary,
                cookie,
                is_exchange,
            }
        }

        async fn dsh_mock(
            axum::extract::State(seen): axum::extract::State<Arc<tokio::sync::Mutex<Vec<String>>>>,
            request: axum::extract::Request,
        ) -> axum::response::Response {
            let observed = observe(request);
            seen.lock().await.push(observed.summary);
            let response = if observed.is_exchange {
                axum::response::Response::builder()
                    .status(303)
                    .header("set-cookie", format!("{DSH_COOKIE}; Max-Age=60; Path=/"))
                    .body(axum::body::Body::empty())
                    .unwrap()
            } else if observed.cookie == DSH_COOKIE {
                axum::response::Response::builder()
                    .status(200)
                    .body(axum::body::Body::from("dsh-authed"))
                    .unwrap()
            } else {
                axum::response::Response::builder()
                    .status(401)
                    .body(axum::body::Body::from("unauthorized"))
                    .unwrap()
            };
            response
        }

        let seen: Arc<tokio::sync::Mutex<Vec<String>>> = Arc::default();
        let upstream_app = axum::Router::new()
            .route("/", axum::routing::any(dsh_mock))
            .with_state(seen.clone());
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("上游绑定");
        let upstream_addr = upstream_listener.local_addr().expect("上游地址");
        tokio::spawn(async move {
            let _ = axum::serve(upstream_listener, upstream_app).await;
        });

        let device = RemoteDevice {
            id: "dev-test-2".to_owned(),
            name: "测试机".to_owned(),
            token: "ef".repeat(32),
            created_at: 0,
            revoked: false,
        };
        let device_token = device.token.clone();
        let handle = remote::gateway::start(
            "127.0.0.2",
            0,
            upstream_addr.to_string(),
            Some(format!("http://{upstream_addr}/?token={LAUNCH_TOKEN}")),
            None,
            vec![device],
            7,
            std::env::temp_dir().join("qianxun-gw-test-mobile-access"),
            remote::proxy::ProxyLaunch::default(),
        )
        .await
        .expect("网关启动");
        let base = format!("http://{}", handle.lan_addr.borrow().expect("LAN 应绑定"));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client");

        // 手机先配对：/qx-gate?token=… → 302 + qx_token cookie。
        let pair = client
            .get(format!("{base}/qx-gate?token={device_token}"))
            .send()
            .await
            .expect("配对请求");
        assert_eq!(pair.status(), reqwest::StatusCode::FOUND);
        let qx_cookie = pair
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next().map(str::to_owned))
            .expect("应下发 qx_token cookie");

        // 手机请求：带的 origin/sec-fetch-site 是网关 LAN authority，网关
        // 必须**改写**为上游 DSH 同源（不再裸剥），这样 dshmarket 等插件
        // 的 same-origin 守卫才能放行。改写规则：Origin 重写为
        // http://<dsh_origin>，Sec-Fetch-Site 强制 same-origin。
        let response = client
            .get(format!("{base}/"))
            .header("origin", base.clone())
            .header("sec-fetch-site", "cross-site")
            .header(reqwest::header::COOKIE, qx_cookie)
            .send()
            .await
            .expect("转发请求");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.expect("正文"), "dsh-authed");

        // 第一次 = 网关兑换（无 origin，Host 是上游 authority）；
        // 第二次 = 手机请求转发（cookie 由网关换为服务端持有的 dsh-auth；
        // origin/sec-fetch-site 都被改写为与上游同源）。
        let entries = seen.lock().await.clone();
        assert_eq!(entries.len(), 2, "实际观察到的上游请求：{entries:?}");
        let authority = format!("host={upstream_addr}");
        assert!(
            entries[0].starts_with(&authority) && entries[0].contains("cookie="),
            "兑换请求形态异常：{}",
            entries[0]
        );
        let upstream_origin = format!("origin=http://{upstream_addr}");
        assert!(
            entries[1].starts_with(&authority)
                // 改写后的 Origin 必须与上游 DSH authority 相同（不能透传
                // 手机带来的网关 LAN authority）。
                && entries[1].contains(&upstream_origin)
                // Sec-Fetch-Site 必须强制为 same-origin。
                && entries[1].contains("site=same-origin")
                // 手机带过来的 cookie 不许到上游：调用方 qx_token 永远留在网关。
                && entries[1].ends_with(&format!("cookie={DSH_COOKIE}")),
            "转发请求形态异常：{}",
            entries[1]
        );
        let _ = handle.shutdown.send(true);
    }
}

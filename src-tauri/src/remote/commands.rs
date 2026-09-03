//! 远程域 IPC：网卡枚举 / 状态 / 配对 / 吊销 / 随设置同步启停。

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::remote::{self, gateway::GatewayHandle, RemoteDevice};

/// 网关运行态：Some = 监听中。锁内持 JoinHandle，停机经 shutdown 通道。
/// `sync_lock`：sync 全程互斥——DSH 就绪与设置保存可能并发触发两次
/// sync，并发重建会在「旧监听器尚未释放」时抢绑同一端口（实测 10048）。
#[derive(Default)]
pub struct RemoteState {
    pub running: Mutex<Option<GatewayHandle>>,
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
        .map(|handle| handle.local_addr.to_string());
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
    let url = remote::gateway::pair_url(
        &settings.remote.bind_ip,
        settings.remote.port,
        &device.token,
    );
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
            running.as_ref().map(|handle| handle.local_addr.to_string()),
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

/// 启动配置指纹：enabled/bind/port/devices/upstream 任一变化都会改变它。
/// sync 用它决定「保留运行中的网关」还是「停旧起新」——设备表是网关
/// 启动快照，不重启就进不去新配对的 token、也断不掉已吊销的（R1 修复）；
/// upstream 必须参与：启动早于 DSH 就绪时上游是占位 0 端口，就绪事件
/// 再 sync 时指纹必须变化才会重建（否则网关永远指向上游 0——实测复现）。
fn fingerprint(
    enabled: bool,
    bind_ip: &str,
    port: u16,
    devices: &[RemoteDevice],
    upstream: Option<&str>,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    enabled.hash(&mut hasher);
    bind_ip.hash(&mut hasher);
    port.hash(&mut hasher);
    devices.hash(&mut hasher);
    upstream.hash(&mut hasher);
    hasher.finish()
}

/// 设置变更后的同步入口：指纹一致且任务存活 → 保留；否则停旧起新。
/// DSH origin 由 supervisor 状态提供；DSH 未跑时网关照常监听
/// （请求会得到「上游不可达」提示），DSH 起来即通。
pub async fn sync(app: AppHandle) {
    // 全程互斥（见 RemoteState::sync_lock 注释）。
    let state = app.state::<crate::AppState>();
    let _serial = state.remote.sync_lock.lock().await;

    let (should_run, bind_ip, port, devices, upstream) = {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let origin = match state.harness.supervisor.status() {
            crate::harness::supervisor::Status::Ready { origin, .. } => Some(origin),
            _ => None,
        };
        (
            settings.remote.enabled && !settings.remote.bind_ip.is_empty(),
            settings.remote.bind_ip.clone(),
            settings.remote.port,
            settings.remote.devices.clone(),
            origin,
        )
    };
    let desired = fingerprint(should_run, &bind_ip, port, &devices, upstream.as_deref());

    // 指纹没变且任务健在才保留；否则（含吊销/配对/换网卡换端口）重建网关。
    // std 锁不跨 await：先在锁内取出待停句柄，锁外再 abort+等待。
    let (keep, stale) = {
        let mut running = state
            .remote
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keep = should_run
            && running
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
    if keep || !should_run {
        return;
    }
    // 上游未知（DSH 未跑/未就绪）：网关仍然监听（等 DSH），
    // 上游地址用占位，DSH 就绪事件会再触发 sync 重建。
    let upstream = upstream.unwrap_or_else(|| "127.0.0.1:0".to_owned());
    match remote::gateway::start(&bind_ip, port, upstream.clone(), devices, desired).await {
        Ok(handle) => {
            let addr = handle.local_addr.to_string();
            let state = app.state::<crate::AppState>();
            *state
                .remote
                .running
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
            crate::logging::log("info", &format!("远程网关监听 {addr}（上游 {upstream}）"));
        }
        Err(cause) => {
            crate::logging::log("warn", &format!("远程网关启动失败：{cause}"));
        }
    }
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
        let url = remote::gateway::pair_url("10.144.3.2", 17400, &token);
        assert_eq!(
            url,
            format!("http://10.144.3.2:17400/qx-gate?token={token}")
        );
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
        let base = fingerprint(true, "10.0.0.1", 17400, std::slice::from_ref(&device), None);
        // 配置没变 → 指纹稳定（keep 的前提）。
        assert_eq!(
            base,
            fingerprint(true, "10.0.0.1", 17400, std::slice::from_ref(&device), None)
        );
        // 新配对（设备表变化）→ 指纹变化。
        let paired = RemoteDevice {
            created_at: 1,
            ..device.clone()
        };
        assert_ne!(base, fingerprint(true, "10.0.0.1", 17400, &[paired], None));
        // 吊销 → 指纹变化。
        let revoked = RemoteDevice {
            revoked: true,
            ..device.clone()
        };
        assert_ne!(base, fingerprint(true, "10.0.0.1", 17400, &[revoked], None));
        // 换网卡 / 换端口 / 开关 / 上游就绪 → 指纹变化。
        assert_ne!(
            base,
            fingerprint(true, "10.0.0.2", 17400, std::slice::from_ref(&device), None)
        );
        assert_ne!(
            base,
            fingerprint(true, "10.0.0.1", 17401, std::slice::from_ref(&device), None)
        );
        assert_ne!(
            base,
            fingerprint(
                false,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                None
            )
        );
        // 上游从占位（None）变为真实端口 → 指纹变化（DSH 就绪重建的依据）。
        assert_ne!(
            base,
            fingerprint(
                true,
                "10.0.0.1",
                17400,
                std::slice::from_ref(&device),
                Some("127.0.0.1:17300")
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
            "127.0.0.1",
            0,
            upstream_addr.to_string(),
            vec![device.clone()],
            1,
        )
        .await
        .expect("网关启动");
        let base = format!("http://{}", handle.local_addr);
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
        let handle2 =
            remote::gateway::start("127.0.0.1", 0, upstream_addr.to_string(), vec![revoked], 2)
                .await
                .expect("网关二启动");
        let base2 = format!("http://{}", handle2.local_addr);
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
}

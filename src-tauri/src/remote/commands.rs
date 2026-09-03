//! 远程域 IPC：网卡枚举 / 状态 / 配对 / 吊销 / 随设置同步启停。

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::remote::{self, gateway::GatewayHandle, RemoteDevice};

/// 网关运行态：Some = 监听中。锁内持 JoinHandle，停机经 shutdown 通道。
#[derive(Default)]
pub struct RemoteState {
    pub running: Mutex<Option<GatewayHandle>>,
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
    Ok(())
}

/// 设置变更后的同步入口：enabled/bind/port/设备表任一变化 → 重启网关。
/// DSH origin 由 supervisor 状态提供；DSH 未跑时网关照常监听
/// （请求会得到「上游不可达」提示），DSH 起来即通。
pub async fn sync(app: AppHandle) {
    let (should_run, bind_ip, port, devices, upstream) = {
        let state = app.state::<crate::AppState>();
        let settings = state.settings.lock().unwrap();
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

    // 停掉现有的（除非配置完全没变且仍在跑）。
    {
        let state = app.state::<crate::AppState>();
        let mut running = state.remote.running.lock().unwrap();
        let keep = should_run
            && running
                .as_ref()
                .is_some_and(|handle| !handle.task.is_finished());
        if !keep {
            if let Some(handle) = running.take() {
                let _ = handle.shutdown.send(true);
                handle.task.abort();
            }
        } else {
            return; // 网关在跑且无需变化。
        }
    }

    if !should_run {
        return;
    }
    // 上游未知（DSH 未跑/未就绪）：网关仍然监听（等 DSH），
    // 上游地址用占位，DSH 就绪事件会再触发 sync 重建。
    let upstream = upstream.unwrap_or_else(|| "127.0.0.1:0".to_owned());
    match remote::gateway::start(&bind_ip, port, upstream.clone(), devices).await {
        Ok(handle) => {
            let addr = handle.local_addr.to_string();
            let state = app.state::<crate::AppState>();
            *state.remote.running.lock().unwrap() = Some(handle);
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
}

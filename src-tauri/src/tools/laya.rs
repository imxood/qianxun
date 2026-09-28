//! Laya 决策引擎侧车（laya-server.exe）的启动/停止/健康探测。
//!
//! 设计要点：
//! - **外部实例也能识别**：running 以 `/health` 探测为准（用户手动
//!   `Start-Process` 起的实例同样算在跑），自有子进程 PID 仅用于停止。
//! - **只杀自己启动的**：stop 只处理 state 里记录的 PID，外部实例提示
//!   到原处停止（与进程终止判定红线一致）。
//! - **启动是异步就绪**：模型加载秒级（实测 ~9s），spawn 后轮询
//!   health 至多 20s；超时不杀进程（可能仍在加载），返回当前快照。

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::error::{Error, Result};
use crate::settings::Settings;

/// 健康探测超时：回环 + 本地进程，800ms 足够宽裕。
const HEALTH_TIMEOUT_MS: u64 = 800;
/// 启动后等待就绪的上限（模型加载实测 ~9s，留一倍裕量）。
const START_READY_TIMEOUT_MS: u64 = 20_000;

/// state 里记录的自有子进程（应用退出时统一收割）。
#[derive(Default)]
pub struct LayaProcState {
    pub child: Mutex<Option<std::process::Child>>,
}

/// laya_status / laya_start 返回快照。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayaStatus {
    /// /health 探测通过（含外部启动的实例）。
    pub running: bool,
    /// `/health` 返回的模型目录（回显，可能与我们配置不同——外部实例）。
    pub health_checkpoint: Option<String>,
    /// 自有子进程 PID；外部实例为 null。
    pub pid: Option<u32>,
    pub port: u16,
    pub threads: u16,
    /// 生效的 sidecar 路径（探测结果回显）；找不到为 null。
    pub server_exe: Option<String>,
    /// 生效的模型目录（探测结果回显）；找不到为 null。
    pub model_dir: Option<String>,
    /// 人类可读的补充信息（探测失败原因等）。
    pub detail: Option<String>,
}

fn settings_snapshot(app: &AppHandle) -> Result<Settings> {
    Ok(app
        .state::<crate::AppState>()
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone())
}

/// 探测链定位 sidecar：settings 显式路径 → 当前 exe 同级 → 上级 release
/// （开发态 debug/qianxun.exe 与 release/laya-server.exe 并存）→ 资源目录。
fn resolve_server_exe(app: &AppHandle, settings: &Settings) -> Option<PathBuf> {
    let raw = settings.tools.laya.server_exe.trim();
    if !raw.is_empty() {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Some(path);
        }
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let mut candidates = Vec::new();
    if let Some(dir) = &exe_dir {
        candidates.push(dir.join("laya-server.exe"));
        candidates.push(dir.join("../release/laya-server.exe"));
    }
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("laya-server.exe"));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// 探测链定位模型目录：settings 显式路径 → exe 同级 models → 上级 models。
fn resolve_model_dir(settings: &Settings) -> Option<PathBuf> {
    let raw = settings.tools.laya.model_dir.trim();
    if !raw.is_empty() {
        let path = PathBuf::from(raw);
        if path.is_dir() {
            return Some(path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [
                dir.join("models/laya-multilingual-onnx"),
                dir.join("../models/laya-multilingual-onnx"),
            ] {
                if candidate.is_dir() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// GET /health（自带超时；手写最小 HTTP）。connect 用显式超时——回环
/// 端口未监听时立即失败；系统栈默认 SYN 重传会拖到秒级，同步命令跑在
/// 主线程时表现为整页周期性卡顿（实测教训）。
fn probe_health(port: u16) -> (bool, Option<String>) {
    use std::net::SocketAddr;
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream =
        match TcpStream::connect_timeout(&addr, Duration::from_millis(HEALTH_TIMEOUT_MS)) {
            Ok(stream) => stream,
            Err(_) => return (false, None),
        };
    stream
        .set_read_timeout(Some(Duration::from_millis(HEALTH_TIMEOUT_MS)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_millis(HEALTH_TIMEOUT_MS)))
        .ok();
    use std::io::{Read, Write};
    if stream
        .write_all(
            format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .is_err()
    {
        return (false, None);
    }
    let mut buffer = Vec::new();
    if stream.read_to_end(&mut buffer).is_err() {
        return (false, None);
    }
    let text = String::from_utf8_lossy(&buffer);
    let ok = text.starts_with("HTTP/1.1 200") || text.starts_with("HTTP/1.0 200");
    let checkpoint = text
        .split("\"checkpoint\"")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(1))
        .map(str::to_owned);
    (ok, checkpoint)
}

fn build_status(
    app: &AppHandle,
    settings: &Settings,
    pid: Option<u32>,
    detail: Option<String>,
) -> LayaStatus {
    let port = settings.tools.laya.port;
    let (running, checkpoint) = probe_health(port);
    LayaStatus {
        running,
        health_checkpoint: checkpoint,
        pid,
        port,
        threads: settings.tools.laya.threads,
        server_exe: resolve_server_exe(app, settings)
            .map(|path| path.to_string_lossy().into_owned()),
        model_dir: resolve_model_dir(settings).map(|path| path.to_string_lossy().into_owned()),
        detail,
    }
}

/// 快照当前 Laya 环境状态（幂等，可轮询）。
/// async + spawn_blocking：探测含网络等待，**绝不能**跑在主线程——
/// 同步命令会周期性卡死整个 UI（实测教训）。
#[tauri::command]
pub async fn laya_status(app: AppHandle) -> Result<LayaStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = settings_snapshot(&app)?;
        let pid = app.try_state::<LayaProcState>().and_then(|state| {
            state
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(|child| child.id())
        });
        Ok(build_status(&app, &settings, pid, None))
    })
    .await
    .map_err(|error| Error::Spawn(format!("探测任务失败：{error}")))?
}

/// 启动侧车（幂等：已在跑则直接返回快照）。阻塞至健康或超时。
/// async + spawn_blocking：内部含 spawn 与最长 20s 的就绪轮询。
#[tauri::command]
pub async fn laya_start(app: AppHandle) -> Result<LayaStatus> {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LayaProcState>();
        laya_start_blocking(&app, &state)
    });
    task.await
        .map_err(|error| Error::Spawn(format!("启动任务失败：{error}")))?
}

fn laya_start_blocking(app: &AppHandle, state: &State<'_, LayaProcState>) -> Result<LayaStatus> {
    let settings = settings_snapshot(app)?;
    let port = settings.tools.laya.port;
    let (running, checkpoint) = probe_health(port);
    if running {
        return Ok(build_status(
            app,
            &settings,
            None,
            Some("已有实例在运行（可能为外部启动），直接复用".to_owned()),
        ));
    }
    let _ = checkpoint;

    let exe = resolve_server_exe(app, &settings).ok_or_else(|| {
        Error::SettingsInvalid("找不到 laya-server.exe：请在下方配置可执行文件路径".to_owned())
    })?;
    let model_dir = resolve_model_dir(&settings).ok_or_else(|| {
        Error::SettingsInvalid(
            "找不到模型目录：请在下方配置模型目录（含 laya.onnx 的 bundle）".to_owned(),
        )
    })?;

    let log_dir = crate::paths::log_path(app)?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("logs"));
    std::fs::create_dir_all(&log_dir)
        .map_err(|error| Error::Spawn(format!("日志目录创建失败：{error}")))?;
    let (stdout_path, stderr_path) = {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        (
            log_dir.join(format!("laya-server-{stamp}.out.log")),
            log_dir.join(format!("laya-server-{stamp}.err.log")),
        )
    };
    let stdout_file = std::fs::File::create(&stdout_path)
        .map_err(|error| Error::Spawn(format!("日志文件创建失败：{error}")))?;
    let stderr_file = std::fs::File::create(&stderr_path)
        .map_err(|error| Error::Spawn(format!("日志文件创建失败：{error}")))?;

    let mut command = std::process::Command::new(&exe);
    command
        .arg("--model-dir")
        .arg(&model_dir)
        .arg("--port")
        .arg(port.to_string())
        .arg("--threads")
        .arg(settings.tools.laya.threads.to_string())
        .arg("--log-dir")
        .arg(log_dir.join("laya-sessions"))
        // 输出走管道：中继线程逐行落文件 + 推 `env://log`（环境页实时日志）。
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    super::suppress_console_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| Error::Spawn(format!("侧车启动失败：{error}")))?;
    if let Some(pipe) = child.stdout.take() {
        super::relay_child_output(app, "laya", "stdout", pipe, stdout_file);
    }
    if let Some(pipe) = child.stderr.take() {
        super::relay_child_output(app, "laya", "stderr", pipe, stderr_file);
    }
    *state
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(child);

    // 轮询就绪：模型加载秒级，超时不杀（可能仍在加载，status 可复查）。
    let deadline = std::time::Instant::now() + Duration::from_millis(START_READY_TIMEOUT_MS);
    loop {
        std::thread::sleep(Duration::from_millis(400));
        let (running, checkpoint) = probe_health(port);
        if running {
            return Ok(LayaStatus {
                running,
                health_checkpoint: checkpoint,
                pid: state
                    .child
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref()
                    .map(std::process::Child::id),
                port,
                threads: settings.tools.laya.threads,
                server_exe: Some(exe.to_string_lossy().into_owned()),
                model_dir: Some(model_dir.to_string_lossy().into_owned()),
                detail: None,
            });
        }
        if std::time::Instant::now() >= deadline {
            return Ok(build_status(
                app,
                &settings,
                state
                    .child
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref()
                    .map(std::process::Child::id),
                Some("启动后 20s 内未就绪：进程可能仍在加载，或已异常退出（见日志）".to_owned()),
            ));
        }
    }
}

/// 停止自有子进程（外部启动的实例不动，返回提示）。kill+wait 有阻塞
/// 等待，async + spawn_blocking 保 UI 流畅。
#[tauri::command]
pub async fn laya_stop(app: AppHandle) -> Result<LayaStatus> {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LayaProcState>();
        laya_stop_blocking(&app, &state)
    });
    task.await
        .map_err(|error| Error::Spawn(format!("停止任务失败：{error}")))?
}

fn laya_stop_blocking(app: &AppHandle, state: &State<'_, LayaProcState>) -> Result<LayaStatus> {
    let settings = settings_snapshot(app)?;
    let taken = state
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    match taken {
        Some(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        None => {
            return Ok(build_status(
                app,
                &settings,
                None,
                Some("没有由千寻启动的实例；外部启动的进程请在原处停止".to_owned()),
            ))
        }
    }
    Ok(build_status(app, &settings, None, None))
}

/// 应用退出收割自有子进程（RunEvent 钩子调用）。
pub fn kill_owned(app: &AppHandle) {
    if let Some(state) = app.try_state::<LayaProcState>() {
        if let Some(mut child) = state
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

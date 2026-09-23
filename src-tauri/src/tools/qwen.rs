//! QWen 本地推理服务（llama-server 等 OpenAI 兼容后端）的启停管理。
//!
//! 与 laya 侧车同款约定：
//! - running 以端口健康探测为准（外部启动的实例同样识别）；
//! - stop 只处理千寻自己启动的子进程（进程终止判定红线）；
//! - 典型形态是 llama-server.exe：依赖同目录 CUDA DLL，spawn 时
//!   工作目录强制为 exe 所在目录。

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::error::{Error, Result};
use crate::settings::Settings;

/// TCP 探活超时。
const HEALTH_TIMEOUT_MS: u64 = 800;
/// 启动后等待就绪的上限（大模型载入可达数十秒，给足裕量）。
const START_READY_TIMEOUT_MS: u64 = 120_000;

/// state 里记录的自有子进程。
#[derive(Default)]
pub struct QwenProcState {
    pub child: Mutex<Option<std::process::Child>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QwenStatus {
    /// 端口有监听者（含外部启动的实例）。
    pub running: bool,
    /// /health 明确返回 200（llama-server 加载中会 503）。
    pub ready: bool,
    /// 自有子进程 PID；外部实例为 null。
    pub pid: Option<u32>,
    pub port: u16,
    /// 生效的可执行路径；未配置/不存在为 null。
    pub server_exe: Option<String>,
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

fn resolve_server_exe(settings: &Settings) -> Option<PathBuf> {
    let raw = settings.tools.qwen.server_exe.trim();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    path.is_file().then_some(path)
}

/// 探活：先 TCP（端口有监听者），再 GET /health 看 ready。
/// connect 用显式超时——回环端口未监听时立即失败；系统栈默认 SYN
/// 重传会拖到秒级，同步命令跑在主线程时表现为整页周期性卡顿（实测教训）。
fn probe(port: u16) -> (bool, bool) {
    use std::net::SocketAddr;
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(HEALTH_TIMEOUT_MS));
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(_) => return (false, false),
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
        return (true, false);
    }
    let mut buffer = Vec::new();
    if stream.read_to_end(&mut buffer).is_err() {
        return (true, false);
    }
    let text = String::from_utf8_lossy(&buffer);
    let ready = text.starts_with("HTTP/1.1 200") || text.starts_with("HTTP/1.0 200");
    (true, ready)
}

fn build_status(settings: &Settings, pid: Option<u32>, detail: Option<String>) -> QwenStatus {
    let port = settings.tools.qwen.port;
    let (running, ready) = probe(port);
    QwenStatus {
        running,
        ready,
        pid,
        port,
        server_exe: resolve_server_exe(settings).map(|path| path.to_string_lossy().into_owned()),
        detail,
    }
}

/// 快照 QWen 环境状态。async + spawn_blocking：探测含网络等待，
/// **绝不能**跑在主线程（同步命令会周期性卡死整个 UI，实测教训）。
#[tauri::command]
pub async fn qwen_status(app: AppHandle) -> Result<QwenStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = settings_snapshot(&app)?;
        let pid = app.try_state::<QwenProcState>().and_then(|state| {
            state
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(std::process::Child::id)
        });
        Ok(build_status(&settings, pid, None))
    })
    .await
    .map_err(|error| Error::Spawn(format!("探测任务失败：{error}")))?
}

/// 启动服务（幂等：端口已有监听者则直接返回快照）。阻塞至 ready 或超时。
/// async + spawn_blocking：内部含 spawn 与最长 120s 的就绪轮询。
#[tauri::command]
pub async fn qwen_start(app: AppHandle) -> Result<QwenStatus> {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<QwenProcState>();
        qwen_start_blocking(&app, &state)
    });
    task.await
        .map_err(|error| Error::Spawn(format!("启动任务失败：{error}")))?
}

fn qwen_start_blocking(app: &AppHandle, state: &State<'_, QwenProcState>) -> Result<QwenStatus> {
    let settings = settings_snapshot(app)?;
    let port = settings.tools.qwen.port;
    if probe(port).0 {
        return Ok(build_status(
            &settings,
            None,
            Some("已有实例在运行（可能为外部启动），直接复用".to_owned()),
        ));
    }
    let exe = resolve_server_exe(&settings)
        .ok_or_else(|| Error::SettingsInvalid("未配置服务可执行文件路径".to_owned()))?;

    let log_dir = crate::paths::log_path(app)?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("logs"));
    std::fs::create_dir_all(&log_dir)
        .map_err(|error| Error::Spawn(format!("日志目录创建失败：{error}")))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();

    // 工作目录 = exe 所在目录：llama-server 依赖同目录的 CUDA/whisper DLL。
    let mut command = std::process::Command::new(&exe);
    command.current_dir(exe.parent().unwrap_or(Path::new(".")));
    let mut argv: Vec<String> = settings
        .tools
        .qwen
        .args
        .split_whitespace()
        .map(String::from)
        .collect();
    if !argv
        .iter()
        .any(|token| token == "--port" || token == "--host")
    {
        // 参数未指定端口：追加千寻配置的端口，保证与探活一致。
        argv.push("--port".to_owned());
        argv.push(port.to_string());
    }
    command.args(&argv);
    command
        .stdout(
            std::fs::File::create(log_dir.join(format!("qwen-server-{stamp}.out.log")))
                .map_err(|error| Error::Spawn(format!("日志文件创建失败：{error}")))?,
        )
        .stderr(
            std::fs::File::create(log_dir.join(format!("qwen-server-{stamp}.err.log")))
                .map_err(|error| Error::Spawn(format!("日志文件创建失败：{error}")))?,
        );
    let child = command
        .spawn()
        .map_err(|error| Error::Spawn(format!("服务启动失败：{error}")))?;
    *state
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(child);

    let deadline = std::time::Instant::now() + Duration::from_millis(START_READY_TIMEOUT_MS);
    loop {
        std::thread::sleep(Duration::from_millis(600));
        let (running, ready) = probe(port);
        if ready {
            let pid = state
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(std::process::Child::id);
            return Ok(QwenStatus {
                running,
                ready,
                pid,
                port,
                server_exe: Some(exe.to_string_lossy().into_owned()),
                detail: None,
            });
        }
        if std::time::Instant::now() >= deadline {
            let pid = state
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(std::process::Child::id);
            return Ok(QwenStatus {
                running,
                ready,
                pid,
                port,
                server_exe: Some(exe.to_string_lossy().into_owned()),
                detail: Some("已启动但尚未就绪：模型可能仍在加载（见日志）".to_owned()),
            });
        }
    }
}

/// 停止自有子进程（外部启动的实例不动）。kill+wait 有阻塞等待，
/// async + spawn_blocking 保 UI 流畅。
#[tauri::command]
pub async fn qwen_stop(app: AppHandle) -> Result<QwenStatus> {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<QwenProcState>();
        qwen_stop_blocking(&app, &state)
    });
    task.await
        .map_err(|error| Error::Spawn(format!("停止任务失败：{error}")))?
}

fn qwen_stop_blocking(app: &AppHandle, state: &State<'_, QwenProcState>) -> Result<QwenStatus> {
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
                &settings,
                None,
                Some("没有由千寻启动的实例；外部启动的进程请在原处停止".to_owned()),
            ))
        }
    }
    Ok(build_status(&settings, None, None))
}

/// 应用退出收割自有子进程。
pub fn kill_owned(app: &AppHandle) {
    if let Some(state) = app.try_state::<QwenProcState>() {
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

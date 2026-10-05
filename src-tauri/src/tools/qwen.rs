//! Qwen3.8-Flash-Next 本地模型服务（Strata 引擎）的启停管理。
//!
//! 部署方案（`D:\AI\Strata\部署方案.md`）：程序本体在 `E:\git\ai\Strata`
//! （自带 .venv 与 serve/server.py），模型数据与**规范引擎配置**在
//! `D:\AI\Strata`（E 盘的 strata-iq2_xs.json 是过时副本，不再修改）。
//! 启动方式与官方 `启动Strata.py` 完全一致——用 venv 的 python 跑
//! `serve/server.py --engine strata --config <json> --port <port>`，
//! 仅去掉 `--open`（千寻托管启动不弹浏览器）。
//!
//! 与 laya 侧车同款约定：
//! - running 以端口健康探测为准（外部启动的实例同样识别）；
//! - stop 只处理千寻自己启动的子进程**树**（进程终止判定红线）；
//! - server.py 会再拉起引擎子进程，停止必须整树结束（taskkill /T /F，
//!   与官方启动脚本 kill_tree 同语义），只 kill 父进程会留孤儿引擎。

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
/// 启动后等待就绪的上限：33-39 GB 内存载入需 1-3 分钟，冷启动更久，给足裕量。
const START_READY_TIMEOUT_MS: u64 = 300_000;

/// state 里记录的自有子进程（server.py；引擎是其子树，随树一起结束）。
#[derive(Default)]
pub struct QwenProcState {
    pub child: Mutex<Option<std::process::Child>>,
    /// 启动互斥（docs/18 A5）：并发双启会让先写的子进程失去引用 → 孤儿。
    pub starting: Mutex<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QwenStatus {
    /// 端口有监听者（含外部启动的实例）。
    pub running: bool,
    /// /health 明确返回 200（载入模型期间连接拒绝或非 200）。
    pub ready: bool,
    /// 自有子进程 PID；外部实例为 null。
    pub pid: Option<u32>,
    pub port: u16,
    /// 生效的引擎配置绝对路径；未配置/不存在为 null。
    pub config_json: Option<String>,
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

/// 校验并解析 Strata 三要素：(程序目录, venv python, serve/server.py)。
fn resolve_strata(settings: &Settings) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let raw = settings.tools.qwen.strata_home.trim();
    if raw.is_empty() {
        return Err(Error::SettingsInvalid(
            "未配置 Strata 程序目录（环境页 · 配置）".to_owned(),
        ));
    }
    let home = PathBuf::from(raw);
    if !home.is_dir() {
        return Err(Error::SettingsInvalid(format!(
            "Strata 程序目录不存在：{}",
            home.display()
        )));
    }
    let python = home.join(".venv").join("Scripts").join("python.exe");
    let server_py = home.join("serve").join("server.py");
    for path in [&python, &server_py] {
        if !path.is_file() {
            return Err(Error::SettingsInvalid(format!(
                "Strata 组件缺失：{}（程序目录是否完整安装？）",
                path.display()
            )));
        }
    }
    Ok((home, python, server_py))
}

/// 引擎配置：绝对路径原样；否则相对程序目录解析。空 = 规范配置
/// （`crate::settings::DEFAULT_QWEN_CONFIG`，D:\AI\Strata 下的校准版）。
fn resolve_config(settings: &Settings, home: &Path) -> PathBuf {
    let raw = settings.tools.qwen.config.trim();
    let raw = if raw.is_empty() {
        crate::settings::DEFAULT_QWEN_CONFIG
    } else {
        raw
    };
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
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

/// 已配置且程序目录存在时，返回解析后的引擎配置绝对路径；否则 null。
fn effective_config(settings: &Settings) -> Option<String> {
    let home = PathBuf::from(settings.tools.qwen.strata_home.trim());
    if home.is_dir() {
        Some(
            resolve_config(settings, &home)
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    }
}

fn build_status(settings: &Settings, pid: Option<u32>, detail: Option<String>) -> QwenStatus {
    let port = settings.tools.qwen.port;
    let (running, ready) = probe(port);
    QwenStatus {
        running,
        ready,
        pid,
        port,
        config_json: effective_config(settings),
        detail,
    }
}

/// 快照 Qwen 环境状态。async + spawn_blocking：探测含网络等待，
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
/// async + spawn_blocking：内部含 spawn 与最长 5 分钟的就绪轮询。
#[tauri::command]
pub async fn qwen_start(app: AppHandle) -> Result<QwenStatus> {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<QwenProcState>();
        qwen_start_blocking(&app, &state)
    });
    task.await
        .map_err(|error| Error::Spawn(format!("启动任务失败：{error}")))?
}

/// 启动互斥的 RAII 复位（docs/18 A5），与 laya.rs 同款。
struct StartGuard<'a> {
    flag: &'a Mutex<bool>,
}

impl Drop for StartGuard<'_> {
    fn drop(&mut self) {
        *self.flag.lock().unwrap() = false;
    }
}

fn qwen_start_blocking(app: &AppHandle, state: &State<'_, QwenProcState>) -> Result<QwenStatus> {
    // 启动互斥：并发双启会让先写的子进程失去引用，变成无人收割的孤儿。
    {
        let mut starting = state.starting.lock().unwrap();
        if *starting {
            return Err(Error::Spawn("Qwen 正在启动中，请等它完成再试".to_owned()));
        }
        *starting = true;
    }
    let _guard = StartGuard {
        flag: &state.starting,
    };
    let settings = settings_snapshot(app)?;
    let port = settings.tools.qwen.port;
    if probe(port).0 {
        return Ok(build_status(
            &settings,
            None,
            Some("已有实例在运行（可能为外部启动），直接复用".to_owned()),
        ));
    }
    let (home, python, server_py) = resolve_strata(&settings)?;
    let config = resolve_config(&settings, &home);
    if !config.is_file() {
        return Err(Error::SettingsInvalid(format!(
            "引擎配置不存在：{}",
            config.display()
        )));
    }

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

    // 启动命令与官方 启动Strata.py 一致（仅去掉 --open）；工作目录 = 程序本体。
    let mut command = std::process::Command::new(&python);
    command
        .current_dir(&home)
        .arg(&server_py)
        .arg("--engine")
        .arg("strata")
        .arg("--config")
        .arg(&config)
        .arg("--port")
        .arg(port.to_string());
    // 输出走管道：中继线程逐行落文件 + 推 `env://log`（环境页实时日志）。
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    super::suppress_console_window(&mut command);
    let stdout_file = std::fs::File::create(log_dir.join(format!("qwen-strata-{stamp}.out.log")))
        .map_err(|error| Error::Spawn(format!("日志文件创建失败：{error}")))?;
    let stderr_file = std::fs::File::create(log_dir.join(format!("qwen-strata-{stamp}.err.log")))
        .map_err(|error| Error::Spawn(format!("日志文件创建失败：{error}")))?;
    let mut child = command
        .spawn()
        .map_err(|error| Error::Spawn(format!("服务启动失败：{error}")))?;
    if let Some(pipe) = child.stdout.take() {
        super::relay_child_output(app, "qwen", "stdout", pipe, stdout_file);
    }
    if let Some(pipe) = child.stderr.take() {
        super::relay_child_output(app, "qwen", "stderr", pipe, stderr_file);
    }
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
                config_json: Some(config.to_string_lossy().into_owned()),
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
                config_json: Some(config.to_string_lossy().into_owned()),
                detail: Some(
                    "已启动但尚未就绪：33-39 GB 内存载入需 1-3 分钟（首次更久），详见日志"
                        .to_owned(),
                ),
            });
        }
    }
}

/// 结束自有子进程树：server.py 会再拉起引擎子进程，仅 kill 父进程会留
/// 孤儿引擎，必须整树结束（与官方启动脚本 kill_tree 同款 taskkill /T /F
/// 语义；只作用于千寻自己 spawn 的那棵树，符合进程终止判定红线）。
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let mut taskkill = std::process::Command::new("taskkill");
        taskkill
            .args(["/T", "/F", "/PID"])
            .arg(child.id().to_string());
        super::suppress_console_window(&mut taskkill);
        let _ = taskkill.output();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// 停止自有子进程树（外部启动的实例不动）。taskkill+wait 有阻塞等待，
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
        Some(mut child) => kill_tree(&mut child),
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

/// 应用退出收割自有子进程树。
pub fn kill_owned(app: &AppHandle) {
    if let Some(state) = app.try_state::<QwenProcState>() {
        if let Some(mut child) = state
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            kill_tree(&mut child);
        }
    }
}

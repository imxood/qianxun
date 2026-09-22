//! Moli 无头浏览器检测（R001 D8/D10）。
//!
//! 三源解析：settings 自备 binaryPath → 受管安装（tools/moli/moli.exe）
//! → 系统 PATH。probe_version 真实拉起 --version（超时保护），输出形如
//! "moli 1.1.9"，解析出版本号；拉不起来 = 未安装。
//! moli_install 的下载/校验流水线随后续里程碑接入，检测先行。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;

use crate::error::Result;
use tauri::Manager;

/// 版本探测超时：本地进程，3s 足够（实测秒级返回）。
const PROBE_TIMEOUT_MS: u64 = 3_000;

/// moli_status 返回（R001 契约）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MoliStatus {
    /// 三源任一命中且版本探测成功。
    pub installed: bool,
    /// --version 解析出的版本（如 "1.1.9"）；未安装为 null。
    pub version: Option<String>,
    /// 命中的二进制绝对路径；未安装为 null。
    pub path: Option<String>,
    /// 命中来源。
    pub source: MoliSource,
}

/// 检测来源：custom = settings 自备；managed = 受管目录；path = 系统 PATH。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MoliSource {
    Custom,
    Managed,
    Path,
    None,
}

/// 平台可执行名。
fn moli_exe_name() -> &'static str {
    if cfg!(windows) {
        "moli.exe"
    } else {
        "moli"
    }
}

/// 受管安装目录下的二进制位置。
fn managed_binary(app: &tauri::AppHandle) -> Result<PathBuf> {
    Ok(crate::paths::tools_dir(app)?
        .join("moli")
        .join(moli_exe_name()))
}

/// 自备路径有效性：非空且是文件。
fn custom_binary(settings: &crate::settings::Settings) -> Option<PathBuf> {
    let raw = settings.tools.moli.binary_path.trim();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    path.is_file().then_some(path)
}

/// 系统 PATH 查找 moli 可执行文件。
fn find_in_path() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exe = moli_exe_name();
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

/// 三源解析（不含版本探测）。
pub fn resolve_binary(
    app: &tauri::AppHandle,
    settings: &crate::settings::Settings,
) -> Option<(PathBuf, MoliSource)> {
    if let Some(path) = custom_binary(settings) {
        return Some((path, MoliSource::Custom));
    }
    if let Ok(managed) = managed_binary(app) {
        if managed.is_file() {
            return Some((managed, MoliSource::Managed));
        }
    }
    find_in_path().map(|path| (path, MoliSource::Path))
}

/// 拉起 --version 并解析版本号；.bat/.cmd 走 cmd /C（Windows 批处理
/// 不能直接 CreateProc）；失败/超时一律 None，探测不打断状态展示。
pub fn probe_version(binary: &Path) -> Option<String> {
    let output = run_capture(binary, "--version")?;
    parse_version_output(&output)
}

/// 执行并收集 stdout（带超时看门狗，超时杀进程）。失败返回 None。
fn run_capture(binary: &Path, arg: &str) -> Option<String> {
    let mut command = if cfg!(windows) {
        let ext = binary
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "bat" || ext == "cmd" {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(binary);
            command
        } else {
            Command::new(binary)
        }
    } else {
        Command::new(binary)
    };
    command.args([arg]);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::null());
    let mut child = command.spawn().ok()?;
    let pipe = child.stdout.take()?;

    // 读线程把 stdout 全量搬回；看门狗线程超时杀进程解除阻塞，
    // 完成标志位让它成功后立即退场（不拖慢正常路径）。
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut pipe = pipe;
        use std::io::Read;
        let _ = pipe.read_to_end(&mut buffer);
        let _ = sender.send(buffer);
    });
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let done = std::sync::Arc::clone(&done);
        let pid = child.id();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_millis(PROBE_TIMEOUT_MS);
            while std::time::Instant::now() < deadline {
                if done.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            // 进程已退出时 kill 是无害 no-op。
            unsafe {
                kill_pid(pid);
            }
        });
    }

    let stdout = receiver
        .recv_timeout(Duration::from_millis(PROBE_TIMEOUT_MS + 1_000))
        .ok()?;
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = child.wait().ok();
    reader.join().ok();
    Some(String::from_utf8_lossy(&stdout).to_string())
}

/// 跨平台按 PID 终止（仅用于自家探测子进程的超时看门狗）。
unsafe fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        // 无 crate 依赖的最小实现：taskkill /PID（失败即进程已退）。
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/// "moli 1.1.9" → "1.1.9"；解析失败 None。
pub fn parse_version_output(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim();
    let version = line.strip_prefix("moli ")?;
    let version = version.trim();
    if version.is_empty() || version.contains(char::is_whitespace) {
        return None;
    }
    Some(version.to_owned())
}

/// moli_status：三源解析 + 版本探测，任何失败都折叠为「未安装」。
pub fn status(app: &tauri::AppHandle, settings: &crate::settings::Settings) -> MoliStatus {
    let Some((path, source)) = resolve_binary(app, settings) else {
        return MoliStatus {
            installed: false,
            version: None,
            path: None,
            source: MoliSource::None,
        };
    };
    match probe_version(&path) {
        Some(version) => MoliStatus {
            installed: true,
            version: Some(version),
            path: Some(path.to_string_lossy().to_string()),
            source,
        },
        None => MoliStatus {
            installed: false,
            version: None,
            path: None,
            source: MoliSource::None,
        },
    }
}

/// Tauri 命令（R001 契约）。
#[tauri::command]
pub fn moli_status(app: tauri::AppHandle) -> Result<MoliStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    Ok(status(&app, &settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 版本输出解析_标准格式() {
        assert_eq!(
            parse_version_output("moli 1.1.9\n"),
            Some("1.1.9".to_owned())
        );
        assert_eq!(
            parse_version_output("moli 1.2.3\n后续行忽略\n"),
            Some("1.2.3".to_owned())
        );
    }

    #[test]
    fn 版本输出解析_非法输入() {
        assert_eq!(parse_version_output(""), None);
        assert_eq!(parse_version_output("moli\n"), None);
        assert_eq!(parse_version_output("moli 1.1.9 extra\n"), None);
        assert_eq!(parse_version_output("not moli output\n"), None);
    }

    #[test]
    fn 版本探测_脚本桩真实拉起() {
        // 真实进程探测整链：spawn → 读取 → 解析。Windows 用 .cmd 桩
        // （覆盖 cmd /C 分支），其他平台用 sh 桩。
        let dir = std::env::temp_dir().join(format!("qx-moli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(windows)]
        {
            let stub = dir.join("moli.cmd");
            std::fs::write(&stub, "@echo moli 9.9.9-test\r\n").unwrap();
            assert_eq!(probe_version(&stub), Some("9.9.9-test".to_owned()));
        }
        #[cfg(not(windows))]
        {
            let stub = dir.join("moli-sh");
            std::fs::write(&stub, "#!/bin/sh\necho moli 9.9.9-test\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(probe_version(&stub), Some("9.9.9-test".to_owned()));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 版本探测_不存在的二进制返回_none() {
        let missing = std::env::temp_dir().join("qx-moli-missing-9x.exe");
        assert_eq!(probe_version(&missing), None);
    }

    #[test]
    fn 自备路径_空串与非文件都不命中() {
        let settings = crate::settings::Settings::default();
        assert!(custom_binary(&settings).is_none());
        let mut with_missing = settings.clone();
        with_missing.tools.moli.binary_path = std::env::temp_dir()
            .join("qx-moli-missing-9x.exe")
            .to_string_lossy()
            .to_string();
        assert!(custom_binary(&with_missing).is_none());
    }
}

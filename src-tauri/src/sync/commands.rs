//! 同步 IPC：git 存在性 / vault 仓状态 / 拉取 / 推送。

use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};

/// 同步面板状态。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// 系统 git 可用。
    pub git_available: bool,
    /// vault 是 git 工作区。
    pub initialized: bool,
    /// 有远端配置。
    pub has_remote: bool,
    /// 未提交文件数 / 领先远端 / 落后远端（None = 未知）。
    pub dirty: u32,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    /// 仓根（展示用）。
    pub vault: String,
}

#[tauri::command]
pub fn sync_status(app: AppHandle) -> Result<SyncStatus> {
    let state = app.state::<crate::AppState>();
    let vault = state.settings.lock().unwrap().notes.vault_dir.clone();
    let mut status = SyncStatus {
        git_available: git().is_some(),
        initialized: false,
        has_remote: false,
        dirty: 0,
        ahead: None,
        behind: None,
        vault: vault.clone(),
    };
    if !vault.is_empty() && Path::new(&vault).join(".git").exists() {
        status.initialized = true;
        status.has_remote = !run(&vault, &["remote"]).unwrap_or_default().is_empty();
        if let Ok(text) = run(&vault, &["status", "--porcelain"]) {
            status.dirty = text.lines().filter(|line| !line.trim().is_empty()).count() as u32;
        }
        if status.has_remote {
            // 先 fetch（网络操作，可能慢/失败），再读 ahead/behind。
            let _ = run(&vault, &["fetch", "--quiet"]);
            if let Some(branch) = branch(&vault) {
                if let Ok(text) = run(
                    &vault,
                    &[
                        "rev-list",
                        "--left-right",
                        "--count",
                        &format!("{branch}...@{{u}}"),
                    ],
                ) {
                    let mut parts = text.split_whitespace();
                    if let (Some(ahead), Some(behind)) = (parts.next(), parts.next()) {
                        status.ahead = ahead.parse().ok();
                        status.behind = behind.parse().ok();
                    }
                }
            }
        }
    }
    Ok(status)
}

/// 在 vault 建 git 仓（含 .trash/.qx-*.tmp 忽略规则与首提交）。
#[tauri::command]
pub fn sync_init(app: AppHandle) -> Result<String> {
    let state = app.state::<crate::AppState>();
    let vault = state.settings.lock().unwrap().notes.vault_dir.clone();
    if vault.is_empty() {
        return Err(Error::Sync("尚未初始化笔记库".to_owned()));
    }
    run(&vault, &["init", "--quiet"])?;
    let ignore = ".trash/\n*.tmp\n";
    std::fs::write(Path::new(&vault).join(".gitignore"), ignore)
        .map_err(|cause| Error::Sync(format!("写 .gitignore 失败：{cause}")))?;
    run(&vault, &["add", "-A"])?;
    run(
        &vault,
        &[
            "commit",
            "--quiet",
            "-m",
            "chore: 千寻笔记库初始化（自动提交）",
        ],
    )?;
    Ok("已初始化 git 仓并完成首提交".to_owned())
}

/// 拉取（rebase 本地提交，冲突时停下让用户手工处理）。
#[tauri::command]
pub fn sync_pull(app: AppHandle) -> Result<Vec<String>> {
    let state = app.state::<crate::AppState>();
    let vault = state.settings.lock().unwrap().notes.vault_dir.clone();
    let output = run(&vault, &["pull", "--rebase", "--autostash"])?;
    Ok(output.lines().map(str::to_owned).collect())
}

/// 推送（先提交本地改动，再推当前分支）。
#[tauri::command]
pub fn sync_push(app: AppHandle) -> Result<Vec<String>> {
    let state = app.state::<crate::AppState>();
    let vault = state.settings.lock().unwrap().notes.vault_dir.clone();
    let mut lines = Vec::new();
    let dirty = run(&vault, &["status", "--porcelain"])
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    if dirty > 0 {
        run(&vault, &["add", "-A"])?;
        let message = format!(
            "chore: 千寻自动同步 {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M")
        );
        let committed = run(&vault, &["commit", "--quiet", "-m", &message]);
        lines.push(format!("已提交 {dirty} 处改动"));
        if let Err(cause) = committed {
            lines.push(format!("提交警告：{cause}"));
        }
    } else {
        lines.push("无本地改动".to_owned());
    }
    for line in run(&vault, &["push"])?.lines() {
        lines.push(line.to_owned());
    }
    Ok(lines)
}

// ---- 内部 ----

fn git() -> Option<&'static str> {
    // 常见安装位兜底；PATH 里的 git 由 Command 直接解析。
    ["git", "C:/Program Files/Git/bin/git.exe"]
        .into_iter()
        .find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok()
        })
}

fn branch(vault: &str) -> Option<String> {
    let name = run(vault, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn run(vault: &str, args: &[&str]) -> Result<String> {
    let Some(git) = git() else {
        return Err(Error::Sync(
            "系统未安装 git：请安装 Git for Windows 后重试".to_owned(),
        ));
    };
    let output = std::process::Command::new(git)
        .args(["-C", vault])
        .args(args)
        .output()
        .map_err(|cause| Error::Sync(format!("启动 git 失败：{cause}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(Error::Sync(format!(
            "git {} 失败：{}{}",
            args.first().unwrap_or(&"?"),
            stdout.trim(),
            stderr.trim()
        )));
    }
    Ok(format!("{stdout}{stderr}"))
}

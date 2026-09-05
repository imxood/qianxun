//! 应用数据目录与关键文件路径。
//!
//! 数据根（无 mode 隔离时安装版与 dev 版会互相覆盖——上一轮讨论点）：
//! - 安装版（release）：`%USERPROFILE%\.qianxun`
//! - 调试版（`pnpm dev`）：`%USERPROFILE%\.qianxun_dev`
//!
//! 沿用 Unix 风格的 dotfile 约定，便于直接 ls 查问题；debug 用 `_dev`
//! 后缀隔离，让开发期间重装/改设置/换 DSH 都不动生产数据。旧版基于
//! `app_data_dir()` 的位置将不再写入。
//!
//! 各文件职责见 docs/02-技术架构.md §4.1：
//! - settings.json：设置唯一持久化事实（settings 模块独占读写）；
//! - logs/qianxun.log：外壳自身日志（logging 模块独占）。

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};

pub(crate) fn data_dir(app: &AppHandle) -> Result<PathBuf> {
    // debug 构建（`pnpm dev` 跑出的二进制）装到 ~/.qianxun_dev，与安装版
    // 的 ~/.qianxun 完全分离——debug 期间重装 DSH、改设置、刷固定版本都
    // 不会污染生产数据。第一次启用 dev 构建会按完整流程装一份 DSH。
    let folder = if cfg!(debug_assertions) {
        "qianxun_dev"
    } else {
        "qianxun"
    };
    let home = app
        .path()
        .home_dir()
        .map_err(|cause| Error::DataDir(format!("解析用户主目录失败：{cause}")))?;
    let dir = home.join(format!(".{folder}"));
    std::fs::create_dir_all(&dir).map_err(|cause| Error::DataDir(cause.to_string()))?;
    Ok(dir)
}

pub fn settings_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("settings.json"))
}

pub fn log_path(app: &AppHandle) -> Result<PathBuf> {
    let logs = data_dir(app)?.join("logs");
    std::fs::create_dir_all(&logs).map_err(|cause| Error::DataDir(cause.to_string()))?;
    Ok(logs.join("qianxun.log"))
}

/// 千寻私有 npm prefix：`node_modules` 落在这里，DSH 安装目标。
pub fn harness_dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("dsh-runtime"))
}

/// 事务安装的备份目录（架构 §4.4：pnpm 绝对路径 junction 决定了
/// 直装 live + rename 备份的形态，不再有 staging）。
pub fn harness_backup_dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("dsh-runtime-backup"))
}

/// DSH 安装后的 CLI 入口（npm 布局固定）。
pub fn harness_entry(app: &AppHandle) -> Result<PathBuf> {
    Ok(harness_dir(app)?
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("lib")
        .join("bin.js"))
}

/// 千寻自管的 Node 发行版目录（node/ 模块下载落位，node-runtime crate
/// 的 discover_in 也扫描这里）。
pub fn managed_node_dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("node"))
}

/// ADR-009：隔离模式下的 DSH_HOME。系统 ~/.dsh 可能被外部 DSH 实例占用，
/// 千寻默认用自己的副本，会话/存储/插件互不干扰。
pub fn dsh_home(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("dsh-home"))
}

/// DSH 的工作目录默认取用户主目录；具体工作区在 DSH 界面里选。
pub fn workspace_dir() -> PathBuf {
    dirs::home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// pnpm 工具目录：npm 把 pnpm 装在这里，安装器幂等复用（架构 §4.4）。
pub fn pnpm_tool_dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("pnpm-tool"))
}

//! DSH 托管域：把 DeepSeek Harness 作为一个受监督的本地服务来运行。
//!
//! mod.rs 负责三件事：探测环境（environment）、把环境变成可执行的
//! 启动计划（launch_plan）、把安装需求变成安装计划（install_plan）。
//! 状态机与进程ownership在 supervisor，安装执行在 install，IPC 在 commands。

pub mod commands;
pub mod health;
pub mod install;
pub mod node_install;
pub mod readiness;
pub mod supervisor;

use std::path::PathBuf;

use node_runtime::NodeInstallation;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::paths;
use crate::settings::{Settings, DSH_HOME_SYSTEM};
use install::InstallPlan;
use supervisor::LaunchPlan;

/// 只绑回环。绑到别处等于把一个能跑 shell 命令的 agent 暴露给局域网，
/// 所以它不做成设置。
const BIND_HOST: &str = "127.0.0.1";

/// 固定启动 web profile；profile 选择器后续接入。
pub const DEFAULT_PROFILE: &str = "web";

/// 这台机器当前能不能跑 DSH、缺什么。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    /// 最佳可用 Node 运行时；没有合格者时为 None。
    pub node: Option<NodeInstallation>,
    /// 找到的全部运行时（UI 据此解释某个为何被淘汰）。
    pub all_node_runtimes: Vec<NodeInstallation>,
    pub minimum_node: node_runtime::Version,
    pub dsh_installed: bool,
    pub dsh_version: Option<String>,
    /// 下一次安装将使用的包说明符（含锁定版本或 latest）。
    pub install_spec: String,
    pub dsh_entry: PathBuf,
    pub workspace: PathBuf,
    /// 当前 DSH_HOME 策略生效的目录。
    pub dsh_home: PathBuf,
}

/// 探测环境。探测本身可能 spawn 若干 `node --version`，命令层把它
/// 放进 spawn_blocking；这里只做同步工作。
pub fn environment(app: &tauri::AppHandle, settings: &Settings) -> Environment {
    // 顺手机会：恢复一次被打断的安装（无标记时是零写入的空操作）。
    let recovered = paths::harness_dir(app)
        .ok()
        .zip(paths::harness_backup_dir(app).ok())
        .and_then(|(live, backup)| install::recover_interrupted_install(&live, &backup).ok());
    if recovered == Some(true) {
        crate::logging::log("info", "检测到未完成的 DSH 安装，已自动恢复");
    }

    // 千寻自管的 Node 与版本管理器一起参与同一套排序规则，
    // 它装的运行机和用户自己装的按同一条规则被选中。
    let managed = paths::managed_node_dir(app).ok();
    let all_node_runtimes = node_runtime::discover_in(managed.as_deref());
    let node = all_node_runtimes
        .iter()
        .find(|install| install.version >= node_runtime::MINIMUM_SUPPORTED)
        .cloned();

    let harness_dir = paths::harness_dir(app).unwrap_or_else(|_| PathBuf::from("."));
    let dsh_entry = paths::harness_entry(app).unwrap_or_else(|_| PathBuf::from("."));
    let dsh_version = install::runtime_version(&harness_dir);
    let dsh_installed = dsh_version.is_some() && dsh_entry.is_file();

    Environment {
        node,
        all_node_runtimes,
        minimum_node: node_runtime::MINIMUM_SUPPORTED,
        dsh_installed,
        dsh_version,
        install_spec: install_spec(settings),
        dsh_entry,
        workspace: paths::workspace_dir(),
        dsh_home: dsh_home(app, settings),
    }
}

/// 安装说明符：锁定版本优先，否则 latest。
fn install_spec(settings: &Settings) -> String {
    if settings.dsh.pinned_version.is_empty() {
        install::LATEST_SPEC.to_owned()
    } else {
        format!("{}@{}", install::PACKAGE, settings.dsh.pinned_version)
    }
}

/// ADR-009：isolated → 应用数据目录下的独立 HOME；system → 系统默认
/// （不注入 DSH_HOME，DSH 自己落到 ~/.dsh）。
pub fn dsh_home(app: &tauri::AppHandle, settings: &Settings) -> PathBuf {
    if settings.dsh.home == DSH_HOME_SYSTEM {
        PathBuf::from(
            std::env::var_os("DSH_HOME").unwrap_or(
                dirs::home_dir()
                    .map(|home| home.join(".dsh"))
                    .map(|p| p.into_os_string())
                    .unwrap_or_default(),
            ),
        )
    } else {
        paths::dsh_home(app).unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// 把当前环境变成可运行的启动计划，或说清缺什么。
pub fn launch_plan(app: &tauri::AppHandle, settings: &Settings) -> Result<LaunchPlan> {
    let environment = environment(app, settings);

    let node = environment.node.clone().ok_or(Error::NoNodeRuntime {
        minimum: node_runtime::MINIMUM_SUPPORTED,
    })?;
    if !environment.dsh_installed || !environment.dsh_entry.is_file() {
        // 诊断：把判定依据原样落进日志，环境问题一眼可辨。
        let dir = paths::harness_dir(app).unwrap_or_else(|_| PathBuf::from("."));
        crate::logging::log(
            "warn",
            &format!(
                "DSH 判定未安装：dir={} version={:?} entry={} entry存在={} node={}",
                dir.display(),
                install::runtime_version(&dir),
                environment.dsh_entry.display(),
                environment.dsh_entry.is_file(),
                node.path.display(),
            ),
        );
        return Err(Error::DshNotInstalled);
    }

    Ok(LaunchPlan {
        node: node.path,
        entry: environment.dsh_entry,
        profile: DEFAULT_PROFILE.to_owned(),
        workspace: environment.workspace,
        host: BIND_HOST.to_owned(),
        // 固定端口（ADR-002）；只有用户显式允许随机 fallback 时才传 0。
        port: if settings.dsh.allow_random_fallback {
            0
        } else {
            settings.dsh.port
        },
        dsh_home: (settings.dsh.home != DSH_HOME_SYSTEM)
            .then(|| paths::dsh_home(app))
            .transpose()?,
    })
}

/// 生成安装计划（用哪个 Node 的 npm、装到哪、装什么、走哪个源）。
pub fn install_plan(app: &tauri::AppHandle, settings: &Settings) -> Result<InstallPlan> {
    let environment = environment(app, settings);
    let supported = environment
        .all_node_runtimes
        .iter()
        .filter(|install| install.version >= node_runtime::MINIMUM_SUPPORTED)
        .collect::<Vec<_>>();
    if supported.is_empty() {
        return Err(Error::NoNodeRuntime {
            minimum: node_runtime::MINIMUM_SUPPORTED,
        });
    }
    // 启动只需要 Node，安装还需要那个 Node 配套的 npm。一个只有
    // Node 的更新安装不能把一个带 npm 的完整旧安装挤掉。
    let selected = environment.node.as_ref().map(|node| node.path.as_path());
    let node = selected
        .and_then(|path| {
            supported
                .iter()
                .copied()
                .find(|install| install.path == path && install::npm_cli(&install.path).is_some())
        })
        .or_else(|| {
            supported
                .into_iter()
                .find(|install| install::npm_cli(&install.path).is_some())
        })
        .ok_or(Error::NpmMissing)?;

    Ok(InstallPlan {
        node: node.path.clone(),
        npm_cli: install::npm_cli(&node.path).ok_or(Error::NpmMissing)?,
        target: paths::harness_dir(app)?,
        pnpm_tool_dir: paths::pnpm_tool_dir(app)?,
        spec: install_spec(settings),
        registry: settings.mirrors.registry_url(),
    })
}

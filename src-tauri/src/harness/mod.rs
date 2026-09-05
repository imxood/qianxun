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
use tauri::Emitter;

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

/// 安装进度事件通道（环境页进度卡的实时数据源）。
pub const INSTALL_PROGRESS_CHANNEL: &str = "harness://install-progress";

/// 一次组件安装的实时进度。
///
/// Node 下载有真实字节进度（HEAD 探测总大小 + 轮询落盘字节数）；DSH 走
/// pnpm，包管理器只暴露包数推进（resolved/downloaded/added），没有字节
/// 概念——UI 据此分别渲染。None 字段一律表示「该维度暂无数据」，前端
/// 沿用上一次的值。
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "stage",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum InstallProgress {
    /// Node：获取 SHA-256 校验清单。
    NodeManifest { source: String },
    /// Node：发行包下载中。total_bytes 来自 HEAD 探测（None = 源未提供）。
    NodeDownload {
        source: String,
        url: String,
        total_bytes: Option<u64>,
        downloaded_bytes: u64,
    },
    /// Node：下载后的校验/解压阶段。
    NodeFinalize { source: String, activity: String },
    /// DSH：pnpm 包数推进。total_hint 来自「Packages: +N」行。
    DshPackages {
        registry: String,
        resolved: Option<u64>,
        downloaded: u64,
        added: u64,
        total_hint: Option<u64>,
    },
    /// 安装流程结束（无论成败，前端收起进度卡）。
    Done,
}

/// 进度事件发射闭包：安装域统一经它上报，命令层不必各自包 emit。
pub fn progress_sink(app: &tauri::AppHandle) -> impl Fn(InstallProgress) + Clone + Send + 'static {
    let app = app.clone();
    move |progress| {
        let _ = app.emit(INSTALL_PROGRESS_CHANNEL, &progress);
    }
}

/// 从 pnpm append-only 输出行提取包数推进。
///
/// 「Packages: +234」给 total_hint；「Progress: resolved 234, reused 200,
/// downloaded 30, added 10」给一次计数推进。未命中返回空。
pub fn parse_dsh_progress(line: &str, registry: &str) -> Vec<InstallProgress> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("Packages: +") {
        return rest
            .trim()
            .parse::<u64>()
            .ok()
            .map(|total| {
                vec![InstallProgress::DshPackages {
                    registry: registry.to_owned(),
                    resolved: None,
                    downloaded: 0,
                    added: 0,
                    total_hint: Some(total),
                }]
            })
            .unwrap_or_default();
    }
    if !line.starts_with("Progress:") {
        return Vec::new();
    }
    let resolved = progress_field(line, "resolved ");
    let downloaded = progress_field(line, "downloaded ");
    let added = progress_field(line, "added ");
    if downloaded.is_none() && added.is_none() {
        return Vec::new();
    }
    vec![InstallProgress::DshPackages {
        registry: registry.to_owned(),
        resolved,
        downloaded: downloaded.unwrap_or(0),
        added: added.unwrap_or(0),
        total_hint: None,
    }]
}

/// 取「<key>数字」字段（pnpm 的 Progress 行形如 `resolved 12, reused 3,`）。
fn progress_field(line: &str, key: &str) -> Option<u64> {
    let start = line.find(key)? + key.len();
    let digits: String = line[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

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
    /// 下一次安装将使用的包说明符（千寻版本锁定的精确 DSH 版本）。
    pub install_spec: String,
    pub dsh_entry: PathBuf,
    pub workspace: PathBuf,
    /// 当前 DSH_HOME 策略生效的目录。
    pub dsh_home: PathBuf,
    /// 千寻一键安装会下载的 Node 版本（UI 按钮文案用它，避免前端硬编码漂移）。
    pub bundled_node_version: String,
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
        install_spec: install::install_spec(),
        dsh_entry,
        workspace: paths::workspace_dir(),
        dsh_home: dsh_home(app, settings),
        bundled_node_version: node_install::NODE_VERSION.to_owned(),
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
        // ADR-002 修订：启动端口恒为 0（OS 分配随机空闲端口）。内核层面
        // 保证端口可用，「被占用重试」没有存在的必要；debug 与安装版
        // 同时启动 DSH 也各拿各的端口。settings.dsh.port 与
        // allow_random_fallback 保留为历史字段，不再参与启动。
        port: 0,
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
        spec: install::install_spec(),
        registry: settings.mirrors.registry_url(),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_dsh_progress, InstallProgress};

    const REGISTRY: &str = "https://registry.npmmirror.com/";

    #[test]
    fn packages行给出总包数提示() {
        let events = parse_dsh_progress("Packages: +234", REGISTRY);
        let Some(InstallProgress::DshPackages {
            registry,
            total_hint,
            ..
        }) = events.first()
        else {
            panic!("应解析出 DshPackages：{events:?}");
        };
        assert_eq!(registry, REGISTRY);
        assert_eq!(*total_hint, Some(234));
    }

    #[test]
    fn progress行解析四个计数() {
        let line = "Progress: resolved 234, reused 200, downloaded 30, added 12";
        let events = parse_dsh_progress(line, REGISTRY);
        let Some(InstallProgress::DshPackages {
            resolved,
            downloaded,
            added,
            total_hint,
            ..
        }) = events.first()
        else {
            panic!("应解析出 DshPackages");
        };
        assert_eq!(*resolved, Some(234));
        assert_eq!(*downloaded, 30);
        assert_eq!(*added, 12);
        assert_eq!(*total_hint, None);
    }

    #[test]
    fn 普通日志行不产生进度() {
        assert!(parse_dsh_progress("Packages are hard linked from the store", REGISTRY).is_empty());
        assert!(parse_dsh_progress("Done in 41.2s", REGISTRY).is_empty());
        assert!(parse_dsh_progress("", REGISTRY).is_empty());
    }
}

//! 替用户安装 DSH，而不是叫他自己开终端敲命令。
//!
//! DSH 是 npm 包；桌面应用在「请打开终端装依赖」这一步就不再是桌面
//! 应用了。千寻把 DSH 装进自己数据目录下的私有 prefix：不全局安装、
//! 不动 PATH、不假设 `npm` 作为命令可达——直接用探测到的 Node 跑
//! npm 的入口脚本。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use proc_guard::ProcessGuard;
use serde::{Deserialize, Serialize};
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::supervisor::Stream;
use crate::atomic;
use crate::error::{Error, Result};

/// DSH 的 npm 包名。
pub const PACKAGE: &str = "@deepseek-ai/dsh";

/// 未设置锁定版本时装 latest（设置 dsh.pinnedVersion 可钉死精确版本）。
pub const LATEST_SPEC: &str = "@deepseek-ai/dsh@latest";

const JOURNAL_VERSION: u8 = 1;
const INSTALL_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
pub(super) const INSTALL_TOTAL_TIMEOUT: Duration = Duration::from_secs(20 * 60);
pub(super) const PIPE_DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

/// 千寻自带的 pnpm 版本。DSH 的依赖树带 peer 链（如 cordis-plugin-group），
/// npm 的解析要么组合爆炸要么丢 peer；pnpm 的 auto-install-peers 是
/// 唯一被验证可正确装出运行时的路线。pnpm 由 npm 装进工具目录（pnpm
/// 自身无原生依赖、无 peer，npm 装它没有上述问题）。
pub const PNPM_SPEC: &str = "pnpm@11.7.0";

/// pnpm 构建脚本白名单：DSH 运行时需要这些原生/生成步骤真正执行
/// （koffi 与 node-pty 是终端/子进程工具的原生绑定，没有它们对应功能
/// 会在运行时报模块缺失）。pnpm 11 从 pnpm-workspace.yaml 读 allowBuilds。
const PNPM_WORKSPACE_YAML: &str = "\
# 千寻安装器生成：允许 DSH 运行时的原生构建脚本执行。
allowBuilds:
  esbuild: true
  koffi: true
  node-pty: true
  protobufjs: true
  \"@google/genai\": true
  \"@deepseek-ai/dsh-subprocess-local\": true
";

/// 一次安装需要的全部输入。
#[derive(Clone, Debug)]
pub struct InstallPlan {
    /// 执行 npm/pnpm 的 Node 运行时。
    pub node: PathBuf,
    /// npm 自己的入口脚本：直接跑而不是经过 shim。
    pub npm_cli: PathBuf,
    /// `node_modules` 所在目录（千寻私有 prefix）。
    pub target: PathBuf,
    /// pnpm 工具目录（npm 装 pnpm 的落位，幂等复用）。
    pub pnpm_tool_dir: PathBuf,
    /// 包说明符（含版本）。
    pub spec: String,
    /// npm registry（按设置解析好的真实 URL）。
    pub registry: String,
}

impl InstallPlan {
    /// pnpm 的入口脚本（npm 装出来的 cjs）。存在即复用。
    pub fn pnpm_cli(&self) -> PathBuf {
        self.pnpm_tool_dir
            .join("node_modules")
            .join("pnpm")
            .join("bin")
            .join("pnpm.cjs")
    }

    /// 用 npm 把 pnpm 工具装进 tool dir 的命令。
    fn pnpm_tool_command(&self) -> Command {
        let mut command = Command::new(&self.node);
        command
            .arg(&self.npm_cli)
            .arg("install")
            .arg(PNPM_SPEC)
            .arg("--prefix")
            .arg(&self.pnpm_tool_dir)
            .arg("--no-audit")
            .arg("--no-fund")
            .arg("--loglevel=http")
            .arg(format!("--registry={}", self.registry))
            .current_dir(&self.pnpm_tool_dir)
            .env("PATH", path_with_node(&self.node))
            .env("npm_config_update_notifier", "false")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        hide_console_window(&mut command);
        command
    }

    /// 安装目标目录里执行 `pnpm add` 的命令。
    fn to_command(&self) -> Command {
        let mut command = Command::new(&self.node);
        command
            .arg(self.pnpm_cli())
            .arg("add")
            .arg(&self.spec)
            // pnpm 的 --dir 要求目录已存在（run() 里先建）。
            .arg("--dir")
            .arg(&self.target)
            .arg("--registry")
            .arg(&self.registry)
            // 无 TTY 时选追加式输出：每行都进日志面板，不画进度条。
            .arg("--reporter=append-only")
            // peer 链必须自动补齐（DSH 运行时的关键依赖走 peer 声明）。
            .arg("--config.auto-install-peers=true")
            .current_dir(&self.target)
            .env("PATH", path_with_node(&self.node))
            .env("npm_config_update_notifier", "false")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        hide_console_window(&mut command);
        command
    }
}

/// CREATE_NO_WINDOW：子进程走重定向管道汇报，控制台宿主不许在外壳上
/// 闪现（npm/curl 通用；node_install 复用）。
pub(super) fn hide_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        // npm 与生命周期脚本走重定向管道汇报；CREATE_NO_WINDOW 阻止
        // 它们的控制台宿主在外壳上闪现，同时保住这些输出。
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// 为指定的 Node 定位 npm 入口脚本。
///
/// 用已知 Node 直接跑 npm-cli.js 是精确的：不会从 PATH 捡到另一个
/// 运行时，在 Windows 上也避开经 cmd 调 `npm.cmd`。
pub fn npm_cli(node: &Path) -> Option<PathBuf> {
    npm_cli_candidates(node)
        .into_iter()
        .find(|candidate| candidate.is_file() && npm_cli_works(node, candidate))
}

/// 官方包、版本管理器与 Homebrew 各自的目录布局。
fn npm_cli_candidates(node: &Path) -> Vec<PathBuf> {
    let Some(directory) = node.parent() else {
        return Vec::new();
    };
    vec![
        // Windows：npm 与 node.exe 并排。
        directory.join("node_modules/npm/bin/npm-cli.js"),
        // 官方 Unix 包、nvm、fnm、Volta。
        directory.join("../lib/node_modules/npm/bin/npm-cli.js"),
        // Homebrew formula 自带的 npm（规范化 Cellar 路径下）。
        directory.join("../libexec/lib/node_modules/npm/bin/npm-cli.js"),
        // Homebrew 前缀共享的 npm。
        directory.join("../../../../lib/node_modules/npm/bin/npm-cli.js"),
    ]
}

/// 用选定的 Node 真跑一次 `npm --version`，证明这个入口脚本属于一个
/// 能工作的 npm。PATH 上同名 shim 永远不被咨询。
fn npm_cli_works(node: &Path, npm_cli: &Path) -> bool {
    let mut command = std::process::Command::new(node);
    command
        .arg(npm_cli)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    proc_guard::hide_console(&mut command);
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return false;
    };
    let output = std::thread::spawn(move || crate::child_output::capture_sync(stdout, 16 << 10));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let Ok(Ok(output)) = output.join() else {
        return false;
    };
    status.is_some_and(|status| status.success()) && !output.iter().all(u8::is_ascii_whitespace)
}

/// 把选定 Node 的目录放到 PATH 最前面。
fn path_with_node(node: &Path) -> OsString {
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let Some(directory) = node.parent() else {
        return existing;
    };

    let mut entries = vec![directory.to_path_buf()];
    entries.extend(std::env::split_paths(&existing));
    std::env::join_paths(entries).unwrap_or(existing)
}

/// 读取安装目标里 DSH 的版本（没有则 None）。
pub fn runtime_version(target: &Path) -> Option<String> {
    let manifest = target
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("package.json");
    let raw = std::fs::read_to_string(manifest).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let version = parsed.get("version")?.as_str()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// DSH 的 CLI 入口路径。
pub fn entry_of(target: &Path) -> PathBuf {
    target
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("lib")
        .join("bin.js")
}

fn runtime_complete(target: &Path) -> bool {
    runtime_version(target).is_some() && entry_of(target).is_file()
}

/// 事务式安装：备份现有 live → 直装 live → 校验 → 清理备份。
///
/// 为什么不是 staging→晋升：pnpm 的 node_modules 是**绝对路径** junction
/// （node_modules/@deepseek-ai/dsh → …\.pnpm\…），装好的目录一旦整体
/// rename，每条链接都指向旧路径、全部断链。而备份目录里的 junction
/// 指向 live 路径——还原时把备份 rename 回 live，链接恰好自洽。
///
/// journal 故意没有阶段字段：恢复逻辑从 live/backup 的真实状态推导真相，
/// 崩溃不可能留下一个「声称已发生」而文件系统不认账的阶段标记。
pub async fn run_transactional<R>(plan: &InstallPlan, report: R) -> Result<()>
where
    R: Fn(Stream, String) + Clone + Send + 'static,
{
    let live = plan.target.clone();
    let backup = sibling_dir(&plan.target, "-backup")
        .ok_or_else(|| Error::Install("安装目标缺少父目录（无法定位备份目录）".to_owned()))?;
    let journal = plan
        .target
        .parent()
        .map(|parent| parent.join("dsh-install.json"))
        .ok_or_else(|| Error::Install("安装目标缺少父目录（无法定位安装状态）".to_owned()))?;

    recover_interrupted_install(&live, &backup)?;
    remove_dir_if_exists(&backup)?;
    if live.exists() {
        std::fs::rename(&live, &backup)
            .map_err(|cause| Error::Install(format!("升级前保留现有 DSH 运行时失败：{cause}")))?;
    }
    write_journal(&journal, &plan.spec)?;

    let installed = run(plan, report).await.and_then(|()| {
        if runtime_complete(&live) {
            Ok(())
        } else {
            Err(Error::Install(
                "pnpm 报告成功，但校验发现 DSH 入口或版本缺失".to_owned(),
            ))
        }
    });

    match installed {
        Ok(()) => {
            remove_dir_if_exists(&backup)?;
            clear_journal(&journal)?;
            Ok(())
        }
        Err(failure) => {
            // 回滚：装坏的 live 让位，把备份放回 live 路径。
            let _ = remove_dir_if_exists(&live);
            if backup.exists() {
                std::fs::rename(&backup, &live).map_err(|cause| {
                    Error::Install(format!("回滚到上一个 DSH 运行时失败：{cause}"))
                })?;
            }
            let _ = std::fs::remove_file(&journal);
            Err(failure)
        }
    }
}

/// live 的同级伴随目录（如 dsh-runtime-backup）。
fn sibling_dir(target: &Path, suffix: &str) -> Option<std::path::PathBuf> {
    let name = target.file_name()?.to_string_lossy().into_owned();
    target.parent().map(|parent| parent.join(name + suffix))
}

/// 执行安装命令，逐行上报 pnpm 的全部输出。
async fn run<R>(plan: &InstallPlan, report: R) -> Result<()>
where
    R: Fn(Stream, String) + Clone + Send + 'static,
{
    std::fs::create_dir_all(&plan.target)
        .map_err(|cause| Error::Install(format!("无法创建 {}：{cause}", plan.target.display())))?;
    // 构建脚本白名单：没有它 pnpm 静默跳过原生构建，运行时才炸。
    std::fs::write(plan.target.join("pnpm-workspace.yaml"), PNPM_WORKSPACE_YAML)
        .map_err(|cause| Error::Install(format!("无法写入 pnpm 构建白名单：{cause}")))?;
    run_with_limits(
        plan.to_command(),
        report,
        "pnpm add",
        INSTALL_IDLE_TIMEOUT,
        INSTALL_TOTAL_TIMEOUT,
        PIPE_DRAIN_TIMEOUT,
    )
    .await
}

/// 幂等确保 pnpm 工具就位。
/// 工具目录整体不属于安装事务：它只含 pnpm 自身，坏了删掉重装即可。
pub async fn ensure_pnpm_tool<R>(plan: &InstallPlan, report: R) -> Result<()>
where
    R: Fn(Stream, String) + Clone + Send + 'static,
{
    if plan.pnpm_cli().is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(&plan.pnpm_tool_dir).map_err(|cause| {
        Error::Install(format!(
            "无法创建 {}：{cause}",
            plan.pnpm_tool_dir.display()
        ))
    })?;
    let installed = run_with_limits(
        plan.pnpm_tool_command(),
        report,
        "npm install pnpm",
        INSTALL_IDLE_TIMEOUT,
        Duration::from_secs(5 * 60),
        PIPE_DRAIN_TIMEOUT,
    )
    .await;
    if installed.is_ok() && !plan.pnpm_cli().is_file() {
        return Err(Error::Install(
            "npm 报告成功，但 pnpm 入口脚本缺失".to_owned(),
        ));
    }
    installed
}

/// 限时执行一条安装器命令：空闲/总超时、进程树回收、逐行转发。
/// Node 安装（node_install.rs）复用同一套闸门。
pub(super) async fn run_with_limits<R>(
    mut command: Command,
    report: R,
    label: &'static str,
    idle_timeout: Duration,
    total_timeout: Duration,
    pipe_drain_timeout: Duration,
) -> Result<()>
where
    R: Fn(Stream, String) + Clone + Send + 'static,
{
    // 安装拥有自己独立的进程树托管：超时回收 npm 的整棵生命周期树，
    // 不会波及 supervisor 独立 guard 拥有的运行中 DSH。
    let guard = ProcessGuard::new().map_err(|cause| Error::ProcessGuard(cause.to_string()))?;
    let mut child = guard
        .spawn(&mut command)
        .map_err(|cause| Error::Spawn(cause.to_string()))?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
        let _ = child.kill().await;
        let _ = child.wait().await;
        if let Some(pid) = pid {
            let _ = guard.finish(pid);
        }
        return Err(Error::Install(format!("{label} 没有提供诊断管道")));
    };
    let (activity, mut observed) = mpsc::channel(1);
    let mut out = tokio::spawn(forward(
        stdout,
        Stream::Stdout,
        report.clone(),
        activity.clone(),
    ));
    let mut err = tokio::spawn(forward(stderr, Stream::Stderr, report, activity.clone()));
    drop(activity);

    let idle = tokio::time::sleep(idle_timeout);
    let total = tokio::time::sleep(total_timeout);
    tokio::pin!(idle, total);
    let mut observing = true;
    let status = loop {
        tokio::select! {
            biased;
            result = child.wait() => {
                if let Some(pid) = pid {
                    if let Err(cause) = guard.finish(pid) {
                        out.abort();
                        err.abort();
                        return Err(Error::Install(format!(
                            "{label} 进程树无法回收：{cause}"
                        )));
                    }
                }
                break result.map_err(|cause| {
                    Error::Install(format!("无法等待 {label}：{cause}"))
                })?;
            }
            activity = observed.recv(), if observing => {
                match activity {
                    // 活动是边沿信号：一个待处理的唤醒足以重置空闲时限，
                    // 噪音大的子进程也不会撑出无界队列。
                    Some(()) => idle.as_mut().reset(Instant::now() + idle_timeout),
                    None => observing = false,
                }
            }
            _ = &mut idle => {
                let _ = guard.terminate_all();
                let _ = child.wait().await;
                if let Some(pid) = pid {
                    let _ = guard.finish(pid);
                }
                out.abort();
                err.abort();
                return Err(Error::Install(format!(
                    "{label} 已 {} 秒没有任何输出，被停止；请在可用网络下重试",
                    idle_timeout.as_secs()
                )));
            }
            _ = &mut total => {
                let _ = guard.terminate_all();
                let _ = child.wait().await;
                if let Some(pid) = pid {
                    let _ = guard.finish(pid);
                }
                out.abort();
                err.abort();
                return Err(Error::Install(format!(
                    "{label} 超过 {} 分钟安全上限，被停止",
                    total_timeout.as_secs() / 60
                )));
            }
        }
    };

    // Windows 上生命周期脚本的后代可能在 npm 退出后仍继承输出句柄。
    // 短暂排空正常输出，但绝不把一次成功/失败的 npm 退出变成永久转圈。
    if tokio::time::timeout(pipe_drain_timeout, async {
        let _ = tokio::join!(&mut out, &mut err);
    })
    .await
    .is_err()
    {
        out.abort();
        err.abort();
    }

    if !status.success() {
        return Err(Error::Install(format!("{label} 以 {status} 退出")));
    }
    Ok(())
}

async fn forward<P, R>(pipe: P, stream: Stream, report: R, activity: mpsc::Sender<()>)
where
    P: tokio::io::AsyncRead + Unpin,
    R: Fn(Stream, String),
{
    let mut lines = BufReader::new(pipe);
    let mut raw = Vec::new();
    while matches!(
        crate::child_output::next_line(&mut lines, &mut raw).await,
        Ok(true)
    ) {
        let _ = activity.try_send(());
        report(stream, String::from_utf8_lossy(&raw).trim_end().to_string());
    }
}

/// 安装目标的安装后校验（对外暴露给环境探测复用）。
pub fn check_installed(target: &Path) -> Result<()> {
    if runtime_complete(target) {
        return Ok(());
    }
    Err(Error::Install(format!(
        "DSH 安装不完整：入口或版本信息缺失（{}）",
        target.display()
    )))
}

/// 修复一次在任意阶段被打断的安装。
///
/// 返回 true 表示发现了 journal。环境探测可以安全地每次都调用：
/// 没有标记时它不做任何文件系统写入。pnpm 布局下备份里的 junction
/// 指向 live 路径，rename 回 live 即恢复可用。
pub fn recover_interrupted_install(live: &Path, backup: &Path) -> Result<bool> {
    let journal = live.parent().map(|parent| parent.join("dsh-install.json"));
    let Some(journal) = journal else {
        return Ok(false);
    };
    if !journal.exists() {
        return Ok(false);
    }
    read_journal(&journal)?;

    if runtime_complete(live) {
        // 安装已成功，只是清理步骤没走完。
        remove_dir_if_exists(backup)?;
    } else if runtime_complete(backup) {
        // 直装 live 中途崩溃：清掉残骸，把备份放回 live 路径。
        remove_dir_if_exists(live)?;
        std::fs::rename(backup, live)
            .map_err(|cause| Error::Install(format!("恢复上一个 DSH 运行时失败：{cause}")))?;
    } else {
        // 中断前后都没有完整结果。留标记只会让重试永远失败。
        remove_dir_if_exists(live)?;
        remove_dir_if_exists(backup)?;
    }

    clear_journal(&journal)?;
    Ok(true)
}

fn clear_journal(journal: &Path) -> Result<()> {
    std::fs::remove_file(journal)
        .map_err(|cause| Error::Install(format!("清除已恢复的安装日志失败：{cause}")))
}

#[derive(Debug, Deserialize, Serialize)]
struct InstallJournal {
    schema: u8,
    package: String,
    spec: String,
}

fn write_journal(path: &Path, spec: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|cause| Error::Install(format!("无法创建安装状态目录：{cause}")))?;
    }
    let journal = InstallJournal {
        schema: JOURNAL_VERSION,
        package: PACKAGE.to_owned(),
        spec: spec.to_owned(),
    };
    let body = serde_json::to_vec_pretty(&journal)
        .map_err(|cause| Error::Install(format!("无法编码安装状态：{cause}")))?;
    atomic::write(path, &body).map_err(|cause| Error::Install(format!("无法提交安装状态：{cause}")))
}

fn read_journal(path: &Path) -> Result<InstallJournal> {
    let raw = std::fs::read_to_string(path)
        .map_err(|cause| Error::Install(format!("无法读取安装状态：{cause}")))?;
    let journal: InstallJournal = serde_json::from_str(&raw)
        .map_err(|cause| Error::Install(format!("安装状态不合法：{cause}")))?;
    if journal.schema != JOURNAL_VERSION || journal.package != PACKAGE {
        return Err(Error::Install("安装状态属于不支持的安装事务".to_owned()));
    }
    Ok(journal)
}

fn remove_dir_if_exists(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(cause) => {
            return Err(Error::Install(format!(
                "无法检查 {}：{cause}",
                path.display()
            )))
        }
    };
    // 拒绝对「链接或非目录」做递归删除：这是防线，防止路径被替换成
    // 指向别处的链接后把别处清空。
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Install(format!(
            "拒绝递归删除非目录或链接路径 {}",
            path.display()
        )));
    }
    std::fs::remove_dir_all(path)
        .map_err(|cause| Error::Install(format!("无法删除 {}：{cause}", path.display())))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{
        check_installed, entry_of, npm_cli_candidates, recover_interrupted_install,
        remove_dir_if_exists, runtime_complete, runtime_version, InstallPlan, LATEST_SPEC, PACKAGE,
        PNPM_SPEC,
    };

    fn write_runtime(root: &Path, version: Option<&str>) {
        if let Some(version) = version {
            let package = root.join("node_modules/@deepseek-ai/dsh");
            fs::create_dir_all(package.join("lib")).expect("runtime directory");
            fs::write(
                package.join("package.json"),
                format!(r#"{{"name":"{PACKAGE}","version":"{version}"}}"#),
            )
            .expect("manifest");
            fs::write(package.join("lib/bin.js"), "").expect("entry");
        }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "qianxun-install-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn 默认安装说明符是latest() {
        assert_eq!(LATEST_SPEC, format!("{PACKAGE}@latest"));
    }

    #[test]
    fn npm候选覆盖版本管理器布局() {
        let node = Path::new("/Users/person/.nvm/versions/node/v24.19.0/bin/node");
        let candidates = npm_cli_candidates(node);
        assert!(candidates.contains(
            &node
                .parent()
                .expect("bin")
                .join("../lib/node_modules/npm/bin/npm-cli.js")
        ));
    }

    #[test]
    fn 安装命令走pnpm形态与镜像源() {
        let plan = InstallPlan {
            node: Path::new("node").to_path_buf(),
            npm_cli: Path::new("npm-cli.js").to_path_buf(),
            target: Path::new("runtime").to_path_buf(),
            pnpm_tool_dir: Path::new("pnpm-tool").to_path_buf(),
            spec: LATEST_SPEC.to_owned(),
            registry: "https://registry.npmmirror.com/".to_owned(),
        };
        let arguments = plan
            .to_command()
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        // 入口是 pnpm.cjs，动作用 add --dir；peer 自动补齐是硬要求。
        assert!(arguments
            .first()
            .is_some_and(|first| first.ends_with("pnpm.cjs")));
        assert!(arguments.contains(&"add".to_owned()));
        assert!(arguments.contains(&"--dir".to_owned()));
        assert!(arguments.contains(&"--config.auto-install-peers=true".to_owned()));
        assert!(arguments.contains(&"--registry".to_owned()));
        assert!(arguments.contains(&"https://registry.npmmirror.com/".to_owned()));

        // pnpm 工具自身的安装走 npm --prefix，镜像策略同样生效。
        let tool = plan
            .pnpm_tool_command()
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(tool.contains(&PNPM_SPEC.to_owned()));
        assert!(tool.contains(&"--prefix".to_owned()));
        assert!(tool.contains(&"--registry=https://registry.npmmirror.com/".to_owned()));
    }

    #[test]
    fn 完整性校验要求版本与入口同时存在() {
        let root = scratch("complete");
        write_runtime(&root, Some("0.1.1"));
        assert_eq!(runtime_version(&root).as_deref(), Some("0.1.1"));
        assert!(runtime_complete(&root));
        assert!(check_installed(&root).is_ok());

        fs::remove_file(entry_of(&root)).expect("remove entry");
        assert!(!runtime_complete(&root));
        assert!(check_installed(&root).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn 中断安装可从备份恢复() {
        let root = scratch("recover");
        fs::create_dir_all(&root).expect("root");
        let live = root.join("dsh-runtime");
        let backup = root.join("dsh-runtime-backup");

        // 场景：直装 live 中途崩溃，备份还完整。
        write_runtime(&backup, Some("0.1.0"));
        fs::create_dir_all(live.join("node_modules")).expect("残骸");
        fs::write(
            root.join("dsh-install.json"),
            format!(r#"{{"schema":1,"package":"{PACKAGE}","spec":"{LATEST_SPEC}"}}"#),
        )
        .expect("journal");

        assert!(recover_interrupted_install(&live, &backup).expect("recover"));
        assert!(runtime_complete(&live));
        assert!(!backup.exists());
        assert!(!root.join("dsh-install.json").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn 安装成功后中断只剩清理() {
        let root = scratch("cleanup");
        fs::create_dir_all(&root).expect("root");
        let live = root.join("dsh-runtime");
        let backup = root.join("dsh-runtime-backup");

        // 场景：live 已装好、备份未删、journal 未清时崩溃。
        write_runtime(&live, Some("0.2.0"));
        write_runtime(&backup, Some("0.1.0"));
        fs::write(
            root.join("dsh-install.json"),
            format!(r#"{{"schema":1,"package":"{PACKAGE}","spec":"{LATEST_SPEC}"}}"#),
        )
        .expect("journal");

        assert!(recover_interrupted_install(&live, &backup).expect("recover"));
        // live 保留新版本，备份被清掉。
        assert_eq!(runtime_version(&live).as_deref(), Some("0.2.0"));
        assert!(!backup.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn 无标记时不做任何恢复() {
        let root = scratch("noop");
        fs::create_dir_all(&root).expect("root");
        assert!(
            !recover_interrupted_install(&root.join("live"), &root.join("backup"))
                .expect("recover")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn 递归清理拒绝意外文件() {
        let path = std::env::temp_dir().join(format!("qianxun-cleanup-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        fs::write(&path, "not a directory").expect("file");
        assert!(remove_dir_if_exists(&path).is_err());
        assert!(path.is_file(), "被拒绝的目标必须原样保留");
        fs::remove_file(path).expect("cleanup");
    }
}

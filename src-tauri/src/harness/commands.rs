//! 托管域的 IPC 命令。命令函数只做「取状态 → 调域逻辑 → 返回」，
//! 重逻辑都在 harness 模块内部（编码规范 §2）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use super::install;
use super::supervisor::{Status, Stream, Supervisor};
use super::{Environment, InstallProgress};
use crate::error::{Error, Result};

/// 托管域自身的运行状态：supervisor + 安装互斥。
/// 与外壳设置（crate::AppState）分开管理，职责不同。
pub struct HarnessState {
    pub supervisor: Arc<Supervisor>,
    /// 安装进行中的标志：第二个点击不能对同一目录再起一个 npm。
    installing: AtomicBool,
    /// 串行化所有会观察或替换运行时的操作（安装前停机等）。
    lifecycle: Mutex<()>,
    /// 当前运行的 DSH 是否来自最小安全 profile（08 设计 §11）。
    /// 启动默认 profile 时复位；market 域据此冻结插件变更。
    safe_mode: AtomicBool,
}

impl HarnessState {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self {
            supervisor,
            installing: AtomicBool::new(false),
            lifecycle: Mutex::new(()),
            safe_mode: AtomicBool::new(false),
        }
    }
}

/// market 域冻结检查用（08 设计 §11.3-4）。
pub fn safe_mode_active(app: &AppHandle) -> bool {
    app.state::<crate::AppState>()
        .harness
        .safe_mode
        .load(Ordering::SeqCst)
}

/// 一行 DSH 输出（日志面板的数据形状）。
#[derive(Serialize)]
pub struct LogLine {
    pub stream: Stream,
    pub line: String,
}

/// 这台机器能跑什么、缺什么。
#[tauri::command]
pub async fn harness_environment(app: AppHandle) -> Result<Environment> {
    let settings = crate::settings_snapshot(&app)?;
    // 探测会 spawn 多个 node --version，别占着异步线程池的同步线程。
    tauri::async_runtime::spawn_blocking(move || super::environment(&app, &settings))
        .await
        .map_err(|cause| Error::Install(format!("环境探测没有完成：{cause}")))
}

#[tauri::command]
pub fn harness_status(state: State<'_, crate::AppState>) -> Status {
    state.harness.supervisor.status()
}

/// DSH 页 iframe 应加载的回环入口地址（网关回环端，端口按构建模式
/// 默认 release 23090 / debug 23091）。
/// DSH 0.1.2 的 Strict cookie 在跨站 iframe 里不可携带（401 死循环），
/// iframe 一律走本机网关的回环端：cookie 由服务端持有（见 dsh_upstream
/// 模块文档）。None = 网关尚未监听成功（前端给出明确提示，不静默回退直连）。
#[tauri::command]
pub fn harness_proxy_url(state: State<'_, crate::AppState>) -> Option<String> {
    let running = state
        .remote
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    running
        .as_ref()
        .map(|handle| format!("http://{}", handle.loopback_addr))
}

/// 启动 DSH 并返回它服务的 origin。
#[tauri::command]
pub async fn harness_start(app: AppHandle) -> Result<String> {
    start_managed(&app).await
}

#[tauri::command]
pub async fn harness_stop(app: AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    let _gate = state.harness.lifecycle.lock().await;
    let supervisor = Arc::clone(&state.harness.supervisor);
    supervisor.note(Stream::Stdout, "[DSH] 停止".to_owned());
    supervisor.stop().await;
    supervisor.note(Stream::Stdout, "[DSH] 已停止".to_owned());
    Ok(())
}

/// 重启 DSH：按当前设置先停（等监督循环退出）再拉起；未运行时等价启动。
/// 桥/设置/版本变化后的「重启生效」都走这里，与生命周期闸门串行。
/// 复用 start_locked：失败时同样享受 known-good 一级自救（08 设计 §11.2）。
#[tauri::command]
pub async fn harness_restart(app: AppHandle) -> Result<String> {
    let state = app.state::<crate::AppState>();
    let _gate = state.harness.lifecycle.lock().await;
    let supervisor = Arc::clone(&state.harness.supervisor);
    supervisor.note(Stream::Stdout, "[DSH] 重启".to_owned());
    supervisor.stop().await;
    supervisor.wait_until_inactive().await?;
    start_locked(&app).await
}

/// 安装（或重装）DSH。pnpm 的每一行输出都通过日志事件实时转发，
/// 包数推进另发进度事件（前端 store 留存备用，UI 展示走日志）。
#[tauri::command]
pub async fn harness_install(app: AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    if state.harness.installing.swap(true, Ordering::SeqCst) {
        return Err(Error::AlreadyInstalling);
    }
    let progress = super::progress_sink(&app);
    let outcome = perform_install(&app).await;
    state.harness.installing.store(false, Ordering::SeqCst);
    progress(InstallProgress::Done);

    match &outcome {
        Ok(()) => {
            state
                .harness
                .supervisor
                .note(Stream::Stdout, format!("{} 安装完成", install::PACKAGE));
            // 安全 profile 预热（08 设计 §11.4）：首次安装成功后就地建好。
            ensure_safe_profile_warm(&app);
        }
        Err(failure) => state
            .harness
            .supervisor
            .note(Stream::Stderr, failure.to_string()),
    }
    outcome
}

/// 一键安装千寻自带的 Node（curl 下载 + SHA-256 校验 + 解压到 node/）。
/// 返回装好（或本就满足）的版本号。
#[tauri::command]
pub async fn harness_install_node(app: AppHandle) -> Result<String> {
    let state = app.state::<crate::AppState>();
    if state.harness.installing.swap(true, Ordering::SeqCst) {
        return Err(Error::AlreadyInstalling);
    }
    let supervisor = Arc::clone(&state.harness.supervisor);
    supervisor.note(Stream::Stdout, "开始安装 Node 运行时".to_owned());
    let settings = crate::settings_snapshot(&app)?;
    let managed_dir = crate::paths::managed_node_dir(&app)?;
    let progress = super::progress_sink(&app);
    let outcome = super::node_install::install(
        &settings,
        &managed_dir,
        {
            let supervisor = Arc::clone(&supervisor);
            move |stream, line| supervisor.note(stream, line)
        },
        progress.clone(),
    )
    .await;
    state.harness.installing.store(false, Ordering::SeqCst);
    progress(InstallProgress::Done);
    match &outcome {
        Ok(version) => {
            if let Some(version) = version {
                supervisor.note(Stream::Stdout, format!("Node {version} 就绪"));
            } else {
                supervisor.note(Stream::Stdout, "Node 已满足要求，无需安装".to_owned());
            }
        }
        Err(failure) => supervisor.note(Stream::Stderr, failure.to_string()),
    }
    outcome.map(|version| version.unwrap_or_else(|| "已满足".to_owned()))
}

async fn perform_install(app: &AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    let _gate = state.harness.lifecycle.lock().await;
    // 晋升要 rename live 目录：先停机并等监督循环退出，
    // 否则它可能对刚被挪走的目录重启子进程。
    state.harness.supervisor.stop().await;
    state.harness.supervisor.wait_until_inactive().await?;

    let settings = crate::settings_snapshot(app)?;
    let plan = super::install_plan(app, &settings)?;
    let supervisor = Arc::clone(&state.harness.supervisor);
    supervisor.note(
        Stream::Stdout,
        format!("正在安装 {} 到 {}", plan.spec, plan.target.display()),
    );

    // 先把 pnpm 工具刷新到最新（失败回落已装版本），再走事务安装（备份 → 直装 → 校验 → 清理）。
    let tool_reporter = Arc::clone(&supervisor);
    install::ensure_pnpm_tool(&plan, move |stream, line| tool_reporter.note(stream, line)).await?;
    // pnpm 的 Progress 行顺便解析成进度事件：环境页能看到包数推进。
    let reporter = Arc::clone(&supervisor);
    let dsh_progress = super::progress_sink(app);
    let registry = plan.registry.clone();
    install::run_transactional(&plan, move |stream, line| {
        for event in super::parse_dsh_progress(&line, &registry) {
            dsh_progress(event);
        }
        reporter.note(stream, line);
    })
    .await?;

    // pnpm 可能成功退出却装出别的东西——信文件不信退出码。
    // ADR-015：完整性校验之外还要校验版本号 = PINNED_VERSION；否则 pnpm 复用
    // 旧版本时会被误判为「OK」，下次启动还会拒绝 spawn。
    install::check_installed_at_version(&crate::paths::harness_dir(app)?)?;
    Ok(())
}

/// 启动缓冲至今的输出，晚打开的日志面板不至于空白。
#[tauri::command]
pub fn harness_log(state: State<'_, crate::AppState>) -> Vec<LogLine> {
    state
        .harness
        .supervisor
        .recent_log()
        .into_iter()
        .map(|(stream, line)| LogLine { stream, line })
        .collect()
}

/// 唯一的启动入口：托盘与 IPC 都走这里，不绕过生命周期闸门。
/// （设置里的 autostart、托盘菜单、控制台按钮共用。）
pub async fn start_managed(app: &AppHandle) -> Result<String> {
    let state = app.state::<crate::AppState>();
    let _gate = state.harness.lifecycle.lock().await;
    start_locked(app).await
}

/// 持有生命周期闸门后的启动路径（start_managed 与 harness_restart 共用）。
///
/// 内含启动失败的一级自救（08 设计 §11.2）：默认 profile 启动失败且
/// known-good 快照与当前 manifest 不同 → 自动恢复快照重试一次。快照相同
/// 说明 manifest 不是故障原因，重试没有意义，直接报原始错误。
async fn start_locked(app: &AppHandle) -> Result<String> {
    let state = app.state::<crate::AppState>();
    let settings = crate::settings_snapshot(app)?;
    let profile_dir = default_profile_dir(app, &settings);
    let plan = super::launch_plan_for(app, &settings, super::DEFAULT_PROFILE)?;
    state
        .harness
        .supervisor
        .note(Stream::Stdout, "[DSH] 启动".to_owned());
    state.harness.safe_mode.store(false, Ordering::SeqCst);
    match Arc::clone(&state.harness.supervisor).start(plan).await {
        Ok(origin) => {
            after_successful_start(app, &profile_dir);
            Ok(origin)
        }
        Err(failure) => {
            // 一级自救：known-good 快照恢复 + 重试一次。
            if !super::recovery::snapshot_differs(&profile_dir) {
                return Err(failure);
            }
            state
                .harness
                .supervisor
                .note(Stream::Stderr, format!("[DSH] 启动失败：{failure}"));
            state.harness.supervisor.note(
                Stream::Stderr,
                "[DSH] 尝试恢复到上次能启动的配置…".to_owned(),
            );
            let summary = match super::recovery::recover(&profile_dir) {
                Ok(summary) => summary,
                Err(cause) => {
                    state
                        .harness
                        .supervisor
                        .note(Stream::Stderr, format!("[DSH] 快照恢复失败：{cause}"));
                    return Err(failure);
                }
            };
            if !summary.removed_plugins.is_empty() {
                state.harness.supervisor.note(
                    Stream::Stderr,
                    format!(
                        "[DSH] 已从装配清单卸下：{}（文件保留，可重新安装）",
                        summary.removed_plugins.join("、")
                    ),
                );
            }
            let retry_plan = super::launch_plan_for(app, &settings, super::DEFAULT_PROFILE)?;
            match Arc::clone(&state.harness.supervisor)
                .start(retry_plan)
                .await
            {
                Ok(origin) => {
                    state
                        .harness
                        .supervisor
                        .note(Stream::Stdout, "[DSH] 已用已知良好配置启动成功".to_owned());
                    after_successful_start(app, &profile_dir);
                    Ok(origin)
                }
                Err(retry_failure) => {
                    state.harness.supervisor.note(
                        Stream::Stderr,
                        format!("[DSH] 已知良好配置也无法启动：{retry_failure}"),
                    );
                    Err(retry_failure)
                }
            }
        }
    }
}

/// 成功启动默认 profile 之后的两件家务（都只记 warn，不阻断）：
/// ① 快照当前 manifest 为 known-good；② 校验安全 profile 仍在（预热）。
fn after_successful_start(app: &AppHandle, profile_dir: &std::path::Path) {
    if let Err(cause) = super::recovery::write_snapshot(profile_dir) {
        crate::logging::log("warn", &format!("[DSH] known-good 快照写入失败：{cause}"));
    }
    ensure_safe_profile_warm(app);
}

/// 安全 profile 预热（08 设计 §11.4）：缺失即静默重建。它没有依赖要装
/// （内核包随运行时落位），真正需要逃生舱时不应有等待。
fn ensure_safe_profile_warm(app: &AppHandle) {
    let Ok(settings) = crate::settings_snapshot(app) else {
        return;
    };
    let profiles_dir = super::dsh_home(app, &settings).join("profiles");
    if super::recovery::safe_profile_ready(&profiles_dir) {
        return;
    }
    if let Err(cause) = super::recovery::prepare_safe_profile(&profiles_dir) {
        crate::logging::log("warn", &format!("[DSH] 安全 profile 预热失败：{cause}"));
    }
}

fn default_profile_dir(
    app: &AppHandle,
    settings: &crate::settings::Settings,
) -> std::path::PathBuf {
    super::dsh_home(app, settings)
        .join("profiles")
        .join(super::DEFAULT_PROFILE)
}

/// 恢复状态速览（前端决定失败态显示哪些按钮）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    /// known-good 快照是否存在（失败态「恢复」按钮的显示条件）。
    pub snapshot_available: bool,
    /// 安全 profile 是否就绪。
    pub safe_profile_ready: bool,
    /// 当前运行的 DSH 是否来自安全 profile。
    pub safe_mode: bool,
}

#[tauri::command]
pub fn harness_recovery_status(app: AppHandle) -> Result<RecoveryStatus> {
    let settings = crate::settings_snapshot(&app)?;
    let profiles_dir = super::dsh_home(&app, &settings).join("profiles");
    Ok(RecoveryStatus {
        snapshot_available: super::recovery::snapshot_exists(&default_profile_dir(&app, &settings)),
        safe_profile_ready: super::recovery::safe_profile_ready(&profiles_dir),
        safe_mode: safe_mode_active(&app),
    })
}

/// 手动恢复到上次能启动的配置（08 设计 §11.5）：停机 → 恢复快照 →
/// 以默认 profile 重新启动。返回卸下的插件名（前端展示恢复摘要）。
#[tauri::command]
pub async fn harness_recover_known_good(app: AppHandle) -> Result<Vec<String>> {
    let state = app.state::<crate::AppState>();
    let _gate = state.harness.lifecycle.lock().await;
    let supervisor = Arc::clone(&state.harness.supervisor);
    supervisor.stop().await;
    supervisor.wait_until_inactive().await?;

    let settings = crate::settings_snapshot(&app)?;
    let profile_dir = default_profile_dir(&app, &settings);
    let summary = super::recovery::recover(&profile_dir)?;
    if !summary.removed_plugins.is_empty() {
        supervisor.note(
            Stream::Stderr,
            format!(
                "[DSH] 已从装配清单卸下：{}（文件保留，可重新安装）",
                summary.removed_plugins.join("、")
            ),
        );
    }
    start_locked(&app).await?;
    Ok(summary.removed_plugins)
}

/// 以最小安全 profile 启动（08 设计 §11.4）：用户默认 profile 一个字节不动。
/// 安全 profile 启动成功**不写** known-good 快照（规则 §11.3-2）。
#[tauri::command]
pub async fn harness_safe_mode_start(app: AppHandle) -> Result<String> {
    let state = app.state::<crate::AppState>();
    let _gate = state.harness.lifecycle.lock().await;
    let supervisor = Arc::clone(&state.harness.supervisor);
    supervisor.stop().await;
    supervisor.wait_until_inactive().await?;

    let settings = crate::settings_snapshot(&app)?;
    let profiles_dir = super::dsh_home(&app, &settings).join("profiles");
    super::recovery::prepare_safe_profile(&profiles_dir)?;
    let plan = super::launch_plan_for(&app, &settings, super::recovery::SAFE_PROFILE)?;
    supervisor.note(
        Stream::Stdout,
        "[DSH] 安全模式启动：默认 profile 不受影响".to_owned(),
    );
    state.harness.safe_mode.store(true, Ordering::SeqCst);
    supervisor.start(plan).await
}

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
}

impl HarnessState {
    pub fn new(supervisor: Arc<Supervisor>) -> Self {
        Self {
            supervisor,
            installing: AtomicBool::new(false),
            lifecycle: Mutex::new(()),
        }
    }
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
    state.harness.supervisor.stop().await;
    Ok(())
}

/// 安装（或重装）DSH。pnpm 的每一行输出都通过日志事件实时转发，
/// 包数推进经进度事件驱动环境页进度卡。
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
        Ok(()) => state
            .harness
            .supervisor
            .note(Stream::Stdout, format!("{} 安装完成", install::PACKAGE)),
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

    // 先幂等备好 pnpm 工具，再走事务安装（备份 → 直装 → 校验 → 清理）。
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
    install::check_installed(&crate::paths::harness_dir(app)?)?;
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
    let settings = crate::settings_snapshot(app)?;
    let plan = super::launch_plan(app, &settings)?;
    Arc::clone(&state.harness.supervisor).start(plan).await
}

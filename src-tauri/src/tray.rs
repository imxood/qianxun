//! 托盘：回到窗口、DSH 启停、真正退出三条路；tooltip 实时反映 DSH
//! 运行状态。左键单击 = 显示窗口，右键 = 菜单。

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::supervisor::Status;
use crate::window;

/// 状态变化时更新 tooltip 的入口（持有托盘句柄）。
static TRAY: std::sync::OnceLock<TrayIcon> = std::sync::OnceLock::new();

pub fn build(app: &AppHandle) -> Result<()> {
    let show = MenuItem::with_id(app, "show", "显示千寻", true, None::<&str>)
        .map_err(|error| Error::Tray(error.to_string()))?;
    let snip = MenuItem::with_id(app, "snip", "截图", true, None::<&str>)
        .map_err(|error| Error::Tray(error.to_string()))?;
    let start = MenuItem::with_id(app, "start", "启动 DSH", true, None::<&str>)
        .map_err(|error| Error::Tray(error.to_string()))?;
    let stop = MenuItem::with_id(app, "stop", "停止 DSH", true, None::<&str>)
        .map_err(|error| Error::Tray(error.to_string()))?;
    let separator =
        PredefinedMenuItem::separator(app).map_err(|error| Error::Tray(error.to_string()))?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)
        .map_err(|error| Error::Tray(error.to_string()))?;
    let menu = Menu::with_items(
        app,
        &[&show, &snip, &separator, &start, &stop, &separator, &quit],
    )
    .map_err(|error| Error::Tray(error.to_string()))?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(
            app.default_window_icon()
                .cloned()
                .ok_or_else(|| Error::Tray("应用图标不可用".to_owned()))?,
        )
        .tooltip("千寻 · DSH 未运行")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(front) = window::front(app) {
                    window::reveal(&front);
                }
            }
            "snip" => {
                // 鼠标路：与全局热键同一条 start_session 流水线。
                crate::shots::commands::start_session(app);
            }
            "start" => {
                let handle = app.clone();
                // 启动是异步全过程（就绪等待可长达 120s），托盘回调里
                // 只能派发任务，结果通过状态事件回到 UI 与 tooltip。
                tauri::async_runtime::spawn(async move {
                    if let Err(failure) = crate::harness::commands::start_managed(&handle).await {
                        crate::logging::log("warn", &format!("托盘启动 DSH 失败：{failure}"));
                    }
                });
            }
            "stop" => {
                let state = app.state::<crate::AppState>();
                let supervisor = state.harness.supervisor.clone();
                tauri::async_runtime::spawn(async move {
                    supervisor.stop().await;
                });
            }
            "quit" => {
                // 窗口可能从未经过 CloseRequested（例如一直隐藏），
                // 退出前补一次几何快照。
                window::persist_geometry(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(front) = window::front(tray.app_handle()) {
                    window::reveal(&front);
                }
            }
        })
        .build(app)
        .map_err(|error| Error::Tray(error.to_string()))?;
    let _ = TRAY.set(tray);
    Ok(())
}

/// DSH 状态变化 → 托盘 tooltip。失败静默：tooltip 不是关键路径。
pub fn reflect_status(status: &Status) {
    let Some(tray) = TRAY.get() else {
        return;
    };
    let text: String = match status {
        Status::Stopped => "千寻 · DSH 未运行".to_owned(),
        Status::Starting => "千寻 · DSH 启动中…".to_owned(),
        Status::Ready { origin, .. } => {
            // origin 形如 http://127.0.0.1:17300；tooltip 里只留端口更可读。
            let port = origin.rsplit(':').next().unwrap_or("?");
            format!("千寻 · DSH 运行于 :{port}")
        }
        Status::Restarting { attempt, .. } => {
            format!("千寻 · DSH 重启中（第 {attempt} 次）")
        }
        Status::Failed { .. } => "千寻 · DSH 启动失败".to_owned(),
    };
    let _ = tray.set_tooltip(Some(&text));
}

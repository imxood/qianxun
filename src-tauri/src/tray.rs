//! 托盘：回到窗口、重建界面、截图、DSH 启停、真正退出；tooltip 实时
//! 反映 DSH 运行状态。左键单击 = 显示窗口，右键 = 菜单。

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::supervisor::Status;
use crate::window;

/// 状态变化时更新 tooltip 的入口（持有托盘句柄）。
static TRAY: std::sync::OnceLock<TrayIcon> = std::sync::OnceLock::new();

/// 「退出 UI」菜单句柄：轻量模式下置灰（UI 已不在，无可退出）。
static EXIT_UI: std::sync::OnceLock<tauri::menu::MenuItem<tauri::Wry>> = std::sync::OnceLock::new();

/// 轻量模式切换 → 菜单可用态。失败静默：菜单态不是关键路径。
pub fn reflect_light_mode(light: bool) {
    if let Some(item) = EXIT_UI.get() {
        let _ = item.set_enabled(!light);
    }
}

pub fn build(app: &AppHandle) -> Result<()> {
    // 「进入 UI」双重语义：常态 = 显示/聚焦已有主窗；轻量模式 = 重建
    // webview 回到 UI（rebuild_main 对「无旧窗」有直建分支）。
    let show = MenuItem::with_id(app, "enter-ui", "进入 UI", true, None::<&str>)
        .map_err(|error| Error::Tray(error.to_string()))?;
    // 退出 UI（轻量模式）：销毁主窗 webview，进程只留核心——托盘、
    // DSH supervisor、网关、截屏与侧车管理全部照常。是 UI 异常/资源
    // 紧张时的逃生门（对齐 clash-verge-rev 的轻量模式）。
    let exit_ui = MenuItem::with_id(app, "exit-ui", "退出 UI", true, None::<&str>)
        .map_err(|error| Error::Tray(error.to_string()))?;
    let _ = EXIT_UI.set(exit_ui.clone());
    // 重建界面: webview 白屏/显示异常时销毁主窗的 WebView2 实例并整窗
    // 重建（新 controller + 渲染进程；supervisor 与后端零扰动）。页面级
    // reload 修不了 WebView2 层的挂死——reload 是 fire-and-forget，往
    // 已挂死的进程里投递指令会返回 Ok 却无人执行，白屏依旧。
    let rebuild_ui = MenuItem::with_id(app, "rebuild-ui", "重建界面", true, None::<&str>)
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
        &[
            &show,
            &exit_ui,
            &rebuild_ui,
            &snip,
            &separator,
            &start,
            &stop,
            &separator,
            &quit,
        ],
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
            "enter-ui" => {
                if window::is_light_mode() {
                    // 轻量模式：重建 webview 回到 UI（异步全过程，
                    // 成败落日志，同「重建界面」路径）。
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(failure) = window::exit_light_mode(&handle).await {
                            crate::logging::log("warn", &format!("托盘进入 UI 失败：{failure}"));
                        }
                    });
                } else if let Some(front) = window::front(app) {
                    window::reveal(&front);
                }
            }
            "exit-ui" => {
                if let Err(failure) = window::enter_light_mode(app) {
                    crate::logging::log("warn", &format!("托盘退出 UI 失败：{failure}"));
                }
            }
            "rebuild-ui" => {
                // 重建是异步全过程（destroy → 等 label 释放 → build →
                // 还原几何），托盘回调里只能派发任务，成败都落日志。
                // 只能由 Rust 侧发起：白屏时页面 JS 可能已死，invoke
                // 不可用。
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(failure) = window::rebuild_main(&handle).await {
                        crate::logging::log("warn", &format!("托盘重建界面失败：{failure}"));
                    }
                });
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
                let app = tray.app_handle();
                if window::is_light_mode() {
                    // 左键在轻量模式下同样是「进入 UI」。
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(failure) = window::exit_light_mode(&handle).await {
                            crate::logging::log("warn", &format!("托盘进入 UI 失败：{failure}"));
                        }
                    });
                } else if let Some(front) = window::front(app) {
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
    let light = if crate::window::is_light_mode() {
        "（轻量）"
    } else {
        ""
    };
    let text: String = match status {
        Status::Stopped => format!("千寻{light} · DSH 未运行"),
        Status::Starting => format!("千寻{light} · DSH 启动中…"),
        Status::Ready { origin, .. } => {
            // origin 形如 http://127.0.0.1:17300；tooltip 里只留端口更可读。
            let port = origin.rsplit(':').next().unwrap_or("?");
            format!("千寻{light} · DSH 运行于 :{port}")
        }
        Status::Restarting { attempt, .. } => {
            format!("千寻{light} · DSH 重启中（第 {attempt} 次）")
        }
        Status::Failed { .. } => format!("千寻{light} · DSH 启动失败"),
    };
    let _ = tray.set_tooltip(Some(&text));
}

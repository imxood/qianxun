//! 千寻 —— 个人开发者的工作台：管理 DSH、承载工具、连接远程。
//!
//! 装配：设置系统、窗口记忆、托盘、单实例，DSH 的检测/安装/supervisor
//! 托管与事件转发，以及搜索域（fff-search 索引 + 文件名/内容搜索）。

mod atomic;
mod bridge;
mod child_output;
mod dsh_upstream;
mod error;
mod harness;
mod logging;
mod notes;
mod paths;
mod remote;
mod search;
mod settings;
mod shots;
mod sync;
mod terminal;
mod tray;
mod window;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use harness::supervisor::{Event, Supervisor};
use settings::Settings;

/// 全局可变状态。设置的唯一持有者（IPC 读写都经过它），
/// 加上托管域自己的 supervisor/安装状态与搜索域索引。
/// 截屏域的 ShotsState 独立 manage（commands 直接按类型取）。
struct AppState {
    settings: Mutex<Settings>,
    harness: Arc<harness::commands::HarnessState>,
    search: Arc<search::SearchState>,
    remote: Arc<remote::commands::RemoteState>,
}

/// 读设置快照：托管的启动/安装计划都以它为准。
/// 拿锁只做 clone，不做任何重活（编码规范 §6）。
fn settings_snapshot(app: &AppHandle) -> error::Result<Settings> {
    let state = app.state::<AppState>();
    // 先绑定再返回：锁守卫是 tail 表达式临时值时活得比 state 长，会误报。
    let snapshot = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    Ok(snapshot)
}

/// 前端监听 supervisor 状态与日志的通道。
const EVENT_CHANNEL: &str = "harness://event";

/// IPC：应用元信息。状态栏与关于页的唯一版本来源。
#[tauri::command]
fn app_meta(app: tauri::AppHandle) -> AppMeta {
    let package = app.package_info();
    AppMeta {
        name: package.name.clone(),
        version: package.version.to_string(),
        // PackageInfo 没有 identifier 字段，标识符在运行时配置里。
        identifier: app.config().identifier.clone(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppMeta {
    name: String,
    version: String,
    identifier: String,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二次启动只唤醒已运行的实例。千寻托管着 DSH，
            // 两个实例同时拉起服务会互相打架——从第一天就挡住。
            if let Some(existing) = window::front(app) {
                window::reveal(&existing);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        // 原生目录选择器（搜索页选根目录，替代手输绝对路径）。
        .plugin(tauri_plugin_dialog::init())
        // 截屏热键：注册在 Rust 侧（设置驱动），触发即 capture + 拉起每屏覆盖窗。
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        shots::commands::start_session(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle();

            logging::init(paths::log_path(handle)?);
            let loaded = settings::load(handle)?;
            let autostart = loaded.dsh.autostart;
            if let Some(geometry) = loaded.window.geometry.clone() {
                window::restore(handle, &geometry);
            }
            if loaded.window.start_minimized {
                if let Some(front) = window::front(handle) {
                    let _ = front.hide();
                }
            }

            // 兜底亮窗：主窗以隐藏创建（tauri.conf visible:false），正常
            // 由前端就绪后亮窗；若前端迟迟没起来（如 dev server 未就绪），
            // 20 秒后强制亮出，宁可看到错误页也不让应用「隐身」。
            {
                let handle = handle.clone();
                let start_minimized = loaded.window.start_minimized;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                    if start_minimized {
                        return;
                    }
                    if let Some(front) = window::front(&handle) {
                        if !front.is_visible().unwrap_or(true) {
                            let _ = front.show();
                            let _ = front.set_focus();
                        }
                    }
                });
            }

            let supervisor = Supervisor::new()?;
            app.manage(AppState {
                settings: Mutex::new(loaded),
                harness: Arc::new(harness::commands::HarnessState::new(Arc::clone(
                    &supervisor,
                ))),
                search: Arc::new(search::SearchState::new()),
                remote: Arc::new(remote::commands::RemoteState::default()),
            });
            app.manage(shots::commands::ShotsState::default());
            app.manage(terminal::commands::TerminalState::default());
            forward_events(handle, &supervisor);
            // 远程/回环双端网关：setup 即占位监听回环网关端口（默认
            // release 23090 / debug 23091，DSH 页 iframe 立刻有稳定地址），
            // 启用远程时再额外绑 LAN。DSH 就绪事件由 forward_events
            // 同步触发，热更新上游。
            tauri::async_runtime::spawn(remote::commands::sync(handle.clone()));
            tray::build(handle)?;

            // 截屏热键随设置恢复（空串 = 不注册；失败不阻断启动，日志可见）。
            {
                let accel = settings_snapshot(handle)?.hotkeys.screenshot;
                if !accel.is_empty() {
                    if let Err(failure) = shots::commands::set_hotkey_impl(handle, &accel) {
                        logging::log("warn", &format!("恢复截屏热键失败：{failure}"));
                    }
                }
            }

            // 桥自愈：部署过但插件文件被 DSH 重装清掉时静默补齐（M6）。
            bridge::commands::heal(handle);

            // 关闭 WebView2 浏览器加速键：Ctrl+Shift+C 不再误开 devtools
            // 元素选择器、Ctrl+Shift+V 不再触发「原样粘贴」（双重粘贴的
            // 元凶）。同时把主题色方案设为 AUTO（prefers-color-scheme
            // 跟随 OS，「跟随系统」主题才能在暗色系统下生效）。
            // 开发者工具保留：F12 / Ctrl+Shift+I 由前端显式开关。
            #[cfg(windows)]
            if let Some(main) = app.get_webview_window("main") {
                window::apply_webview_preferences(&main);
            }

            // 远程网关：设置里启用过就恢复监听（上游 origin 等 DSH 就绪事件补）。
            tauri::async_runtime::spawn(remote::commands::sync(handle.clone()));

            if autostart {
                // 自启在后台走：窗口先见面，DSH 就绪与否由状态事件汇报。
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(failure) = harness::commands::start_managed(&handle).await {
                        logging::log("warn", &format!("自启 DSH 失败：{failure}"));
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            let label = window.label().to_owned();
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // 「关到托盘」只约束主窗；覆盖窗/贴图窗的关闭就是关闭。
                    if label == "main" {
                        window::on_close_requested(window, api);
                    } else if label.starts_with(window::STANDALONE_PREFIX) {
                        // 独立窗口：转交前端确认（有活动终端时弹窗），
                        // 确认后走 window_force_close 真正销毁。
                        window::on_standalone_close_requested(window, api);
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    if label.starts_with(shots::commands::OVERLAY_LABEL_PREFIX) {
                        shots::commands::overlay_closed(window.app_handle());
                    }
                    if label.starts_with(window::STANDALONE_PREFIX) {
                        window::on_standalone_destroyed(window.app_handle(), &label);
                    }
                }
                tauri::WindowEvent::ThemeChanged(_) => {
                    // OS 深浅色切换：重设 webview 配色 + 广播事件，
                    // 「跟随系统」主题立即切换。
                    window::on_theme_changed(window.app_handle());
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_meta,
            settings::commands::settings_get,
            settings::commands::settings_update,
            harness::commands::harness_environment,
            harness::commands::harness_status,
            harness::commands::harness_proxy_url,
            harness::commands::harness_start,
            harness::commands::harness_stop,
            harness::commands::harness_install,
            harness::commands::harness_install_node,
            harness::commands::harness_log,
            search::commands::search_open,
            search::commands::search_status,
            search::commands::search_files,
            search::commands::search_content,
            search::commands::search_cancel,
            search::commands::search_wait_ready,
            search::commands::search_list_drives,
            shots::commands::shots_capture,
            shots::commands::shots_overlay_ready,
            shots::commands::shots_set_hotkey,
            shots::commands::shots_clear_hotkey,
            shots::commands::shots_copy_clipboard,
            shots::commands::shots_save,
            shots::commands::shots_pin,
            shots::commands::shots_close_overlays,
            shots::commands::shots_open_pin,
            window::app_toggle_devtools,
            window::system_theme,
            window::window_spawn_view,
            window::window_reveal_main,
            window::window_force_close,
            terminal::commands::terminal_spawn,
            terminal::commands::terminal_write,
            terminal::commands::terminal_resize,
            terminal::commands::terminal_kill,
            terminal::commands::terminal_replay,
            terminal::commands::terminal_clear,
            terminal::commands::terminal_sessions,
            terminal::commands::terminal_transfer,
            terminal::commands::terminal_pin,
            terminal::commands::terminal_unpin,
            terminal::commands::terminal_pin_resume,
            terminal::commands::terminal_pinned_list,
            terminal::commands::terminal_pinned_replay,
            notes::commands::notes_list,
            notes::commands::notes_read,
            notes::commands::notes_save,
            notes::commands::notes_create,
            notes::commands::notes_delete,
            notes::commands::notes_init,
            bridge::commands::bridge_deploy,
            bridge::commands::bridge_status,
            remote::commands::remote_interfaces,
            remote::commands::remote_status,
            remote::commands::remote_pair,
            remote::commands::remote_revoke,
            remote::commands::remote_self_check,
            sync::commands::sync_status,
            sync::commands::sync_init,
            sync::commands::sync_pull,
            sync::commands::sync_push,
        ])
        .run(tauri::generate_context!())
        .expect("千寻主循环异常退出");
}

/// 把 supervisor 事件转发给前端（emit）与托盘（状态反射）。
/// 单独一个任务常驻：broadcast 接收端不活跃时事件自然丢弃，不积压。
fn forward_events(app: &AppHandle, supervisor: &Arc<Supervisor>) {
    let handle = app.clone();
    let mut events = supervisor.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let Ok(event) = events.recv().await else {
                // 发送端随 supervisor 存活到进程结束；循环退出即异常路径。
                break;
            };
            match &event {
                Event::Status(status) => {
                    tray::reflect_status(status);
                    // DSH 就绪/停止触发网关同步：上游 origin 出现或消失，
                    // 回环 iframe 与 LAN 设备共享同一套更新。
                    if matches!(
                        status,
                        harness::supervisor::Status::Ready { .. }
                            | harness::supervisor::Status::Stopped
                    ) {
                        tauri::async_runtime::spawn(remote::commands::sync(handle.clone()));
                    }
                    let _ = handle.emit(EVENT_CHANNEL, &event);
                }
                Event::Log { .. } => {
                    let _ = handle.emit(EVENT_CHANNEL, &event);
                }
            }
        }
    });
}

//! 窗口域：前置窗口查找、显示/聚焦、几何记忆（恢复与持久化）。
//!
//! 几何记忆采用「关闭/退出时一次性快照」而非每次移动都写盘：
//! 设置文件不值得为窗口拖动承受成倍的写入（架构 §4.1 的原子写代价）。

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::paths;
use crate::settings::{self, Geometry};
use crate::{logging, AppState};

pub fn front(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

pub fn reveal(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

pub fn restore(app: &AppHandle, geometry: &Geometry) {
    let Some(window) = front(app) else {
        return;
    };
    // 顺序有意为之：先尺寸后位置；最大化必须最后——否则
    // set_position 会把窗口从最大化状态拉回来。
    let _ = window.set_size(PhysicalSize::new(geometry.width, geometry.height));
    if geometry.maximized {
        let _ = window.maximize();
    } else {
        let _ = window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
    }
}

/// 当前几何快照。最大化时 outer_position 仍是还原锚点，直接记录即可。
pub fn capture(app: &AppHandle) -> Option<Geometry> {
    let window = front(app)?;
    let maximized = window.is_maximized().ok()?;
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(Geometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        maximized,
    })
}

/// 关闭请求的裁决：先无条件持久化几何，再按设置决定隐藏还是放行退出。
/// 这里不拿 Result：任何失败都不改变「用户想关窗」的语义，只记日志。
/// 入参用 &Window：on_window_event 回调给的就是它；操作集与 WebviewWindow 相同。
pub fn on_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    let app = window.app_handle();
    persist_geometry(app);

    // guard 先绑定再用：布尔值 Copy 出块，锁在块尾随 state 一起释放。
    let hide = {
        let state = app.state::<AppState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.window.close_to_tray
    };

    if hide {
        api.prevent_close();
        let _ = window.hide();
        logging::log("info", "窗口隐藏到托盘（托盘菜单可退出）");
    }
}

/// 几何变化才写盘，避免退出路径上无意义的重复 IO。
pub fn persist_geometry(app: &AppHandle) {
    let Some(geometry) = capture(app) else {
        return;
    };
    let Ok(path) = paths::settings_path(app) else {
        return;
    };
    let state = app.state::<AppState>();
    let mut guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.window.geometry.as_ref() != Some(&geometry) {
        guard.window.geometry = Some(geometry);
        if let Err(error) = settings::save(&path, &guard) {
            logging::log("error", &format!("窗口几何持久化失败：{error}"));
        }
    }
}

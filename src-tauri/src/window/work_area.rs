//! 工作区（work area）：窗口「最大化」时应当占住的矩形。
//!
//! 背景（远程 omarchy/Arch 实测，2026-11-13）：
//! - omarchy 左侧有一根 28px 的竖向工具栏。Hyprland 把它列为**保留区**
//!   （`hyprctl monitors -j` 里 `reserved: [28, 0, 0, 0]`），别的程序铺满
//!   整屏就会把它盖住。
//! - 但 GTK4/Chromium 报出来的可用区是**整块屏幕**：千寻 CEF webview 里
//!   `screen.availWidth=1920 / availLeft=0`，Tauri 的 `monitor.work_area()`
//!   同样是 1920×1080——两边都不知道 WM 保留区的存在。
//! - X11 也没法通用地查：`root` 上没有 `_NET_WORKAREA` 属性（实测 nitems=0）。
//!
//! 所以 Linux 下要直接问 WM。Hyprland 的 IPC socket 直接吐 JSON，
//! `j/monitors` 里带每个显示器的 `reserved`（left/right/top/bottom）。
//!
//! 取不到 WM（没装 Hyprland / socket 不在 / 超时）就退回 Tauri 的
//! `monitor.work_area()`：Windows/macOS 上它就是真工作区，Linux 上等于
//! 整屏（即本模块落地前的行为）。

use serde::Serialize;
use tauri::{PhysicalPosition, WebviewWindow};

/// 工作区矩形（物理像素）。
#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkArea {
    /// 左上角 X（物理像素，全局坐标）。
    pub x: i32,
    /// 左上角 Y（物理像素，全局坐标）。
    pub y: i32,
    /// 宽度（物理像素）。
    pub width: u32,
    /// 高度（物理像素）。
    pub height: u32,
}

/// 当前窗口所在显示器的可用工作区（物理像素）。
///
/// 找不到显示器 / 枚举失败时返回 `None`，由前端退回 `screen.avail*`。
pub async fn for_window(window: &WebviewWindow) -> Option<WorkArea> {
    let monitors = window.available_monitors().ok()?;
    if monitors.is_empty() {
        return None;
    }
    let origin = window
        .outer_position()
        .unwrap_or_else(|_| PhysicalPosition::new(0, 0));
    let monitor = pick_monitor(&monitors, origin)?;

    // 基准矩形：Tauri 自己认的工作区（物理像素）。
    let area = monitor.work_area();
    #[allow(unused_mut)]
    let mut work = WorkArea {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width,
        height: area.size.height,
    };

    // Linux：Chromium/GTK 都不知道 WM 保留区，向 WM 补一刀。
    #[cfg(target_os = "linux")]
    {
        if let Some(reserved) = hyprland::reserved_for(monitor).await {
            let left = reserved.left.max(0) as u32;
            let right = reserved.right.max(0) as u32;
            let top = reserved.top.max(0) as u32;
            let bottom = reserved.bottom.max(0) as u32;
            work.x += left as i32;
            work.y += top as i32;
            work.width = work.width.saturating_sub(left.saturating_add(right));
            work.height = work.height.saturating_sub(top.saturating_add(bottom));
        }
    }

    Some(work)
}

/// 选出窗口当前所在显示器：先按左上角命中，再按交叠面积最大。
fn pick_monitor(
    monitors: &[tauri::window::Monitor],
    origin: PhysicalPosition<i32>,
) -> Option<&tauri::window::Monitor> {
    if let Some(found) = monitors.iter().find(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        let x = position.x as i64;
        let y = position.y as i64;
        let w = size.width as i64;
        let h = size.height as i64;
        let ox = origin.x as i64;
        let oy = origin.y as i64;
        ox >= x && ox < x + w && oy >= y && oy < y + h
    }) {
        return Some(found);
    }
    // 窗口可能被停放在屏幕外（拔显示器等），退化为交叠面积最大者。
    monitors
        .iter()
        .max_by_key(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            let x = position.x as i64;
            let y = position.y as i64;
            let w = size.width as i64;
            let h = size.height as i64;
            let overlap_w = (x + w).min(origin.x as i64 + 1) - x.max(origin.x as i64);
            let overlap_h = (y + h).min(origin.y as i64 + 1) - y.max(origin.y as i64);
            (overlap_w.max(0)) * (overlap_h.max(0))
        })
        .or_else(|| monitors.first())
}

/// Linux 专用：向 Hyprland 问保留区。
#[cfg(target_os = "linux")]
mod hyprland {
    use serde::Deserialize;
    use std::path::PathBuf;
    use std::time::Duration;

    /// 保留区（物理像素）。
    pub struct Reserved {
        pub left: i32,
        pub right: i32,
        pub top: i32,
        pub bottom: i32,
    }

    #[derive(Deserialize)]
    struct HyprMonitor {
        #[serde(default)]
        x: i32,
        #[serde(default)]
        y: i32,
        #[serde(default)]
        width: i32,
        #[serde(default)]
        height: i32,
        #[serde(default = "default_scale")]
        scale: f64,
        #[serde(default)]
        reserved: Vec<i32>,
        #[serde(default)]
        transform: i32,
    }

    fn default_scale() -> f64 {
        1.0
    }

    /// IPC socket 路径：`$XDG_RUNTIME_DIR/hypr/<instance>/.socket.sock`。
    ///
    /// `HYPRLAND_INSTANCE_SIGNATURE` 在 GUI 会话里一定有；SSH 起的进程
    /// 没有，就退化成「hypr/ 下的第一个带 socket 的实例目录」。
    fn socket_path() -> Option<PathBuf> {
        let runtime = runtime_dir()?;
        let base = runtime.join("hypr");
        if let Some(signature) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
            let direct = base.join(signature).join(".socket.sock");
            if direct.exists() {
                return Some(direct);
            }
        }
        let mut found: Vec<PathBuf> = std::fs::read_dir(&base)
            .ok()?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join(".socket.sock"))
            .filter(|path| path.exists())
            .collect();
        found.sort();
        found.into_iter().next()
    }

    /// 运行期目录：`XDG_RUNTIME_DIR` 优先，否则从 `/proc/self/status` 的
    /// Uid 推 `/run/user/<uid>`（SSH 会话里前者常常缺失）。
    fn runtime_dir() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            if !dir.is_empty() {
                return Some(PathBuf::from(dir));
            }
        }
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let uid = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))?
            .split_whitespace()
            .next()?
            .to_string();
        Some(PathBuf::from(format!("/run/user/{uid}")))
    }

    /// 问 Hyprland 要显示器列表（`j/monitors`）。
    async fn query_monitors() -> Option<Vec<HyprMonitor>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;

        let path = socket_path()?;
        // 超时包两层：外层是 tokio 的 Elapsed，内层是 io::Error。
        let connect = tokio::time::timeout(Duration::from_millis(800), UnixStream::connect(&path))
            .await
            .ok()?;
        let mut stream = connect.ok()?;
        stream.write_all(b"j/monitors").await.ok()?;
        let mut buffer = Vec::new();
        let read =
            tokio::time::timeout(Duration::from_millis(800), stream.read_to_end(&mut buffer))
                .await
                .ok()?;
        read.ok()?;
        let value: serde_json::Value = serde_json::from_slice(&buffer).ok()?;
        serde_json::from_value(value).ok()
    }

    /// 取与 Tauri 显示器对应的那块 Hyprland 屏幕的保留区（物理像素）。
    pub async fn reserved_for(target: &tauri::window::Monitor) -> Option<Reserved> {
        let hypr = query_monitors().await?;
        let target_x = target.position().x as i64;
        let target_y = target.position().y as i64;
        let target_w = target.size().width as i64;
        let target_h = target.size().height as i64;

        let mut best: Option<(i64, Reserved)> = None;
        for monitor in hypr {
            // 旋转过的屏幕坐标系不同，先不管（omarchy 上是 0）。
            if monitor.transform != 0 {
                continue;
            }
            let scale = if monitor.scale > 0.0 {
                monitor.scale
            } else {
                1.0
            };
            // Hyprland 的坐标是逻辑像素，乘 scale 才是物理像素。
            let x = (monitor.x as f64 * scale).round() as i64;
            let y = (monitor.y as f64 * scale).round() as i64;
            let w = (monitor.width as f64 * scale).round() as i64;
            let h = (monitor.height as f64 * scale).round() as i64;
            let overlap_w = (x + w).min(target_x + target_w) - x.max(target_x);
            let overlap_h = (y + h).min(target_y + target_h) - y.max(target_y);
            if overlap_w <= 0 || overlap_h <= 0 {
                continue;
            }
            let area = overlap_w * overlap_h;
            if best
                .as_ref()
                .is_some_and(|(best_area, _)| area <= *best_area)
            {
                continue;
            }
            let reserved = monitor.reserved;
            let pick = |index: usize| -> i32 { reserved.get(index).copied().unwrap_or(0) };
            best = Some((
                area,
                Reserved {
                    left: (pick(0) as f64 * scale).round() as i32,
                    right: (pick(1) as f64 * scale).round() as i32,
                    top: (pick(2) as f64 * scale).round() as i32,
                    bottom: (pick(3) as f64 * scale).round() as i32,
                },
            ));
        }
        best.map(|(_, reserved)| reserved)
    }
}

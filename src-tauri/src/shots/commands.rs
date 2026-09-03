//! 截屏域 IPC 命令与全局热键管理。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::error::{Error, Result};
use crate::logging;

/// 冻结帧目录：会话内固定，热键触发时清空重写。
const SHOTS_DIR: &str = "qx-shots";

/// 覆盖窗 label 前缀（每屏一窗，销毁事件据此清理）。
pub const OVERLAY_LABEL_PREFIX: &str = "shot-overlay-";

/// 一块显示器的冻结帧描述（物理像素 + 虚拟屏幕坐标）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FrozenMonitor {
    /// 覆盖窗要用的 monitor 索引。
    pub index: usize,
    /// 虚拟屏幕坐标（物理像素，多屏拼接原点可能为负）。
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// DPI 缩放（物理→逻辑换算用；建窗口的尺寸参数是逻辑单位）。
    pub scale: f64,
    /// 底图文件（asset protocol 引用）。
    pub image: String,
}

#[derive(Default)]
pub struct ShotsState {
    /// 当前注册中的热键（换绑时先注销旧的）。
    registered: Mutex<Option<String>>,
    /// 覆盖窗会话进行中（防重复触发热键）。
    overlay_active: AtomicBool,
    /// 会话代号：每次开会话递增，兜底定时器据此作废旧会话。
    session: AtomicU64,
    /// 待首帧就绪上报的覆盖窗 label（全部就绪才亮窗，消除黑屏空窗期）。
    pending_ready: Mutex<HashSet<String>>,
    /// 亮窗时要聚焦的覆盖窗（光标所在屏）。
    focus_label: Mutex<Option<String>>,
}

/// 捕获所有显示器到冻结帧目录。返回每屏一张 PNG 的清单。
#[tauri::command]
pub fn shots_capture(app: AppHandle) -> Result<Vec<FrozenMonitor>> {
    capture_all(&app)
}

fn capture_all(app: &AppHandle) -> Result<Vec<FrozenMonitor>> {
    let dir = shots_dir(app);
    std::fs::create_dir_all(&dir)
        .map_err(|cause| Error::Screenshot(format!("建冻结帧目录失败：{cause}")))?;
    // 上一次的冻结帧不再需要。
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }

    let monitors = xcap::Monitor::all()
        .map_err(|cause| Error::Screenshot(format!("枚举显示器失败：{cause}")))?;
    let mut frozen = Vec::new();
    for (index, monitor) in monitors.into_iter().enumerate() {
        let geometry = monitor_geometry(&monitor)?;
        let image = monitor
            .capture_image()
            .map_err(|cause| Error::Screenshot(format!("捕获显示器 {index} 失败：{cause}")))?;
        let path = dir.join(format!("mon-{index}.png"));
        image
            .save(&path)
            .map_err(|cause| Error::Screenshot(format!("写冻结帧失败：{cause}")))?;
        frozen.push(FrozenMonitor {
            index,
            x: geometry.0,
            y: geometry.1,
            width: geometry.2,
            height: geometry.3,
            scale: monitor_scale(&monitor),
            image: path.to_string_lossy().into_owned(),
        });
    }
    Ok(frozen)
}

/// xcap 0.9 的几何访问都返回 Result；统一解包成 (x, y, w, h)。
fn monitor_geometry(monitor: &xcap::Monitor) -> Result<(i32, i32, i32, i32)> {
    let x = monitor
        .x()
        .map_err(|cause| Error::Screenshot(format!("读取显示器坐标失败：{cause}")))?;
    let y = monitor
        .y()
        .map_err(|cause| Error::Screenshot(format!("读取显示器坐标失败：{cause}")))?;
    let width = monitor
        .width()
        .map_err(|cause| Error::Screenshot(format!("读取显示器尺寸失败：{cause}")))?
        as i32;
    let height = monitor
        .height()
        .map_err(|cause| Error::Screenshot(format!("读取显示器尺寸失败：{cause}")))?
        as i32;
    Ok((x, y, width, height))
}

/// DPI 缩放；读取失败按 1.0 兜底（坐标只偏不大）。
fn monitor_scale(monitor: &xcap::Monitor) -> f64 {
    monitor.scale_factor().unwrap_or(1.0) as f64
}

/// 注册（或换绑）截屏热键。accel 为 Tauri 快捷键语法（如 "Alt+A"）。
/// 触发逻辑在前端覆盖窗流程之外：这里直接发事件，前端拉起覆盖窗。
#[tauri::command]
pub fn shots_set_hotkey(app: AppHandle, accel: String) -> Result<()> {
    set_hotkey_impl(&app, &accel)
}

/// 启动恢复与命令共用的注册路径。
pub fn set_hotkey_impl(app: &AppHandle, accel: &str) -> Result<()> {
    let shortcuts = app.global_shortcut();
    let state = app.state::<ShotsState>();
    let mut registered = state.registered.lock().unwrap();
    if let Some(previous) = registered.take() {
        // 旧键可能已被系统侧 unregister（如插件重载），忽略失败。
        let _ = shortcuts.unregister(previous.as_str());
    }
    shortcuts
        .register(accel)
        .map_err(|cause| Error::Screenshot(format!("注册快捷键 {accel} 失败：{cause}")))?;
    *registered = Some(accel.to_owned());
    Ok(())
}

/// 注销截屏热键（设置清空时）。
#[tauri::command]
pub fn shots_clear_hotkey(app: AppHandle) -> Result<()> {
    let shortcuts = app.global_shortcut();
    let state = app.state::<ShotsState>();
    let mut registered = state.registered.lock().unwrap();
    if let Some(previous) = registered.take() {
        shortcuts
            .unregister(previous.as_str())
            .map_err(|cause| Error::Screenshot(format!("注销快捷键失败：{cause}")))?;
    }
    Ok(())
}

/// 产出到剪贴板：PNG（base64）→ 解码为 RGBA → 系统剪贴板。
#[tauri::command]
pub async fn shots_copy_clipboard(app: AppHandle, png_base64: String) -> Result<()> {
    let bytes = decode_base64(&png_base64)?;
    let rgba = tauri::async_runtime::spawn_blocking(move || {
        image::load_from_memory(&bytes)
            .map_err(|cause| Error::Screenshot(format!("解码 PNG 失败：{cause}")))
            .map(|loaded| loaded.to_rgba8())
    })
    .await
    .map_err(|cause| Error::Screenshot(format!("解码任务失败：{cause}")))??;
    let (width, height) = (rgba.width(), rgba.height());
    app.clipboard()
        .write_image(&tauri::image::Image::new_owned(
            rgba.into_raw(),
            width,
            height,
        ))
        .map_err(|cause| Error::Screenshot(format!("写剪贴板失败：{cause}")))
}

/// 产出到文件：保存到截图目录（默认「图片\千寻截屏」），返回完整路径。
#[tauri::command]
pub fn shots_save(png_base64: String) -> Result<String> {
    let bytes = decode_base64(&png_base64)?;
    let dir = default_shots_library();
    std::fs::create_dir_all(&dir)
        .map_err(|cause| Error::Screenshot(format!("建截图目录失败：{cause}")))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("qx-{stamp}.png"));
    std::fs::write(&path, bytes)
        .map_err(|cause| Error::Screenshot(format!("写截图失败：{cause}")))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 产出到贴图：写临时 PNG，前端 Pin 窗用 asset protocol 加载。
#[tauri::command]
pub fn shots_pin(app: AppHandle, png_base64: String) -> Result<String> {
    let bytes = decode_base64(&png_base64)?;
    let dir = shots_dir(&app);
    std::fs::create_dir_all(&dir)
        .map_err(|cause| Error::Screenshot(format!("建冻结帧目录失败：{cause}")))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let path = dir.join(format!("pin-{stamp}.png"));
    std::fs::write(&path, bytes)
        .map_err(|cause| Error::Screenshot(format!("写贴图失败：{cause}")))?;
    Ok(path.to_string_lossy().into_owned())
}

fn shots_dir(_app: &AppHandle) -> PathBuf {
    std::env::temp_dir().join(SHOTS_DIR)
}

/// 默认截图目录：图片库\千寻截屏（不存在图片库时落到图片目录）。
fn default_shots_library() -> PathBuf {
    let pictures = dirs::picture_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
    pictures.join("千寻截屏")
}

fn decode_base64(text: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|cause| Error::Screenshot(format!("解码 PNG 失败：{cause}")))
}

/// 热键触发的完整流程：capture → 每屏一个覆盖窗。
/// 由 lib.rs 的快捷键 handler 调用（也供托盘菜单/前端按钮复用）。
pub fn start_session(app: &AppHandle) {
    let state = app.state::<ShotsState>();
    if state.overlay_active.swap(true, Ordering::AcqRel) {
        // 上一轮覆盖窗还开着：视为误触，忽略。
        return;
    }
    if let Err(failure) = open_overlays_impl(app) {
        logging::log("warn", &format!("截屏会话启动失败：{failure}"));
        state.overlay_active.store(false, Ordering::Release);
    }
}

fn open_overlays_impl(app: &AppHandle) -> Result<()> {
    let frozen = capture_all(app)?;
    if frozen.is_empty() {
        return Err(Error::Screenshot("没有可用的显示器".into()));
    }
    let state = app.state::<ShotsState>();
    // 亮窗门控：覆盖窗先隐藏，等各屏页面画好首帧（overlay_ready 上报）再一起亮出，
    // 消除热键后 WebView 未绘制 / 底图未解码的一瞬黑屏。
    let generation = state.session.fetch_add(1, Ordering::AcqRel) + 1;
    {
        let mut pending = state.pending_ready.lock().unwrap();
        pending.clear();
        pending.extend(
            frozen
                .iter()
                .map(|monitor| format!("{OVERLAY_LABEL_PREFIX}{}", monitor.index)),
        );
    }
    *state.focus_label.lock().unwrap() = Some(format!(
        "{OVERLAY_LABEL_PREFIX}{}",
        focus_monitor_index(&frozen, cursor_position(app))
    ));

    for monitor in &frozen {
        let label = format!("{OVERLAY_LABEL_PREFIX}{}", monitor.index);
        // 旧窗残留（异常路径）先清。
        if let Some(existing) = app.get_webview_window(&label) {
            let _ = existing.close();
        }
        let url = format!(
            "index.html#/overlay?monitor={}&path={}",
            monitor.index,
            urlencode(&monitor.image)
        );
        // builder 的 position/inner_size 是逻辑单位；物理坐标除以 DPI 缩放。
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
            .title("千寻截屏")
            .decorations(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .shadow(false)
            .visible(false) // overlay_ready 上报首帧后由 reveal_overlays 亮出并聚焦
            .position(
                monitor.x as f64 / monitor.scale,
                monitor.y as f64 / monitor.scale,
            )
            .inner_size(
                monitor.width as f64 / monitor.scale,
                monitor.height as f64 / monitor.scale,
            )
            .build()
            .map_err(|cause| Error::Screenshot(format!("建覆盖窗失败：{cause}")))?;
    }

    // 兜底：页面异常没能上报就绪时强制亮窗（用户至少还能框选或 Esc 退出）。
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if handle.state::<ShotsState>().session.load(Ordering::Acquire) == generation {
            reveal_overlays(&handle);
        }
    });
    Ok(())
}

/// 当前光标的虚拟屏幕坐标（物理像素）；拿不到时 None（聚焦回退第一屏）。
fn cursor_position(app: &AppHandle) -> Option<(f64, f64)> {
    let position = app.cursor_position().ok()?;
    Some((position.x, position.y))
}

/// 光标所在的显示器下标（亮窗后聚焦该屏，"在哪屏触发就在哪屏选"）；
/// 光标不在任何屏上时回退第一个。
fn focus_monitor_index(frozen: &[FrozenMonitor], cursor: Option<(f64, f64)>) -> usize {
    let Some((cursor_x, cursor_y)) = cursor else {
        return 0;
    };
    frozen
        .iter()
        .position(|monitor| {
            let left = f64::from(monitor.x);
            let top = f64::from(monitor.y);
            left <= cursor_x
                && cursor_x < left + f64::from(monitor.width)
                && top <= cursor_y
                && cursor_y < top + f64::from(monitor.height)
        })
        .unwrap_or(0)
}

/// 亮出全部覆盖窗并聚焦目标屏（全部就绪 / 兜底超时共用；幂等）。
fn reveal_overlays(app: &AppHandle) {
    let focus = app
        .state::<ShotsState>()
        .focus_label
        .lock()
        .unwrap()
        .clone();
    let overlays = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with(OVERLAY_LABEL_PREFIX))
        .collect::<Vec<_>>();
    for (_, window) in &overlays {
        let _ = window.show();
    }
    let target = focus
        .filter(|label| overlays.iter().any(|(existing, _)| existing == label))
        .or_else(|| overlays.first().map(|(label, _)| label.clone()));
    if let Some(label) = target {
        if let Some((_, window)) = overlays.iter().find(|(existing, _)| *existing == label) {
            let _ = window.set_focus();
        }
    }
    app.state::<ShotsState>()
        .pending_ready
        .lock()
        .unwrap()
        .clear();
}

/// 覆盖窗全部关闭时解除会话锁（lib.rs 的窗口销毁事件调用）。
pub fn overlay_closed(app: &AppHandle) {
    let remaining = app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with(OVERLAY_LABEL_PREFIX))
        .count();
    if remaining == 0 {
        app.state::<ShotsState>()
            .overlay_active
            .store(false, Ordering::Release);
    }
}

/// 覆盖窗前端首帧就绪上报：全部屏就绪后统一亮窗（消除热键后的黑屏空窗期）。
#[tauri::command]
pub fn shots_overlay_ready(app: AppHandle, label: String) -> Result<()> {
    let state = app.state::<ShotsState>();
    let remaining = {
        let mut pending = state.pending_ready.lock().unwrap();
        pending.remove(&label);
        pending.len()
    };
    if remaining == 0 {
        reveal_overlays(&app);
    }
    Ok(())
}

/// 手动关闭全部覆盖窗（前端 Esc 取消走窗口 close，殊途同归到销毁事件）。
#[tauri::command]
pub fn shots_close_overlays(app: AppHandle) -> Result<()> {
    for (label, window) in app.webview_windows() {
        if label.starts_with(OVERLAY_LABEL_PREFIX) {
            let _ = window.close();
        }
    }
    Ok(())
}

/// 打开贴图 Pin 窗：path 为 shots_pin 写出的 PNG 路径。
#[tauri::command]
pub fn shots_open_pin(app: AppHandle, path: String) -> Result<()> {
    let stamp = chrono::Local::now().format("%H%M%S%3f");
    let label = format!("pin-{stamp}");
    let url = format!("index.html#/pin?path={}", urlencode(&path));
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .title("千寻贴图")
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(false)
        .inner_size(420.0, 320.0)
        .build()
        .map_err(|cause| Error::Screenshot(format!("建贴图窗失败：{cause}")))?;
    Ok(())
}

/// 简易百分号编码：路径进 URL query 用。
fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 百分号编码覆盖保留字与中文() {
        assert_eq!(urlencode("aZ09-_.~"), "aZ09-_.~");
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
        assert!(urlencode("千寻").starts_with("%E5%8D%83"));
    }

    fn monitor(index: usize, x: i32, y: i32, width: i32, height: i32) -> FrozenMonitor {
        FrozenMonitor {
            index,
            x,
            y,
            width,
            height,
            scale: 1.0,
            image: String::new(),
        }
    }

    #[test]
    fn 聚焦屏取光标所在显示器并兜底第一屏() {
        let frozen = vec![
            monitor(0, 0, 0, 1920, 1080),
            monitor(1, 1920, 0, 2560, 1440),
        ];
        // 光标在第二屏 → 聚焦第二屏。
        assert_eq!(focus_monitor_index(&frozen, Some((3000.0, 100.0))), 1);
        // 光标在第一屏 → 第一屏。
        assert_eq!(focus_monitor_index(&frozen, Some((10.0, 900.0))), 0);
        // 光标悬在屏外（多屏间隙）/拿不到 → 回退第一屏。
        assert_eq!(focus_monitor_index(&frozen, Some((1900.0, 2000.0))), 0);
        assert_eq!(focus_monitor_index(&frozen, None), 0);
        // 负坐标屏（第二屏在主屏左侧）也能命中。
        let nested = vec![
            monitor(0, -2560, 0, 2560, 1440),
            monitor(1, 0, 0, 1920, 1080),
        ];
        assert_eq!(focus_monitor_index(&nested, Some((-100.0, 50.0))), 0);
    }

    /// 真实显示器捕获链路（本机有屏才能跑）：枚举 → 截图 → PNG 落盘非空。
    #[test]
    #[ignore = "真实显示器截屏，验收时手跑"]
    fn 真实捕获显示器() {
        let monitors = xcap::Monitor::all().expect("枚举显示器");
        assert!(!monitors.is_empty());
        for monitor in &monitors {
            let geometry = monitor_geometry(monitor).expect("几何");
            assert!(geometry.2 > 0 && geometry.3 > 0);
            let image = monitor.capture_image().expect("捕获");
            let out = std::env::temp_dir().join(format!("qx-cap-test-{}.png", geometry.0));
            image.save(&out).expect("保存");
            let size = std::fs::metadata(&out).expect("元数据").len();
            assert!(size > 10_000, "截图过小：{size}");
            let _ = std::fs::remove_file(out);
        }
    }
}

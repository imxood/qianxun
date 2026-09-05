//! 窗口域：前置窗口查找、显示/聚焦、几何记忆（恢复与持久化），
//! 以及「独立窗口」的生命周期（终端 / DSH 页分离到独立 OS 窗口）。
//!
//! 几何记忆采用「关闭/退出时一次性快照」而非每次移动都写盘：
//! 设置文件不值得为窗口拖动承受成倍的写入（架构 §4.1 的原子写代价）。

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::paths;
use crate::settings::{self, Geometry};
use crate::{logging, AppState};

/// 独立窗口 label 前缀：`standalone-{view}-{n}`。Destroyed 清理与前端
/// 窗口身份判断都以该前缀为准。
pub const STANDALONE_PREFIX: &str = "standalone-";

/// 独立窗口计数器（label 唯一性 + 级联偏移）。
static NEXT_STANDALONE: AtomicU64 = AtomicU64::new(1);

/// 独立窗口支持的两类视图。
fn standalone_view_meta(view: &str) -> Option<(&'static str, f64, f64)> {
    match view {
        "terminal" => Some(("终端 · 千寻", 900.0, 640.0)),
        "dsh" => Some(("DSH · 千寻", 1040.0, 740.0)),
        _ => None,
    }
}

/// 从独立窗口 label 解出 view（非独立窗口返回 None）。
pub fn standalone_view_of_label(label: &str) -> Option<&str> {
    let rest = label.strip_prefix(STANDALONE_PREFIX)?;
    let view = rest.rsplit_once('-')?.0;
    Some(view)
}

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

    // 设了「关到托盘」→ 隐藏；没设但独立窗口还开着 → 也隐藏：
    // 真退出会把独立窗口连同它们的终端会话一起带走，用户预期是
    // 「关的是主窗，不是整个应用」。独立窗口全关后恢复设置语义。
    let any_standalone = app
        .webview_windows()
        .keys()
        .any(|label| label.starts_with(STANDALONE_PREFIX));

    if hide || any_standalone {
        api.prevent_close();
        let _ = window.hide();
        if any_standalone && !hide {
            logging::log("info", "主窗隐藏到托盘：仍有独立窗口在运行");
        } else {
            logging::log("info", "窗口隐藏到托盘（托盘菜单可退出）");
        }
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

/// 开/关开发者工具（F12 由前端捕获后调用）。
/// 浏览器加速键已整体关闭（Ctrl+Shift+C 误触的根因），devtools 的
/// 唯一入口收敛到这条命令。
#[tauri::command]
pub fn app_toggle_devtools(app: AppHandle) -> crate::error::Result<()> {
    let Some(window) = front(&app) else {
        return Ok(());
    };
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
        let _ = window.set_focus();
    }
    Ok(())
}

// ---- 独立窗口（终端 / DSH 分离窗）----

/// `window://closed` 事件负载：主窗前端据此恢复侧栏项。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StandaloneClosedEvent<'a> {
    label: &'a str,
    view: &'a str,
}

/// WebView2 运行时偏好，每个 webview 创建后应用一次：
/// 1. 关浏览器加速键（Ctrl+Shift+C/V 误触的根因）；
/// 2. 首选配色显式设为 OS「应用模式」明暗——WebView2 的
///    prefers-color-scheme 默认恒报 light，「跟随系统」主题会失灵
///    （深夜白屏刺眼）。配色值从注册表直读（AppsUseLightTheme），
///    不用 window.theme()：隐藏窗口阶段它的返回不可靠。OS 深浅色
///    切换时 lib.rs 的 ThemeChanged 钩子会再次调用本函数，实时跟随。
#[cfg(windows)]
pub fn apply_webview_preferences(window: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Settings3, COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
    };
    use windows_core::Interface;

    let os_app_dark = os_app_mode_is_dark();
    let scheme = if os_app_dark {
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK
    } else {
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT
    };
    if let Err(error) = window.with_webview(move |webview| unsafe {
        let controller = webview.controller();
        let Ok(core) = controller.CoreWebView2() else {
            return;
        };
        if let Ok(settings) = core.Settings() {
            if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                let _ = settings3.SetAreBrowserAcceleratorKeysEnabled(false);
            }
        }
        // Profile() 在 ICoreWebView2_13（runtime 1.0.1108+，Evergreen 均已覆盖）。
        if let Ok(profile13) =
            core.cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_13>()
        {
            if let Ok(profile) = profile13.Profile() {
                let _ = profile.SetPreferredColorScheme(scheme);
            }
        }
    }) {
        logging::log("warn", &format!("应用 WebView2 偏好失败：{error}"));
        return;
    }
    logging::log(
        "info",
        &format!(
            "WebView2 配色方案已应用：{}",
            if os_app_dark { "dark" } else { "light" }
        ),
    );
}

/// OS「应用模式」是否为暗色（注册表 AppsUseLightTheme=0）。
/// 读取失败按暗色处理：深夜白屏刺眼是实打实的伤害，浅色用户误得
/// 暗色只是观感差异（且可在设置里显式覆盖）。
#[cfg(windows)]
fn os_app_mode_is_dark() -> bool {
    use std::os::windows::process::CommandExt;
    let Ok(output) = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
    else {
        return true;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    !text.contains("0x1")
}

/// 分离：创建独立窗口承载某页（终端 / DSH）。返回新窗口 label，
/// 前端随后把会话/状态转移给它。
///
/// 必须是 async 命令：sync 命令在主线程上执行，同步 build 新窗口会与
/// WebView2 的异步初始化互相等消息泵而死锁（官方文档明示的模式）。
/// async 后跑在 runtime 线程，build 内部自行派发到主线程并等待。
#[tauri::command]
pub async fn window_spawn_view(app: AppHandle, view: String) -> crate::error::Result<String> {
    let (title, width, height) = standalone_view_meta(&view)
        .ok_or_else(|| crate::error::Error::Window(format!("不支持的独立窗口视图：{view}")))?;
    let n = NEXT_STANDALONE.fetch_add(1, Ordering::AcqRel);
    let label = format!("{STANDALONE_PREFIX}{view}-{n}");
    let url = format!("index.html#/standalone/{view}");

    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .title(title)
        .decorations(false)
        .visible(false) // 前端挂载完成后自行亮窗（同主窗策略，避免白闪）
        .inner_size(width, height)
        .minimizable(true)
        .maximizable(true)
        .resizable(true);

    // 级联定位：以主窗为锚，每次错开 32 逻辑像素（8 个一循环）。
    if let Some(main) = front(&app) {
        if let (Ok(position), Ok(scale)) = (main.outer_position(), main.scale_factor()) {
            let offset = ((n - 1) % 8) as f64 * 32.0;
            builder = builder.position(
                position.x as f64 / scale + offset,
                position.y as f64 / scale + offset,
            );
        }
    }

    let spawned = builder
        .build()
        .map_err(|cause| crate::error::Error::Window(format!("创建独立窗口失败：{cause}")))?;
    #[cfg(windows)]
    apply_webview_preferences(&spawned);
    Ok(label)
}

/// 前置并聚焦主窗（独立窗口「回到主窗口」按钮用）。
#[tauri::command]
pub fn window_reveal_main(app: AppHandle) {
    if let Some(main) = front(&app) {
        reveal(&main);
    }
}

/// 强制关闭调用方窗口（绕过 CloseRequested 拦截）。只对独立窗口放行，
/// main 的关闭语义（托盘/几何）不容前端绕过。
#[tauri::command]
pub fn window_force_close(window: tauri::WebviewWindow) -> crate::error::Result<()> {
    if !window.label().starts_with(STANDALONE_PREFIX) {
        return Err(crate::error::Error::Window("只允许关闭独立窗口".to_owned()));
    }
    window
        .destroy()
        .map_err(|cause| crate::error::Error::Window(format!("关闭窗口失败：{cause}")))
}

/// OS「应用模式」是否暗色（前端主题 seed；ThemeChanged 事件实时推送）。
#[tauri::command]
pub fn system_theme() -> bool {
    #[cfg(windows)]
    {
        os_app_mode_is_dark()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// OS 深浅色切换（lib.rs 的 ThemeChanged 钩子）：
/// 1. 重设全部 webview 的首选配色（DSH iframe 等网页内容跟随）；
/// 2. 广播 `system://theme` 让前端主题 store 立即切换（「跟随系统」）。
pub fn on_theme_changed(app: &AppHandle) {
    #[cfg(windows)]
    {
        for (_, webview) in app.webview_windows() {
            apply_webview_preferences(&webview);
        }
        let _ = app.emit("system://theme", os_app_mode_is_dark());
    }
}

/// 独立窗口关闭请求：拦截后转交给该窗口的前端做确认
/// （有活动终端时弹「结束进程？」对话框），确认后走 window_force_close。
pub fn on_standalone_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    api.prevent_close();
    let _ = app_emit_to(window, "window://close-requested", ());
}

/// 向指定窗口转发事件的小包装（Window → AppHandle::emit_to）。
fn app_emit_to(
    window: &tauri::Window,
    event: &str,
    payload: impl Serialize + Clone,
) -> tauri::Result<()> {
    window.app_handle().emit_to(window.label(), event, payload)
}

/// 独立窗口已销毁：终结它名下的终端会话（固定记录按「进程退出」语义
/// 保留回放），并广播 closed 事件让主窗恢复侧栏项。
pub fn on_standalone_destroyed(app: &AppHandle, label: &str) {
    let killed = crate::terminal::commands::kill_window_sessions(app, label);
    if killed > 0 {
        logging::log(
            "info",
            &format!("独立窗口 {label} 关闭：结束 {killed} 个终端会话"),
        );
    }
    if let Some(view) = standalone_view_of_label(label) {
        let _ = app.emit("window://closed", StandaloneClosedEvent { label, view });
    }
}

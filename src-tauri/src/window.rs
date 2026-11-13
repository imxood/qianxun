//! 窗口域：前置窗口查找、显示/聚焦、几何记忆（恢复与持久化），
//! 以及「独立窗口」的生命周期（DSH 页分离到独立 OS 窗口），
//! 外加内置浏览器窗（DSH 页外链分流：单例复用 + 多开级联）。
//!
//! 几何记忆采用「关闭/退出时一次性快照」而非每次移动都写盘：
//! 设置文件不值得为窗口拖动承受成倍的写入（架构 §4.1 的原子写代价）。
//! 快照与还原两头都做「屏幕相交」校验：Windows 最小化会把窗口停放到
//! (-32000,-32000)，拔显示器等拓扑变化也会让记忆坐标失效——垃圾值
//! 不许入账，历史坏数据在下次启动/重建时就地自修复（居中兜底）。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::paths;
use crate::settings::{self, Geometry};
use crate::{logging, AppState};

/// 工作区（work area）：最大化时窗口应当占住的矩形。
pub mod work_area;

/// 独立窗口 label 前缀：`standalone-{view}-{n}`。Destroyed 清理与前端
/// 窗口身份判断都以该前缀为准。
pub const STANDALONE_PREFIX: &str = "standalone-";

/// 独立窗口计数器（label 唯一性 + 级联偏移）。
static NEXT_STANDALONE: AtomicU64 = AtomicU64::new(1);

/// 主窗重建进行中：销毁旧窗到新窗建成之间会出现「零窗口」瞬间，
/// 事件循环对此的默认语义是发 code=None 的 ExitRequested（退出整个
/// 应用）。置位期间 lib.rs 的 run 回调对该事件 prevent_exit，进程才
/// 活得过重建间隙。`app.exit(0)`（托盘退出）走 Some(code) 不受影响；
/// `app.restart()` 的 RESTART_EXIT_CODE 连 prevent 都被框架忽略——
/// 两条正经退出路径都安然无恙。
static REBUILDING: AtomicBool = AtomicBool::new(false);

/// 重建标志的读取口（lib.rs 的 run 回调用）。
pub fn is_rebuilding() -> bool {
    REBUILDING.load(Ordering::Acquire)
}

/// 轻量模式（对齐 clash-verge-rev）：主窗 webview 已销毁，进程只保留
/// 核心——托盘、DSH supervisor、回环/局域网网关、截屏与侧车管理。
/// WebView2 的 renderer/browser 进程随主窗一并终结，是 UI 异常时的
/// 资源逃生门（实测 UI 存续时渲染进程群常驻两位数百分比 CPU）。
static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

/// 轻量模式读取口（lib.rs 的 run 回调与托盘菜单用）。
pub fn is_light_mode() -> bool {
    LIGHT_MODE.load(Ordering::Acquire)
}

/// 进入轻量模式：几何快照后销毁主窗（destroy 绕过「关到托盘」拦截，
/// 真正终结 WebView2）。Zero-window 的 ExitRequested 由 lib.rs 按本
/// 标志 prevent_exit，进程留在托盘。
pub fn enter_light_mode(app: &AppHandle) -> crate::error::Result<()> {
    LIGHT_MODE.store(true, Ordering::Release);
    persist_geometry(app);
    let destroyed = match front(app) {
        Some(front) => front
            .destroy()
            .map_err(|cause| crate::error::Error::Window(format!("销毁主窗失败：{cause}"))),
        None => Ok(()),
    };
    crate::tray::reflect_light_mode(true);
    logging::log("info", "已进入轻量模式（UI 已退出，核心服务继续运行）");
    destroyed
}

/// 退出轻量模式：重建主窗（rebuild_main 对「无旧窗」有直建分支，
/// 恰好覆盖本路径），前端就绪后自行亮窗。
pub async fn exit_light_mode(app: &AppHandle) -> crate::error::Result<()> {
    LIGHT_MODE.store(false, Ordering::Release);
    crate::tray::reflect_light_mode(false);
    let result = rebuild_main(app).await;
    if result.is_ok() {
        logging::log("info", "已退出轻量模式（UI 已重建）");
    }
    result
}

/// 独立窗口支持的视图。
fn standalone_view_meta(view: &str) -> Option<(&'static str, f64, f64)> {
    match view {
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
    // 启动/重建即自修复：记忆几何可能来自旧版本写下的停放点（-32000）
    // 或已失效的显示拓扑（拔显示器、改主屏）。按当前屏幕枚举校验，矩形
    // 与所有屏幕都不相交就放弃记忆位置、改居中——宁可挪位置，不可不亮窗。
    // 枚举失败按有效处理（fail-open），不能让校验本身弄丢窗口。
    let screens = screen_rects(&window);
    let on_screen = screens.is_empty()
        || intersects_any_screen(
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            &screens,
        );
    // 顺序有意为之：先尺寸后位置，最大化必须最后——位置在最大化之前
    // 落定，set_position 拉不回最大化状态，顺带把「取消最大化」时的
    // 还原锚点一并修成合法值。
    if geometry.width > 0 && geometry.height > 0 {
        let _ = window.set_size(PhysicalSize::new(geometry.width, geometry.height));
    }
    if on_screen {
        let _ = window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
    } else {
        logging::log(
            "warn",
            &format!(
                "记忆窗口位置 ({},{}) 不在任何屏幕内，已改为居中恢复",
                geometry.x, geometry.y
            ),
        );
        let _ = window.center();
    }
    if geometry.maximized {
        let _ = window.maximize();
    }
}

/// 当前几何快照。最大化时 outer_position 仍是还原锚点，直接记录即可。
/// 两类垃圾值在源头拦下、本次不入账（设置里保住上一次的正常几何）：
/// 1. 最小化中——Windows 把最小化窗口停放到 (-32000,-32000)；
/// 2. 矩形与所有屏幕都不相交——最小化后隐藏（关到托盘）的窗口
///    IsIconic 已为假、停放点会漏过第 1 条；以及拔显示器后退出等。
pub fn capture(app: &AppHandle) -> Option<Geometry> {
    let window = front(app)?;
    if window.is_minimized().unwrap_or(false) {
        return None;
    }
    let maximized = window.is_maximized().ok()?;
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let geometry = Geometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        maximized,
    };
    // 屏幕枚举失败按放行处理：校验不能反把快照弄丢。
    let screens = screen_rects(&window);
    if !screens.is_empty()
        && !intersects_any_screen(
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            &screens,
        )
    {
        logging::log(
            "warn",
            "窗口位置不在任何屏幕内，跳过几何快照（保留上次正常值）",
        );
        return None;
    }
    Some(geometry)
}

/// 当前全部屏幕的物理矩形 `(x, y, width, height)`，供相交校验使用。
fn screen_rects(window: &WebviewWindow) -> Vec<(i32, i32, u32, u32)> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|monitor| {
            (
                monitor.position().x,
                monitor.position().y,
                monitor.size().width,
                monitor.size().height,
            )
        })
        .collect()
}

/// 窗口矩形是否与任一屏幕相交（至少露出 1px，压屏边窗口不算失联）。
/// 坐标加法走 i64：i32 边界附近的野值不会溢出绕回而误判成相交。
fn intersects_any_screen(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    screens: &[(i32, i32, u32, u32)],
) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let (x, y) = (i64::from(x), i64::from(y));
    let (width, height) = (i64::from(width), i64::from(height));
    screens.iter().any(|&(sx, sy, sw, sh)| {
        let right = i64::from(sx) + i64::from(sw);
        let bottom = i64::from(sy) + i64::from(sh);
        x < right && x + width > i64::from(sx) && y < bottom && y + height > i64::from(sy)
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
    // 真退出会把独立窗口连同它们的 DSH 会话一起带走，用户预期是
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

// ---- 主窗重建（托盘「重建界面」）----

/// 重建主窗：销毁现有 WebView2 实例（controller + 渲染进程）后以同
/// label 从零建回。页面级 reload 只是往既有实例里投递一条导航指令——
/// fire-and-forget：WebView2 渲染/browser 进程挂死时调用返回 Ok 却
/// 无人执行，白屏依旧。只有销毁重建才是真正的「重启 webview」。
///
/// 必须 async 并跑在 runtime 线程：sync 上下文里同步 build 新窗会与
/// WebView2 的异步初始化互相等消息泵而死锁（同 window_spawn_view 注释）。
pub async fn rebuild_main(app: &AppHandle) -> crate::error::Result<()> {
    // destroy 走不到 CloseRequested（几何快照挂在那条路径上），先补一次。
    persist_geometry(app);
    // 几何在动手前读快照；persist 刚写过，读到的就是当前实况。
    let geometry = {
        let state = app.state::<AppState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.window.geometry.clone()
    };

    REBUILDING.store(true, Ordering::Release);

    // 先销毁旧窗：destroy 绕过 CloseRequested 拦截（「关到托盘」），真正
    // 终结 WebView2 controller + 渲染进程。窗口要在事件循环走完 Destroyed
    // 后 label 才释放，所以建窗交给下面的重试循环。
    let destroyed = match front(app) {
        Some(old) => old
            .destroy()
            .map_err(|cause| crate::error::Error::Window(format!("销毁旧主窗失败：{cause}"))),
        // 没有旧窗（异常残局，如启动期就坏了）：直接建新的。
        None => Ok(()),
    };

    let result = match destroyed {
        Err(cause) => Err(cause),
        Ok(()) => {
            // window 与 webview 双注册同 label，二者的释放都挂在事件循环
            // 的 Destroyed 处理上，刚 destroy 完立刻建必撞 AlreadyExists
            // （两个变体都可能）——首次立即尝试，撞了稍候重试；事件循环
            // 一两拍内即腾出 label，5 秒上限只是兜底（超时意味着事件循环
            // 本身出了问题）。
            let mut attempted = false;
            let mut result = Err(crate::error::Error::Window(
                "重建主窗超时：旧 label 迟迟未释放".to_owned(),
            ));
            for _ in 0..50 {
                if attempted {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                attempted = true;
                match build_main(app, geometry.as_ref()) {
                    Ok(()) => {
                        result = Ok(());
                        break;
                    }
                    Err(
                        tauri::Error::WindowLabelAlreadyExists(_)
                        | tauri::Error::WebviewLabelAlreadyExists(_),
                    ) => continue,
                    Err(cause) => {
                        result = Err(crate::error::Error::Window(format!(
                            "重建主窗失败：{cause}"
                        )));
                        break;
                    }
                }
            }
            result
        }
    };
    REBUILDING.store(false, Ordering::Release);

    match result {
        Ok(()) => {
            logging::log("info", "主窗已重建（WebView2 实例焕新）");
            Ok(())
        }
        Err(cause) => {
            logging::log("error", &format!("主窗重建失败：{cause}"));
            Err(cause)
        }
    }
}

/// 按 tauri.conf.json 主窗参数建窗（重建路径专用；启动路径由配置文件
/// 建窗，参数与这里保持一致）。成功后应用 WebView2 偏好、还原几何，
/// 并自备兜底亮窗——setup 里的 20s 兜底只管启动，前端 boot 若再挂死，
/// 重建出的窗口至少不会「隐身」。
///
/// 窗口一律自绘无边框（decorations=false），标题栏由前端 TitleBar 组件
/// 负责。Linux(CEF) 下虽然 WM 也不给 GTK4 窗口画边框（实测无
/// _NET_FRAME_EXTENTS，GTK4 走客户端装饰），且拖拽/最小化要靠前端自己
/// 实现（见 src/lib/window/linuxWindow.ts），但保持「全部平台统一自绘」
/// 的一致性更省心。
fn build_main(app: &AppHandle, geometry: Option<&Geometry>) -> tauri::Result<()> {
    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("千寻")
        .inner_size(1100.0, 720.0)
        .min_inner_size(880.0, 580.0)
        .decorations(false)
        .visible(false); // 前端就绪后自行亮窗（同启动策略，避免白闪）
    if geometry.is_none() {
        builder = builder.center();
    }
    let _window = builder.build()?;
    #[cfg(windows)]
    apply_webview_preferences(&_window);
    if let Some(geometry) = geometry {
        restore(app, geometry);
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        if let Some(front) = front(&handle) {
            if !front.is_visible().unwrap_or(true) {
                let _ = front.show();
                let _ = front.set_focus();
            }
        }
    });
    Ok(())
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

/// 当前窗口所在显示器的可用工作区（物理像素）。
///
/// 用途：Linux/CEF 路线下「最大化」由前端自绘（见
/// `src/lib/window/linuxWindow.ts`），而 Chromium 的 `screen.avail*` 与
/// Tauri 的 `monitor.work_area()` 都只认整块屏幕，不知道窗口管理器的
/// 保留区（omarchy 左侧 28px 竖条）——铺满整屏会把它盖住。
/// Rust 侧直接向 WM 问，拿不到就退回整屏（即旧行为）。
#[tauri::command]
pub async fn window_work_area(app: AppHandle) -> Option<work_area::WorkArea> {
    let window = front(&app)?;
    work_area::for_window(&window).await
}

// ---- 独立窗口（DSH 分离窗）----

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
    // Tauri 3：with_webview 的 wry 专属形态移到 runtime 扩展 trait。
    use tauri_runtime_wry::WebviewWryExt;
    use windows_core::Interface;

    let os_app_dark = os_app_mode_is_dark();
    let scheme = if os_app_dark {
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK
    } else {
        COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT
    };
    if let Err(error) = window.with_wry_webview(move |webview| unsafe {
        let controller = webview.controller();
        let Ok(core) = controller.CoreWebView2() else {
            return;
        };
        if let Ok(settings) = core.Settings() {
            if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                let _ = settings3.SetAreBrowserAcceleratorKeysEnabled(true);
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

/// 分离：创建独立窗口承载 DSH 页。返回新窗口 label，
/// 前端随后把状态转移给它。
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

    let _spawned = builder
        .build()
        .map_err(|cause| crate::error::Error::Window(format!("创建独立窗口失败：{cause}")))?;
    #[cfg(windows)]
    apply_webview_preferences(&_spawned);
    Ok(label)
}

// ---- 内置浏览器窗（DSH 页外链分流）----

/// 内置浏览器单例窗 label：普通点击复用此窗换址，不在任何 capability 里。
pub const BROWSER_LABEL: &str = "browser";

/// 多开窗口计数（`browser-{n}`，中键/Shift+点击新开，级联错位）。
static NEXT_BROWSER: AtomicU64 = AtomicU64::new(2);

/// 内置浏览器窗：以系统 WebView2（Edge 内核）直载目标 URL——千寻即浏览器。
/// 由 DSH 页注入脚本（gateway 的 /qx-shell/links.js）经外壳转发的外链在此分流：
/// - 普通点击：复用单例窗 navigate 换址并前置（远程页没有外壳前端，事件线
///   不可用；navigate 走 WebView2 导航，比 eval 更正式）；
/// - 中键/Shift+点击（`new_window`）：新开窗口且不抢焦点，级联错位。
///
/// 安全：`browser*` label 不在任何 capability —— 远程页面零 IPC 权限，
/// Tauri 调用一律默认拒绝；入参仅放行 http/https。
/// 必须 async：sync 命令在主线程同步 build 窗口会与 WebView2 异步初始化
/// 互等消息泵而死锁（同 window_spawn_view 注释）。
#[tauri::command]
pub async fn window_open_external(
    app: AppHandle,
    url: String,
    new_window: bool,
) -> crate::error::Result<String> {
    let parsed = tauri::Url::parse(&url)
        .map_err(|cause| crate::error::Error::Window(format!("链接不合法（{url}）：{cause}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(crate::error::Error::Window(format!(
            "内置浏览器仅支持 http/https，拒绝：{url}"
        )));
    }

    // 单例复用：窗还活着就换址 + 前置，不弹新窗。
    if !new_window {
        if let Some(existing) = app.get_webview_window(BROWSER_LABEL) {
            existing.navigate(parsed.clone()).map_err(|cause| {
                crate::error::Error::Window(format!("内置浏览器换址失败：{cause}"))
            })?;
            let _ = existing.show();
            let _ = existing.unminimize();
            let _ = existing.set_focus();
            return Ok(BROWSER_LABEL.to_owned());
        }
    }

    let n = NEXT_BROWSER.fetch_add(1, Ordering::AcqRel);
    let label = if new_window {
        format!("{BROWSER_LABEL}-{n}")
    } else {
        BROWSER_LABEL.to_owned()
    };
    let title = format!("浏览 · {}", parsed.host_str().unwrap_or("页面"));

    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed))
        .title(title)
        .inner_size(1120.0, 760.0)
        .min_inner_size(420.0, 320.0)
        .focused(!new_window) // 后台窗（中键/Shift）不抢焦点
        .visible(false); // 首个页面开始加载即亮窗（on_page_load），避免白闪

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

    // Started 即亮窗：错误页也会走 load 事件，不会永远隐身；后台窗只亮不抢焦。
    let focus_on_show = !new_window;
    builder = builder.on_page_load(move |window, event| {
        if matches!(event.event(), tauri::webview::PageLoadEvent::Started) {
            let _ = window.show();
            if focus_on_show {
                let _ = window.set_focus();
            }
        }
    });

    let _spawned = builder
        .build()
        .map_err(|cause| crate::error::Error::Window(format!("创建内置浏览器窗失败：{cause}")))?;
    #[cfg(windows)]
    apply_webview_preferences(&_spawned);

    // 兜底亮窗：万一目标站连 load 事件都不发（极端网络黑洞），8 秒后强制
    // 亮出，宁可白屏也不让窗口「隐身」（同主窗 20s 兜底的思路）。
    let handle = app.clone();
    let fallback_label = label.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        if let Some(window) = handle.get_webview_window(&fallback_label) {
            if !window.is_visible().unwrap_or(true) {
                let _ = window.show();
            }
        }
    });
    Ok(label)
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
    // 非 Windows：没有 WebView2 偏好可设（CEF 路线不需要），此处保持静默。
    #[cfg(not(windows))]
    let _ = app;
}

/// 独立窗口已销毁：广播 closed 事件让主窗恢复侧栏项。
pub fn on_standalone_destroyed(app: &AppHandle, label: &str) {
    if let Some(view) = standalone_view_of_label(label) {
        let _ = app.emit("window://closed", StandaloneClosedEvent { label, view });
    }
}

#[cfg(test)]
mod tests {
    use super::intersects_any_screen;

    /// 单屏：1920×1080 @ (0,0)。
    const SINGLE: [(i32, i32, u32, u32); 1] = [(0, 0, 1920, 1080)];
    /// 双屏：主屏 @ (0,0) + 左侧副屏 @ (-1920,0)。
    const DUAL: [(i32, i32, u32, u32); 2] = [(0, 0, 1920, 1080), (-1920, 0, 1920, 1080)];

    #[test]
    fn windows最小化停放点_负三万二_判为失联() {
        assert!(!intersects_any_screen(-32000, -32000, 455, 99, &SINGLE));
    }

    #[test]
    fn 正常屏幕内坐标_判为可见() {
        assert!(intersects_any_screen(800, 800, 455, 99, &SINGLE));
    }

    #[test]
    fn 只露一像素也算可见_完全出界才算失联() {
        assert!(intersects_any_screen(-454, 100, 455, 99, &SINGLE));
        assert!(!intersects_any_screen(-455, 100, 455, 99, &SINGLE));
    }

    #[test]
    fn 拔掉副屏后_原副屏坐标判为失联() {
        assert!(intersects_any_screen(-1800, 100, 455, 99, &DUAL));
        assert!(!intersects_any_screen(-1800, 100, 455, 99, &SINGLE));
    }

    #[test]
    fn 零尺寸判为失联() {
        assert!(!intersects_any_screen(800, 800, 0, 99, &SINGLE));
        assert!(!intersects_any_screen(800, 800, 455, 0, &SINGLE));
    }

    #[test]
    fn 野值坐标_不溢出绕回误判() {
        assert!(!intersects_any_screen(i32::MAX, i32::MAX, 455, 99, &SINGLE));
        assert!(!intersects_any_screen(i32::MIN, i32::MIN, 455, 99, &SINGLE));
    }
}

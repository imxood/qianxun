# 06 · Tauri 窗口重建指南（真·重启 webview）

状态：实战提炼（2026-09-14，千寻首次落地）。实测环境：**tauri 2.11.5 / tauri-runtime-wry 2.11.4 / WebView2 Evergreen / Windows**。
适用场景：任何 Tauri 2 桌面项目需要「白屏自愈」「重置 WebView2」的能力。本仓库实现见
`src-tauri/src/window.rs`（`rebuild_main`）、`tray.rs`（托盘入口）、`lib.rs`（事件循环）。

## 1. 问题：reload 修不了的白屏

WebView2 有三层：**宿主进程**（Tauri 应用自己）、**browser 进程**（`msedgewebview2.exe`，
含 network service，页面 socket 挂在这里）、**渲染进程**（页面 JS/DOM）。

`WebviewWindow::reload()` 映射到 `ICoreWebView2::Reload()`——它只是向**既有 WebView2
实例**投递一条导航指令，且是 fire-and-forget：

- 渲染/browser 进程**挂死**时，调用**返回 Ok 却无人执行**——白屏依旧，日志无错，
  无人告警；
- 它永远不会给你新的 WebView2 实例，webview 层的坏状态（渲染进程崩溃残留、
  GPU 上下文损坏等）原样保留。

**唯一可靠的「重启 webview」= 销毁宿主窗口并原 label 重建**：新 controller、新
渲染进程、页面从零 boot。代价与页面 reload 相同（前端状态丢失），后端（Rust 侧
supervisor/网关/会话）零扰动。

## 2. 五个坑（每个都实测撞过）

| #   | 坑                                | 后果（不处理的话）                                                                                                        | 对策                                                                                          |
| --- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| 1   | 用 `close()` 销毁                 | `close()` 先发 `CloseRequested`，会被应用的「关到托盘」拦截改成 hide——窗口没死，重建必撞 label 冲突                       | 用 `destroy()`：绕过拦截，真正终结                                                            |
| 2   | `destroy()` 返回后立刻重建        | 撞 `WindowLabelAlreadyExists` **或** `WebviewLabelAlreadyExists`（window+webview **双注册**同 label，两个错误变体都可能） | 首次立即尝试，撞了两种变体都重试；间隔 100ms、上限 5s                                         |
| 3   | 销毁最后一个窗口 → 整个应用退出   | 事件循环在最后一个 `Destroyed` 时发 `ExitRequested{code:None}`，默认语义是退出进程——重建把自己搞死了                      | 事件循环回调里对 `code:None` 且「重建中」标志置位时 `api.prevent_exit()`                      |
| 4   | 在主线程 sync 上下文里 build 新窗 | 与 WebView2 异步初始化互相等消息泵，**死锁**                                                                              | `rebuild_main` 做成 async，经 `tauri::async_runtime::spawn` 跑在 runtime 线程                 |
| 5   | 以为建窗会继承 `tauri.conf.json`  | 运行时 builder 不读窗口配置：标题/尺寸/无边框/隐藏创建全得手动复刻；WebView2 COM 偏好、几何、兜底亮窗同理                 | 建窗参数与配置文件保持一致并注释「两处同步改」；建成后重应用偏好、还原几何、自备 20s 兜底亮窗 |

另有一条纪律：**几何快照要先做**。正常关闭路径的几何持久化挂在 `CloseRequested`
上，`destroy()` 走不到那里，动手前先补一次。

## 3. 时序

```
托盘点击
  └─ async_runtime::spawn(rebuild_main)
       ├─ persist_geometry（destroy 走不到 CloseRequested，先补快照）
       ├─ 读几何快照（state 锁 clone）
       ├─ REBUILDING = true
       ├─ old.destroy()            ← WebView2 实例终结（窗口可见地消失）
       │    └─ 事件循环（主线程）：Destroyed → label 注册表清理
       │                            → [零窗口瞬间] ExitRequested{None}
       │                            → 回调见 REBUILDING → prevent_exit ✓
       ├─ 重试 build（首次立即，撞 AlreadyExists 则 100ms 后再试）
       ├─ 应用 WebView2 偏好 + 还原几何 + 派发 20s 兜底亮窗
       ├─ REBUILDING = false
       └─ 前端重新 boot → 就绪后自行亮窗
```

## 4. 实现

### 4.1 `window.rs`：重建本体

```rust
use std::sync::atomic::{AtomicBool, Ordering};

/// 重建进行中标志：销毁旧窗到新窗建成之间会出现「零窗口」瞬间，事件
/// 循环对此的默认语义是发 code=None 的 ExitRequested（退出整个应用）。
/// 置位期间 run 回调对该事件 prevent_exit，进程才活得过重建间隙。
/// app.exit(0) 走 Some(code) 不受影响；app.restart() 的 RESTART_EXIT_CODE
/// 连 prevent 都被框架忽略——两条正经退出路径都安然无恙。
static REBUILDING: AtomicBool = AtomicBool::new(false);

pub fn is_rebuilding() -> bool {
    REBUILDING.load(Ordering::Acquire)
}

/// 重建主窗：销毁现有 WebView2 实例（controller + 渲染进程）后以同
/// label 从零建回。页面级 reload 只是往既有实例投递导航指令——
/// fire-and-forget：WebView2 进程挂死时返回 Ok 却无人执行，白屏依旧。
///
/// 必须 async 并跑在 runtime 线程：sync 上下文里同步 build 新窗会与
/// WebView2 的异步初始化互相等消息泵而死锁。
pub async fn rebuild_main(app: &AppHandle) -> Result<()> {
    // destroy 走不到 CloseRequested（几何快照挂在那条路径上），先补一次。
    persist_geometry(app);
    let geometry = {
        let state = app.state::<AppState>();
        let guard = state
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.window.geometry.clone()
    };

    REBUILDING.store(true, Ordering::Release);

    // 先销毁旧窗：destroy 绕过 CloseRequested 拦截（「关到托盘」）。
    // 窗口要在事件循环走完 Destroyed 后 label 才释放，建窗交给下面的重试。
    let destroyed = match front(app) {
        Some(old) => old
            .destroy()
            .map_err(|cause| Error::Window(format!("销毁旧主窗失败：{cause}"))),
        None => Ok(()), // 没有旧窗（异常残局）：直接建新的
    };

    let result = match destroyed {
        Err(cause) => Err(cause),
        Ok(()) => {
            // window 与 webview 双注册同 label，二者的释放都挂在事件循环
            // 的 Destroyed 处理上，刚 destroy 完立刻建必撞 AlreadyExists
            // （两个变体都可能）——首次立即尝试，撞了稍候重试；事件循环
            // 一两拍内即腾出 label，5 秒上限只是兜底。
            let mut attempted = false;
            let mut result = Err(Error::Window("重建主窗超时：旧 label 迟迟未释放".into()));
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
                        result = Err(Error::Window(format!("重建主窗失败：{cause}")));
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
            log("info", "主窗已重建（WebView2 实例焕新）");
            Ok(())
        }
        Err(cause) => {
            log("error", &format!("主窗重建失败：{cause}"));
            Err(cause)
        }
    }
}

/// 按 tauri.conf.json 主窗参数建窗（运行时 builder 不读配置文件，
/// 参数与配置两处同步改）。
fn build_main(app: &AppHandle, geometry: Option<&Geometry>) -> tauri::Result<()> {
    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("千寻")
        .inner_size(1100.0, 720.0)
        .min_inner_size(880.0, 580.0)
        .decorations(false)
        .visible(false); // 前端就绪后自行亮窗（避免白闪）
    if geometry.is_none() {
        builder = builder.center();
    }
    let _window = builder.build()?;
    #[cfg(windows)]
    apply_webview_preferences(&_window); // COM 偏好（配色/加速键）必须重应用
    if let Some(geometry) = geometry {
        restore(app, geometry);
    }
    // 兜底亮窗：启动路径的 20s 兜底只在 setup 里跑一次，重建自备一份——
    // 前端 boot 若再挂死，窗口至少不会「隐身」。
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
```

### 4.2 `lib.rs`：事件循环改 `build + run(callback)`

原 `.run(context)` 无回调，挡不住重建间隙的退出请求：

```rust
.build(tauri::generate_context!())
.expect("初始化失败")
.run(|_app, event| {
    // 重建间隙：最后一个窗口 Destroyed 触发 code=None 的 ExitRequested，
    // 默认语义是退出整个应用。重建标志置位期间挡下，新窗建成即自动恢复。
    if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
        if window::is_rebuilding() {
            api.prevent_exit();
        }
    }
});
```

### 4.3 入口（托盘/快捷键）：只派发，不等待

```rust
let handle = app.clone();
tauri::async_runtime::spawn(async move {
    if let Err(failure) = window::rebuild_main(&handle).await {
        log("warn", &format!("重建界面失败：{failure}"));
    }
});
```

入口必须在 Rust 侧：白屏时页面 JS 可能已死，前端 `invoke` 不可用。

## 5. 手段阶梯（什么时候用哪个）

| 级别 | 手段                                          | 修什么                                                        | 修不了什么                                                   |
| ---- | --------------------------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------ |
| 1    | `window.reload()`                             | 前端 JS 白屏、导航异常                                        | WebView2 层任何坏状态                                        |
| 2    | **destroy + 重建（本文）**                    | 上述全部 + 渲染/browser 进程挂死、崩溃残留                    | 宿主进程（Rust 侧）自身的问题                                |
| 3    | `app.restart()`                               | 一切（含宿主）                                                | 副作用大：子进程树回收（DSH 等托管进程全重启）、终端会话丢失 |
| 应急 | 任务管理器结束应用名下的 `msedgewebview2.exe` | WebView2 运行时检测到异常退出会自动重启并重载页面，宿主不用死 | 临时手段，不进代码                                           |

## 6. 验证清单

- [ ] 点重建：窗口**可见地消失又重建**，几何/主题保持，前端重新 boot；
- [ ] 日志出现 `主窗已重建（WebView2 实例焕新）`，无 `already exists`；
- [ ] 有独立窗口时：它们不受影响；全无独立窗口时：重建期间进程**不退出**；
- [ ] 重建后托盘「退出」、`app.restart()` 仍正常（退出路径未被误伤）；
- [ ] 前端 boot 挂死时 20s 兜底亮窗生效（不「隐身」）；
- [ ] 失败路径（如手动模拟 label 不释放）：日志有 error，界面无假动作。

## 7. 实测踩坑实录（为什么清单里是这两条）

1. **漏写 `destroy()`**：注释写了「刚 destroy 完立刻建必撞」，但调用本身没写。每次点击
   都以 `a webview with label `main` already exists` 失败，界面毫无动静，只有日志知道。
   教训：**注释不是代码，重构后必须当场验证**。
2. **只匹配一个错误变体**：`WebviewWindow` 是 window + webview 双注册同 label，
   `tauri::Error` 有 `WindowLabelAlreadyExists` 和 `WebviewLabelAlreadyExists` 两个
   变体；实测先抛的是后者，只匹配前者会让重试循环第一次就 break。

## 8. 可选加固：崩溃自动自愈

在已有的 `with_webview` COM 块里 cast 到 `ICoreWebView2_4`，注册 `ProcessFailed`
事件（`RENDER_PROCESS_UNRESPONSIVE` / `RENDER_PROCESS_FAILED` /
`BROWSER_PROCESS_EXITED`），收到即派发 `rebuild_main`——webview 挂死无需用户碰托盘。
本仓库尚未实现，钩子位置在 `window.rs::apply_webview_preferences`。

## 9. 依据索引（tauri 2.11.5 源码）

- label 注册表清理挂在事件循环：`app.rs::on_event_loop_event` → `manager.on_window_close`
  （`Destroyed` 事件时才从注册表移除——这是坑 2 重试的根本原因）；
- `ExitRequested{code:None}` 的发起点与 prevent 通道：`tauri-runtime-wry/src/lib.rs`
  最后一个 `Destroyed` 处理内；`code:Some(_)` 走 `RequestExit`（`app.exit`/`app.restart`）；
- `prevent_exit` 对 `RESTART_EXIT_CODE` 无效（重启不可阻止）：`app.rs::ExitRequestApi`；
- 两个 label 冲突错误变体：`tauri/src/error.rs`（`WindowLabelAlreadyExists` /
  `WebviewLabelAlreadyExists`）。

# 28 · Linux(CEF) 右键菜单与输入法修复

> 记录 2026-11-13 一轮实机排障：千寻在 omarchy(Hyprland/纯 Wayland) 上「右键菜单与输入法候选框变成巨大的窗口」。
> 结论：**右键菜单巨大已根治**（前端 `preventDefault` + 自绘菜单）；**输入法候选框巨大仍未闭环**，需要在图形会话里实测开关。

## 1. 现象

| 现象         | 表现                                                                                                   |
| ------------ | ------------------------------------------------------------------------------------------------------ |
| 右键菜单     | 在千寻页面（顶层或 DSH iframe 内）右键，弹出一个与**主窗口等大**的深色 GTK 面板，只有一个 `Inspect` 项 |
| 输入法候选框 | 输入时 fcitx5 的输入窗覆盖整个窗口（1100x734），完全挡住输入区                                         |

## 2. 根因（三层，逐层实测）

### 2.1 右键菜单不是网页菜单，是 CEF/Chromium 自绘的独立 X11 窗口

- X11 窗口树（自写 ctypes 枚举器，远端无 `xwininfo`/`xprop -tree`）里，右键后多出一个窗口：
  `0xc0000c`，`_NET_WM_WINDOW_TYPE_MENU`、`override-redirect=YES`、`_NET_WM_PID`＝千寻进程自身，
  几何 **1100x734 + (主窗口 +24,+24)**（+24 = `_GTK_FRAME_EXTENTS`）＝与主窗口等大。
- 截图（`.tmp/probe/probe_remote_popup/shots/cap_0xc0000c.png`）是 GTK 深色圆角面板，左上角唯一菜单项 `Inspect`；
  DOM 侧 11 个菜单选择器命中全 0、innerHTML 不变 ⇒ **不是前端 HTML 菜单**。
- CEF 源码：`tauri-runtime-cef-3.0.0-alpha.7/src/cef_impl/client/context_menu.rs:173-209`
  `on_before_context_menu` 只过滤 `BROWSER_ONLY_COMMANDS` / `DEVTOOLS_COMMANDS`（undo/redo/cut/copy/paste/拼写/emoji 等保留），
  注释写明「An empty model is left empty on purpose: CEF then shows no menu at all」
  ⇒ **只要模型非空，CEF 就一定会画菜单**；`client/mod.rs:253-254` 的 `context_menu_handler` 没有开关可关。
- Chromium 侧：这是 `views::MenuRunner` 自绘 popup，在 X11 下就是 override-redirect 窗口
  （`ui/views/controls/menu/menu_controller.cc` 全文无 GTK 引用）。上游同类问题见 CEF #4329（GNOME/Wayland+Xwayland，右键菜单秒消失）与 PR #4346。

### 2.2 输入法候选框是 fcitx5 画的，尺寸由「IM 客户端上报的输入区」决定

- fcitx5 守护进程（pid 92126）的输入窗 `0x400003`（WM_CLASS=fcitx/fcitx、`_NET_WM_WINDOW_TYPE_COMBO`、XEMBED）
  几何 **1100x734**＝整个 webview；位置＝应用窗口位置。
- Chromium 的 IM 客户端是 **dummy window**（`ui/gtk/gtk_util.cc:202-206`：把一个 GTK toplevel 真实化，大小＝浏览器窗口），
  光标矩形经 `gtk_im_context_set_cursor_location` 传入（`ui/gtk/input_method_context_impl_gtk.cc:185-189`、:237），
  缩放换算 `GetDeviceScaleFactor()/gtk_widget_get_scale_factor(dummy)`。
  ⇒ **dummy window 是整窗大，fcitx5 就把候选窗做成整窗大**。
- 候选框由 **ibus/fcitx 守护进程**绘制，不是 Chromium 画的（`ui/base/ime/linux/input_method_auralinux.cc:449-450` 明确「无法检测候选窗/弹窗」）。

### 2.3 实例没有继承图形会话的 IM 环境

- 千寻进程 environ（SSH/tty 侧启动）只有 `DISPLAY=:0` / `XDG_RUNTIME_DIR` / `DBUS_SESSION_BUS_ADDRESS`，
  **没有** `WAYLAND_DISPLAY`、`XMODIFIERS`、`GTK_IM_MODULE`、`GDK_BACKEND`、`LANG/LC_*`；
  而 Hyprland(1218) 与 `systemctl --user show-environment` 都有
  `XMODIFIERS=@im=fcitx`、`WAYLAND_DISPLAY=wayland-1`、`GDK_BACKEND=wayland,x11,*`、`LANG=zh_CN.UTF-8`。
- 进程 locale 为 **POSIX**；root 上有 `XIM_SERVERS=@server=fcitx` 却**始终无 `XIM_CLIENTS`**；
  `fcitx5-diagnose` 的 InputContext 里没有千寻。
- `cef.log` 报 `object class 'GtkSettings' has no property named 'gtk-modules'`
  （GTK4 已移除该属性）⇒ CEF 想注入 GTK IM 模块的那条路也走不通。
- 对照实验：补上 `XMODIFIERS=@im=fcitx` 后，**GTK4 会自动加载 `/usr/lib/gtk-4.0/4.0.0/immodules/libim-fcitx5.so`**
  （不依赖 `GTK_IM_MODULE`；实测只设 `XMODIFIERS` 也会加载），且 IME 组字**不再**产生额外窗口
  ⇒ IM 环境变量解决「输入法不可用」，但**不能**解决「候选框巨大」。

### 2.4 为什么「Chrome 同版本不复现」

千寻的 CEF 内容是 **GTK4 顶层窗口 → `CefX11Host` → CEF child X window** 两层嵌套
（tauri-runtime-cef `webview.rs:851 set_as_child`、`window.rs:486-488`），
并强制 `--ozone-platform=x11` + `GDK_BACKEND=x11`（runtime.rs:3326/3336）。
runtime style 只有 Alloy/Chrome，**没有 Views**（`webview.rs:842-846`）。
⇒ 弹窗是「child-of-child」坐标系下的 popup，坐标/缩放换算与 Chrome 的顶层窗口不同。

## 3. 修法

### 3.1 右键菜单：前端 `preventDefault` + 自绘菜单（已根治）

| 位置       | 文件                                              | 作用                                                                                                                                              |
| ---------- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| 千寻顶层   | `src/components/ContextMenuLayer.svelte:24-36`    | `<svelte:window oncontextmenu>`；**第一动作 `event.preventDefault()`** ⇒ CEF 不再画那个巨大的 override-redirect 弹窗                              |
| 千寻顶层   | `src/lib/window/defaultContextMenu.ts`            | 纯函数 `buildDefaultMenuItems(ctx)` / `describeContextMenuContext(target)`，剪切/复制/粘贴/全选（`navigator.clipboard` 优先，`execCommand` 退路） |
| DSH iframe | `src-tauri/src/remote/assets/qx-shell-ctxmenu.js` | 网关注入脚本（IIFE，`window.__qxShellCtxMenu` 幂等），只在 `window.parent !== window` 时接管；纯 DOM 自绘菜单                                     |

要点：

- 「业务菜单优先」：先看 `event.defaultPrevented`，业务侧 `contextMenu.show()` 内部 `stopPropagation` 使事件到不了 window，再以 `defaultPrevented` 兜底，两边不会叠加。
- 无内容时「只压不弹」：空数组不弹面板（否则会出现空白小面板）。
- iframe 侧脚本由 `src-tauri/src/remote/gateway.rs` 的 `DESKTOP_LAYER_SNIPPET` 注入（`<script src="/qx-shell/ctxmenu.js" defer></script>`），
  新路由 `/qx-shell/ctxmenu.js` **仅回环可取**（LAN Host → 401），与既有 `/qx-shell/links.js` 同风格。

### 3.2 输入法环境自愈：`src-tauri/src/linux_im.rs`（新建）

`pub fn ensure_input_method_env()` 在 `src-tauri/src/lib.rs` 的 `run()` **最开头**（`tauri::Builder` 之前）调用：

- 探测可执行文件（PATH 里存在 + 普通文件 + x 位，不查守护进程）：优先级 `fcitx5 → fcitx → ibus-daemon → ibus`；
  fcitx 族 → `XMODIFIERS=@im=fcitx` + `GTK_IM_MODULE=fcitx`；ibus 族 → `@im=ibus` + `ibus`。
- **只在变量缺失或为空时补**，已有非空值一律不覆盖；一个都没探到时只 warn 不乱设。
- locale：`LANG` 与 `LC_ALL` 都缺失或为 `POSIX`/`C` 时才补 `LANG`（优先 `en_US.UTF-8`，判据 `locale -a`，
  退回 `/usr/lib/locale/en_US.utf8` 目录，都没有则 `C.UTF-8`）；`LC_ALL` 为 `POSIX`/`C` 时一并 `remove_var`
  （glibc 里 `LC_ALL` 优先级最高，会遮蔽 `LANG`）。
- 决策逻辑写成纯函数（`detect_im_from` / `pick_im` / `locale_is_usable` / `decide_lang` / `lc_all_shadows_lang`），Windows 侧可单测。
- 日志走 `crate::logging::log(...)`；此时 `logging::init` 还没跑（它在 `setup()` 里），自动降级 stderr。
- ⚠️ `set_var` 不会改 `/proc/<pid>/environ`（那里记录的是 exec 时的初始环境）——不要用 environ 判断是否生效，看启动日志。

### 3.3 CEF/Chromium 开关口子：`src-tauri/src/cef.rs`（新建，Linux-only）

- 环境变量 `QIANXUN_CEF_ARGS`（逗号/空白分隔，只收 `--` 开头的项）→ 逐条 `Cef::command_line_arg(arg, None::<String>)`。
- 用途：CEF/Chromium 行为开关（输入法、缩放、弹窗）**不用重新编译**，改环境变量重启即可试。
- CEF API 事实（`tauri-runtime-cef-3.0.0-alpha.7/src/runtime.rs`）：
  `command_line_arg`（L583，对所有进程生效）、`command_line_args`（L598，仅 browser 进程）、
  `disable_features`(L919)/`enable_features`(L933)（逗号列表必须合并，`--disable-features` 是替换语义）。

## 4. 实测结果（远端 192.168.20.3，omarchy/Arch，CEF 154.0.8037.98）

远端构建：`cargo build --features tauri/custom-protocol` **首次编译通过**（cef.rs + linux_im.rs 的 platform 模块都是 Linux-only）。

| 检查项          | 结果                                                                                                                                                                                      |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| IM 环境自愈     | 日志：`补齐输入法环境：XMODIFIERS=@im=fcitx（探测自 /usr/bin/fcitx5）`、`补齐输入法环境：GTK_IM_MODULE=fcitx（探测自 /usr/bin/fcitx5）`、`补齐 locale：LANG=en_US.UTF-8（原值 <未设置>）` |
| 顶层页右键      | `ctx=1`（事件到达渲染进程），**新增 X11 窗口数 = 0**；空白处无菜单（只压不弹）                                                                                                            |
| DSH iframe 右键 | `window.__qxShellCtxMenu === true`，script 列表含 `/qx-shell/links.js` 与 `/qx-shell/ctxmenu.js`；选中文字右键 `ctx=3`、`injectedMenu=true`、`items=["复制"]`，**新增 X11 窗口数 = 0**    |
| 注入脚本可达性  | `GET /qx-shell/ctxmenu.js` → 200 + `content-type: text/javascript; charset=utf-8` + `cache-control: no-cache`；LAN Host → 401                                                             |
| 方案 A 未回归   | `harness_start` → `http://127.0.0.1:3080`，日志「[DSH] 已连接到正在运行的实例（127.0.0.1:3080）」「网关使用自签 cookie 连接共享的 DSH 实例」                                              |

本地门禁全绿：prettier / eslint / svelte-check / vitest / `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test`（228 passed / 0 failed / 6 ignored）。

## 5. 未闭环：输入法候选框仍然巨大

- 现象：fcitx5 输入窗 `0x400003` = 1100x734（整个 webview）。
- 归因：Chromium 的 GTK IM **dummy window** 大小＝浏览器窗口 ⇒ fcitx5 按整窗创建候选窗（§2.2）。
- 为什么远端 SSH 实例验证不了：X input focus 恒为 `0x1 (PointerRoot)`（SSH 起的实例没有 WM 焦点），
  `XSetInputFocus` 成功后被 XWayland 覆盖成 `0x0`，XTEST 按键进不了渲染进程；
  `hyprctl dispatch focuswindow address:0x…` 在新版 Hyprland 改用 lua 语法，旧语法报
  `error: 3 [string "return hl.dispatch(focuswindow address:0x6000..."]:1: ')' expected near 'address'`。
  ⇒ **真实 IME 交互只能由用户在自己的图形会话里验证**。
- 候选修法（都可用 §3.3 的环境变量口子试，不用改代码）：
  - `QIANXUN_CEF_ARGS=--disable-gtk-ime` —— Chromium 不再用 GTK IM（绕过 dummy window），改走 XIM。
  - `QIANXUN_CEF_ARGS=--enable-wayland-ime` —— 反向验证 IME 路径归属（CEF 目前无 Wayland 支持，见 #2804/#4233）。
- 远端机器本身：Hyprland **纯 Wayland** + fcitx5，原生 Wayland 应用走 text-input 协议（用户在远端自己那轮 DSH 会话里已确认 fcitx5 拼音可用、词典齐全）；
  千寻因 CEF 无 Wayland 支持只能走 XWayland，IM 路径天然更「重」。

## 6. 经验沉淀

1. **CEF 原生 popup 无法用配置关掉**：`context_menu_handler` 没有开关；唯一可靠手段是前端 `preventDefault`（capture 阶段，第一动作）。
2. **弹窗类问题先看 X11 窗口树，不要先看 DOM**：DOM 命中 0 + 多出一个 override-redirect 窗口 = 原生弹窗。
3. **IM 类问题先看进程 environ**：`XMODIFIERS`/`GTK_IM_MODULE`/`LANG`/`WAYLAND_DISPLAY` 缺一项就会走 XIM 这条退化路径。
4. **GTK4 的 IM 模块由 `XMODIFIERS` 决定**（不是 `GTK_IM_MODULE`）；GTK4 没有 `gtk-modules` 这个 GtkSettings 属性，CEF 注入会失败。
5. **Linux-only 代码必须远端 `cargo build` 证伪**：本轮前一次远端编译炸了 5 个错（i32/u32 混算、`?` 用在 Result 上、参数名遮蔽同名函数、未用 import、未用参数），本地 Windows 门禁全绿也看不出来。
6. **探测远端窗口归属的手段**：远端缺 `xwininfo`/`xprop -tree`/`xdotool`/`wmctrl`；用 ctypes 调 libX11 自己枚举（**先装 `XSetErrorHandler` 并只取只读子集，否则整条 ssh 会 segfault**）；`grim` 在 Wayland 下抓不到 XWayland 窗口（整屏全黑），只能 `import -window <hexid>` 抓单窗。
7. **CDP 探针与 iframe**：iframe 在别的页签里是 `display:none`（无布局盒）→ 必须先点 `[data-testid="nav-dsh"]` 切页并 sleep ~2.5s；切页后 iframe target 会重建，要重新取 `/json/list` 的 `webSocketDebuggerUrl`。

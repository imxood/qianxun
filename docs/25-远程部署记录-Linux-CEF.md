# 25 - 远程部署记录：Arch Linux + Tauri v3 CEF 内嵌 Chromium（2026-11-13）

> 目标（用户 m00665）：把千寻源码在远程 omarchy（Arch，192.168.20.3）上编译部署，
> 支持「不内置 DSH、直接用全局 npm 安装的 dsh」，用 Tauri v3 的 **内嵌 Chromium（CEF）**
> 运行，并通过 CDP 协议验证。
> 状态：**已闭环**——远程编译通过、应用跑起、CDP 功能探针全部通过、截图存档。
> 主机：`maxu@192.168.20.3`（omarchy / Arch，内核 7.2.15-1-omarchy，20 核 / 31G）。

## 1. 结论速览

| 项       | 结果                                                                                                                                          |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| 编译     | `cargo build --features tauri/custom-protocol` → exit 0，产物 `target/debug/qianxun`（739,781,072 B）                                         |
| 内核     | CEF 154.0.34（Chromium **154.0.8037.98**），走 GTK4 + XWayland（X11）                                                                         |
| CDP      | `http://127.0.0.1:10222/json/version` → `Chrome/154.0.8037.98`，Protocol-Version 1.3                                                          |
| 页面     | page target `title=千寻` `url=http://tauri.localhost/`；iframe target `http://127.0.0.1:23091/`（DSH 页已加载）                               |
| DSH      | **不内置**：`dshSource=external`、`dshExternalPath=/home/maxu/.local/share/mise/shims/dsh`、`dshVersion=0.2.2-rc.1`、`dshVersionMatches=true` |
| DSH 状态 | `phase=ready`，origin `http://127.0.0.1:34957`，pid 240082；网关 `http://127.0.0.1:23091` 已向 DSH 兑换登录 cookie                            |
| 截图     | `~/qianxun-shot.png`（123,844 B）——UI 显示「来源：全局安装」「运行中 · … · v0.2.2-rc.1」                                                      |

## 2. 代码改动（本轮落盘，按关注点分两笔 commit）

### 2.1 Tauri v3 + 按平台分家 runtime（commit 1）

- `src-tauri/Cargo.toml`
  - `[workspace]` 加 `resolver = "2"`：否则 target 专属依赖（Windows 的 wry → tauri/gtk3）会把 feature 泄漏到 Linux 构建，害 CEF 路线同时拖进 GTK3+GTK4 两套绑定。
  - `tauri-runtime-wry` 与 `xcap` 移入 `[target.'cfg(windows)'.dependencies]`；新增 `[target.'cfg(target_os = "linux")'.dependencies]` 放 `tauri-runtime-cef = { version = "3.0.0-alpha.7", features = ["devtools"] }`。
  - `tauri-plugin-dialog` 改 `default-features = false, features = ["xdg-portal"]`：默认的 gtk3 只对 Linux/BSD 生效，但 **GTK3 与 GTK4 官方不允许同进程共存**（CEF 路线走 GTK4）。rfd 的 build.rs 要求 xdg-portal 必须配 tokio/async-std 之一，插件的 xdg-portal feature 自带 `rfd/tokio`，约束满足。
- `src-tauri/src/lib.rs:124-131` — runtime 按 target 分家：Windows → `tauri_runtime_wry::Wry::default()`；Linux → `linux_cef_runtime()`（v3 不选 runtime 会报 `RuntimeNotConfigured`）。
- `src-tauri/src/cef.rs`（新文件）— Linux runtime 装配：`Cef::default().remote_debugging(RemoteDebugging::Port{ port, allowed_origins })`；端口取 `QIANXUN_CDP_PORT`，缺省 debug 构建用 10222（合法区间 1024–65535）；`allowed_origins` 取 `QIANXUN_CDP_ORIGINS`（逗号分隔）。
- `src-tauri/src/main.rs:6-13` — `#[cfg_attr(target_os = "linux", tauri_runtime_cef::cef_entry_point)]`：CEF 子进程（`--type=`）靠这个宏转交 `run_cef_helper_process()`。该属性在无返回类型的 `fn main()` 上可用（已实读 tauri-macros-3.0.0-alpha.4/src/cef.rs 确认两分支都带 `let`）。
- `src-tauri/src/shots/commands.rs` — xcap / `hwnd()` 全部 cfg 门控：Linux 上 `shots_capture` 返回明确错误「截屏功能当前仅支持 Windows」，不 panic、不静默返回空数组；11 个 IPC 命令名与签名不变。
- `src-tauri/src/window.rs` — `with_webview` → `tauri_runtime_wry::WebviewWryExt::with_wry_webview`（v3 迁移）；非 Windows 的 `let spawned` → `let _spawned`、`on_theme_changed` 加 `#[cfg(not(windows))] let _ = app;`（Linux 无 WebView2 偏好可设）。
- `src-tauri/tauri.linux.conf.json`（新文件）— 平台专属配置：`bundle.targets = ["deb","appimage"]`、`category=DeveloperTool`。按 tauri-utils 的 RFC 7396 JSON Merge Patch 与主配置合并。
- `src-tauri/crates/fff-core/src/watcher/background_watcher.rs:713-717` — Linux 专属分支里的 `warn!(?e, dir = %dir.display(), "…")` 是 tracing 风格简写，log 0.4 的 kv 形式不认 → 改成普通格式化 `warn!`。

### 2.2 不内置 DSH（commit 2）

- `src-tauri/src/harness/external.rs`（新文件）— 全局 dsh 探测：`QIANXUN_DSH_BIN` 显式路径 → PATH 上的 `dsh`/`dsh.exe`/`dsh.cmd` → 已知全局目录（`~/.local/share/mise/shims/dsh`、`/usr/local/bin`、`~/.local/bin`、`~/.npm-global/bin`、`~/.nvm/versions/node/*/bin` 按数字版本取新、`~/AppData/Roaming/npm`）。版本靠 `<exe> --version` 解析（跳过 mise WARN 噪声行），5s 超时、探测预算 8 次。
- `src-tauri/src/settings.rs` — `DshSettings` 新增 `source`（`bundled` 默认 / `external`），并入 `validate()` 白名单；旧 settings.json 缺字段自动回落 `bundled`。
- `src-tauri/src/harness/mod.rs` — `Environment` 增 `dsh_source` / `dsh_external_path`；external 分支走 `external::resolve()`，不做严格版本相等（`external_version_compatible` 按 release 三段全等判定，`0.2.2-rc.1` 对 `0.2.2-rc.2` 判兼容）。
- `src-tauri/src/harness/supervisor.rs` — `LaunchPlan` 的 `node + entry` 拆成 `program: PathBuf + entry: Option<PathBuf>`：外部模式直接跑全局 dsh（不再 `node bin.js`），参数序 `--profile <p> --no-open --host <h> --port <port>` 与 DSH_DESKTOP/QIANXUN_VERSION/DSH_HOME env 保持不变。
- `src-tauri/src/error.rs` / `harness/commands.rs` — 新增 `DshExternalNotFound` / `DshExternalManagedInstallUnsupported`；external 模式下托管安装命令明确报错。
- `src/features/settings/SettingsPage.svelte` / `src/features/env/EnvPage.svelte` / `src/lib/ipc/contract.ts` — 设置页加「DSH 来源」下拉（bundled / external）；环境页显示「来源：全局安装」与全局 dsh 路径 + 版本，external 时隐藏托管安装/升级按钮。
- `src-tauri/src/harness/install.rs:36` — `PINNED_VERSION` 由 `0.2.2-rc.1` 升到 `0.2.2-rc.2`（npm 双源已上架实测）。

## 3. 远程构建配方（可复现）

### 3.1 CEF 运行库（离线配方 B，绕开上游 index.json 10 MiB 上限）

上游 `download-cef 3.0.0` 用 ureq 读 `index.json`，而该文件已 10,629,364 B > ureq 的 10 MiB 上限，
默认 `cargo build` 必失败（原文：`Error: HTTP request error: json: the response body is larger than request limit: 10485760`）。
绕过办法（无需改上游）：把 CEF 发行版手工铺到 `CEF_PATH` 约定目录并写好 `archive.json`。

```
缓存层级：$CEF_PATH/<CEF 版本>/<OS_ARCH>/   ← 版本目录名 = crate build metadata，OS_ARCH 用下划线
例：$HOME/.cache/tauri-cef/154.0.34/cef_linux_x86_64/
内容：tarball 的 Release/* 与 Resources/* 平铺 + include/ libcef_dll/ cmake/
      locales/ CMakeLists.txt CREDITS.html + archive.json
archive.json（必需，167B）：
{"type":"minimal","name":"cef_binary_154.0.34+g14c5a08+chromium-154.0.8037.98_linux64_minimal.tar.bz2","sha1":"15c91ad767097217b2e82ba8b933fe30b134be69"}
```

归档下载地址：`https://cef-builds.spotifycdn.com/cef_binary_154.0.34+g14c5a08+chromium-154.0.8037.98_linux64_minimal.tar.bz2`
（326,489,962 B，实测 33 s 下载，sha1 校验一致）。

### 3.2 构建与运行

```bash
export PATH="$HOME/.cargo/bin:$PATH"          # cargo 1.99.1 不在默认 PATH
export CEF_PATH="$HOME/.cache/tauri-cef"
cd ~/qianxun-src/src-tauri
cargo build --features tauri/custom-protocol # custom-protocol 才能加载打包进二进制的 dist
```

- `custom-protocol` 是 tauri 的 production 开关（tauri-macros 的 `dev: cfg!(not(feature="custom-protocol"))`），不启用就走 dev 语义连 devUrl；tauri CLI 构建时会自动注入，手工 cargo 必须自己带。
- Linux 上 cef-dll-sys **不需要** cmake/ninja/gcc（`.build()` 只在 win/mac 分支调用），Linux 直接用发行版预编译 libcef.so；构建脚本会把 CEF 文件复制到 `target/<profile>/`（二进制旁）。
- **libcef.so 是硬 DT_NEEDED 且发行包无 RPATH** → 运行期必须给库路径：
  ```bash
  export LD_LIBRARY_PATH=~/qianxun-src/src-tauri/target/debug
  # 或构建期 RUSTFLAGS="-C link-arg=-Wl,-rpath,$ORIGIN"
  ```
- 其他运行期变量：`DISPLAY=:0`（rootless XWayland，CEF runtime 强制 `--ozone-platform=x11` + `GDK_BACKEND=x11`）、
  `QIANXUN_CDP_PORT=10222`、`QIANXUN_DATA_DIR=$HOME/qianxun-verify-data`。
- 外部 DSH 模式：数据根 `settings.json` 写 `dsh.source="external"`（旧文件缺字段会回落 bundled）。

## 4. 源码传输（踩坑记录）

- 白名单打包（`src-tauri dist src scripts e2e packages` + 13 个根文件），剪掉 `node_modules/.git/.tmp/target/**` 等
  → **615 files / 9.47 MB raw / 2.92 MB gz，传输 4.5 s**。
- 踩坑 1：`pathlib.rglob` 遍历进 `src-tauri/target`（192 GB）→ 10 分钟没结束，被 kill。
- 踩坑 2：`os.walk` 剪枝漏了 `models/`(2.8 GB)、`mobile-flutter/`(4 GB)、`mobile/`(1.6 GB) → 打包 8.5 GB，传输超时被 kill。
- 远程用自带的 node 26.13.1 + pnpm 12.18.0 跑前端构建，不需要传 node_modules。
- Git：远程只能走 **HTTPS**（`git ls-remote https://github.com/imxood/qianxun.git` 通；SSH 被 clash fake-ip 拦：`Connection closed by 198.18.0.12 port 22`）。

## 5. CDP 验证（实测输出）

远程探针用 **Node 26 内置 WebSocket 直连 CDP**（远程没装 playwright，且根 package.json 有 `"type":"module"`，
`.js` 会被按 ESM 解析报 `require is not defined in ES module scope` → 探针写成 `.cjs`）。

```json
// GET /json/version
{
  "Browser": "Chrome/154.0.8037.98",
  "Protocol-Version": "1.3",
  "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36",
  "V8-Version": "15.4.80.20",
  "webSocketDebuggerUrl": "ws://127.0.0.1:10222/devtools/browser/5250c1bc-c0f6-480d-aa69-e72e7b1ace81"
}
```

`GET /json/list`：page target `title=千寻 url=http://tauri.localhost/`；iframe target `http://127.0.0.1:23091/`。

CDP 功能探针（`Runtime.evaluate` 调 Tauri IPC）结果：

| 调用                  | 返回要点                                                                                                                                                                                                                                                                       |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `harness_environment` | `dshInstalled=true`、`dshVersion="0.2.2-rc.1"`、`dshVersionMatches=true`、`installSpec="@deepseek-ai/dsh@0.2.2-rc.2"`、`dshSource="external"`、`dshExternalPath="/home/maxu/.local/share/mise/shims/dsh"`、`dshHome="/home/maxu/.dsh"`、`node=null`（外部模式不要求本机 Node） |
| `harness_status`      | `phase="ready"`、`origin="http://127.0.0.1:34957"`、`pid=240082`                                                                                                                                                                                                               |
| `harness_proxy_url`   | `http://127.0.0.1:23091`                                                                                                                                                                                                                                                       |
| 页面文本              | 「运行中 · http://127.0.0.1:34957 · v0.2.2-rc.1」「来源：全局安装」「当前使用全局 DSH（千寻不托管安装与升级）」                                                                                                                                                                |

DSH 进程证据（**来自 mise 全局安装，不是千寻私有 prefix**）：

```
node .../mise/@deepseek-ai+dsh@0.2.2-rc.1_.../node_modules/@deepseek-ai/dsh/lib/bin.js \
     --profile web --no-open --host 127.0.0.1 --port 0
```

子进程：`billion-context/dist/index.js start --host 127.0.0.1 --port 18787`、`qx-playwright/launcher.js`、`@playwright/mcp/cli.js`。
监听端口：10222（CEF DevTools）、23091（千寻回环网关）。

CEF 进程树：主进程 + 4 个 `--type=zygote` 子进程（带 `--no-zygote-sandbox`，说明走的是非特权 userns 沙箱，
没触发 Chromium 的 `No usable sandbox!` FATAL）；user-data-dir = `/home/maxu/.cache/com.qianxun.desktop/cef`。

## 6. 已知噪声（均非阻断）

- `Fontconfig error: Cannot load default config file: File not found`（远端 fontconfig 未配默认配置）
- `GetVSyncParametersIfAvailable() failed` / `Unable to get gpu adapter`（无 GPU 环境）
- `Gtk-WARNING: gtk_disable_setlocale() must be called before gtk_init()`、`GtkSettings has no property named 'gtk-modules'`、`Use GtkSettings:gtk-interface-color-scheme instead`
- `mise WARN deprecated [system_tool_version]`（DSH 由 mise shim 启动时必现；版本探测已跳过该噪声行）
- `dsh: all_proxy names a SOCKS proxy, which is not supported`（远端 all_proxy 是 socks5）
- 情报域 LLM 429（`open.bigmodel.cn` 限流，与 CEF 路线无关）

## 7. Linux 窗口交互：CEF 的两条硬限制与解法（2026-11-13 追加）

远程首跑后暴露两个用户可见问题：**窗口拖不动**、**最小化没反应**；DSH 内嵌页在网关重建窗口期还会留下 Chromium
「127.0.0.1 refused to connect」错误页。三件事都查清了根因并已修。

### 7.1 拖拽失效：CEF 的浏览器是 X11 子窗口，GTK toplevel 收不到指针

CEF 路线下窗口层级是 `GtkApplicationWindow → GtkBox → CefX11Host(X11 子窗口) → CEF webview`
（`tauri-runtime-cef-3.0.0-alpha.7/src/platform/linux/window.rs:37/58/73/75`、
`src/platform/linux/webview.rs:37/106`）。鼠标事件全部落在 reparent 进来的 CEF 子窗口上，GTK toplevel 收不到
button press，于是 winit-gtk4 的 `pointer_button_press` 永远是 `None`：

```
tauri-winit-gtk4-0.31.0-beta.3/src/window/mod.rs:946
  let Some(press) = self.pointer_button_press.lock().unwrap().clone() else {
      return Err(NotSupportedError::new("window dragging requires a pointer button press").into());
  };
```

而 CEF runtime 把这个错误直接丢掉（`tauri-runtime-cef/src/window.rs:1282`
`WindowMessage::StartDragging => _ = window.drag_window();`），表现为「点了没反应」。

**「让窗口管理器接管标题栏」这条路实测走不通**：GTK4 在 X11 下走客户端装饰（CSD），
远程 `xprop` 显示千寻窗口没有 `_NET_FRAME_EXTENTS` / `_GTK_FRAME_EXTENTS`，对照实验系统 GTK4 应用
（`/usr/bin/gliff`）同样没有；且 `WM_PROTOCOLS` 里根本没有 `WM_STATE`。

### 7.2 最小化失效：GTK4 在 XWayland 下不产生 WM_STATE=Iconic

`plugin:window|minimize` 的 IPC 是通的（返回 ok），但远程 `xprop` 显示 minimize 前后 `WM_STATE` 恒为 `Normal`；
GTK4 的 `gtk_window_minimize()` 在 X11 下没有让 XWayland 侧图标化。**Linux 上最小化改为「隐藏到托盘」**
（`win.hide()`），语义上等价于收起窗口。

### 7.3 采用的修法：标题栏自绘 + 前端自己拖拽

- `src/lib/window/linuxWindow.ts`（新文件）：`isLinuxRuntime()` 走 `app_meta` 拿平台；`attachLinuxDrag()`
  用 pointer 事件算位移，再调 `win.setPosition(new PhysicalPosition(x, y))`。
- `src/components/TitleBar.svelte`、`src/StandaloneWindow.svelte`：Linux 下把
  `data-tauri-drag-region` 置 `false`（让 Tauri 内置 drag.js 别插手），改挂自己的 pointer 监听；
  最小化按钮在 Linux 走 `win.hide()`。
- `src-tauri/capabilities/default.json` 与 `capabilities/standalone-windows.json`：新增
  `core:window:allow-hide` / `allow-outer-position` / `allow-set-position` / `allow-scale-factor`
  （`set_position` 不在 `core:window:default` 里，缺了会被 ACL 拒）。
- `src-tauri/src/lib.rs`：`app_meta` 增加 `platform` 字段（`std::env::consts::OS`），前端据此判平台。

**踩坑：拖拽第一版用 `requestAnimationFrame` 节流，结果窗口完全不动。** 根因是窗口被遮挡时 Chromium 会挂起
渲染，rAF 回调不再触发；改成直接调 `set_position` 后一次通过。实测（CDP 合成 pointer 事件）：

```
outer_position: {"x":474,"y":222}
after synthetic drag {"x":534,"y":262}   # 位移 +60/+40，与合成的指针位移一致
```

> `set_position` 的入参在 v3 是枚举：`PhysicalPosition` 会被 JS API 包成 `Position`，
> 序列化成 `{"Physical":{"x":…,"y":…}}`；直接 `invoke('plugin:window|set_position', {value:{x,y}})`
> 会被 Rust 拒：`invalid args \`value\` … expected map with a single key`。

### 7.4 DSH 内嵌页错误页：重启后 iframe 没重建

DSH 重启瞬间千寻回环网关会被整体重建（`src-tauri/src/remote/gateway.rs:264-286` 的 `signal.changed()`
分支会丢弃全部 server），此时 iframe 发起的 Document 导航被拒 → Chromium 渲染错误页；
之后 iframe 的 `src` 没变（代理地址恒为 23091），Svelte 不会重挂，错误页就一直留着。

`src/features/dsh/DshPage.svelte` 增加 `frameEpoch`：`phase` 由非 ready 回到 ready 或 `origin` 变化时自增，
iframe 用 `{#key frameEpoch}` 包住强制重建文档。实测（重启前后）：

```
[after start]  status={"phase":"ready","origin":"http://127.0.0.1:38951",…}  iframes=[{"src":"http://127.0.0.1:23091/"}]
[after restart] status={"phase":"ready","origin":"http://127.0.0.1:39115",…}  iframes=[{"src":"http://127.0.0.1:23091/"}]
network events -> [{"loadingFailed":"net::ERR_ABORTED","type":"Document"},
                   {"url":"http://127.0.0.1:23091/","status":200,"type":"Document"}]
```

旧文档被中止（ERR_ABORTED）、新文档 200 —— 错误页自愈。

## 8. 未闭环 / 后续可做

- Linux 上 **托盘菜单**走 GTK3/GTK4 混用的顾虑已通过 dialog 走 xdg-portal 规避，但托盘本体（ksni）与截图功能仍是 Windows 专属（截图在非 Windows 返回明确错误）。
- Linux 无单实例机制（`single_instance` 模块整体 cfg(windows)），并发启动会起多个实例；本次未实现 flock/DBus 替代。
- 远程 pacman 默认镜像不可用（`stable-mirror.omarchy.org` TLS error）；`mirrorlist.bak` 的国内镜像版本号与本机 DB 不一致，装包需手工核对文件名再 `pacman -U`。
- 未做 Linux 打包产物（deb/appimage）实跑，仅配好 `bundle.targets`。
- Linux 拖拽/最小化已用 CDP 合成事件验证；**真实鼠标拖拽与最小化仍需用户在远程桌面手动确认**。
- 用户红线仍然生效：全程未写入/未重装/未终止任何 release 路径内容（安装目录 `C:\Users\maxu\AppData\Local\qianxun\` 与 `~/.qianxun`）。

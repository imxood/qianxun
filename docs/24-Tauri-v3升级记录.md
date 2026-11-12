# 24 - Tauri v3 升级记录（2026-11-12）

> 状态：**debug 构建通过并已启动；后续在 Arch Linux 上以 CEF 内嵌 Chromium 路线完成远程部署与 CDP 验证**（见 `docs/25-远程部署记录-Linux-CEF.md`）。本文档记录的是 Windows 侧的 v3 迁移过程。
> 目标（用户 m00047）：把千寻使用的 Tauri 升级到 v3 最新版，debug 模式启动供验证。

## 1. 版本对照

| 依赖 | 升级前 | 升级后 |
| --- | --- | --- |
| tauri | 2（2.12.5） | 3.0.0-alpha.6 |
| tauri-build | 2 | 3.0.0-alpha.4 |
| tauri-runtime-wry | （tauri 内置 feature） | 3.0.0-alpha.6（独立 crate，新增） |
| tauri-plugin-opener | 2.5.5 | 3.0.0-alpha.3 |
| tauri-plugin-dialog | 2.7.3 | 3.0.0-alpha.4 |
| tauri-plugin-clipboard-manager | 2.3.3 | 3.0.0-alpha.3 |
| tauri-plugin-global-shortcut | 2.3.2 | 3.0.0-alpha.3 |
| tauri-plugin-autostart | 2 | 3.0.0-alpha.3 |
| @tauri-apps/api | ^2.12.4 | 3.0.0-alpha.3 |
| @tauri-apps/cli | ^2.11.4 | 3.0.0-alpha.6 |
| @tauri-apps/plugin-dialog | ^2.7.3 | 3.0.0-alpha.4 |
| @tauri-apps/plugin-opener | ^2.5.5 | 3.0.0-alpha.3 |

间接依赖实测：wry 0.57.0、tao 0.37.1、webview2-com 0.39.1（与项目原有 pin 一致，COM 版本未动）。
运行时仍是 **wry/WebView2**（Windows = Chromium，实测 Edg/157.0.4341.46）；未启用 CEF runtime。

## 2. 改动清单

- `src-tauri/Cargo.toml:31,39-42,76,78,81,113` — 版本升级；tauri 去掉 `devtools` feature，新增 `tauri-runtime-wry`（devtools 移到 runtime crate 上开）。
- `src-tauri/src/lib.rs:122` — 注入 runtime：`let builder = builder.runtime(tauri_runtime_wry::Wry::default());`（v3 不选 runtime 会报 `RuntimeNotConfigured`）。
- `src-tauri/src/tray.rs:17-22` — 菜单 static 泛型：`MenuItem<tauri::DynRuntime>` / `CheckMenuItem<tauri::DynRuntime>`（v3 菜单类型**无默认泛型**，AppHandle/TrayIcon 才有）。
- `src-tauri/src/tray.rs:58,159` — autostart breaking：`ManagerExt::autolaunch()` → `.autostart()`。
- `src-tauri/src/window.rs:443,452` — `with_webview` → `tauri_runtime_wry::WebviewWryExt::with_wry_webview`（WebView2 COM 加速键/配色设置）。
- `src-tauri/capabilities/default.json:19` — ACL 改名：`opener:allow-reveal-item-in-dir` → `opener:allow-reveal-items-in-dir`。
- `src/features/search/locate.ts:1,38`、`src/features/search/disk/DiskScan.svelte:43,419,1271` — JS API 改名：`revealItemInDir` → `revealItemsInDir`（新签名接受 string | string[]）。
- `package.json` — 4 个 npm 包升 3.x alpha。
- `src-tauri/tauri.conf.json`、`src-tauri/tauri.debug.conf.json`、`e2e/tauri.e2e.conf.json` — `$schema` config/2 → config/3。
- `src-tauri/Cargo.lock` — `cargo fetch` 最小化重解析（只动必需项）。
- `vite.config.ts:24` — `server.watch.ignored` 追加 `**/docs/**` 与根级 AGENTS/README/TODO.md，杜绝文档编辑触发整页 reload（见踩坑 8）。

## 3. 踩坑与 breaking 点（实测）

1. **Cargo.lock 未刷新 → CLI 版本一致性检查拦截**：改 Cargo.toml 后必须先 `cargo fetch`（或 cargo update）再 tauri build，否则报 `Found version mismatched Tauri packages`。
2. **opener 插件双改名**（CHANGELOG 明确 Breaking）：Rust 命令 `reveal_item_in_dir` → `reveal_items_in_dir`，JS `revealItemInDir` → `revealItemsInDir`，ACL 权限同步改。Rust 侧保留 `reveal_item_in_dir` 兼容函数，但命令/权限名不兼容。
3. **autostart 插件**：`ManagerExt::autolaunch` → `ManagerExt::autostart`（仅改名，init 签名不变）。
4. **菜单泛型**：迁移指南说"默认 DynRuntime"只对 AppHandle/Window/TrayIcon 等成立；`MenuItem`/`CheckMenuItem` 必须显式 `<tauri::DynRuntime>`，否则 E0107。
5. dialog/clipboard-manager/global-shortcut 三个插件 3.x alpha **无 breaking**（已逐一核对 CHANGELOG）；dialog 仅移除 deprecated `okLabel`（项目未用）。
6. `additionalBrowserArgs`（CDP 10222 注入）在 v3 配置中保留，debug 基建无需改。
7. vite 只监听 IPv6 `::1`：探活要用 `http://localhost:5190/`，打 `127.0.0.1` 会误报拒绝（Node 24 行为）。
8. **改 md 会不会引起 Tauri 重启重编**（2026-11-12 用户问，实测+源码分析）：
   - Tauri 端**不会**：tauri-cli watcher 只监听 `src-tauri/` + workspace 路径依赖（`src-tauri/crates/*`）+ `--watch-folders` 额外目录（源码 `crates/tauri-cli/src/interface/rust.rs` `get_watch_folders`/`run_dev_watcher`）。`docs/` 在项目根，不在范围。实测改 docs/04 后 exe mtime 与进程创建时间均不变。例外：md 若放在 `src-tauri/` 内会触发 watcher → 杀进程重启（cargo 增量空转不重编，但进程重启）。
   - Vite 端**会整页 reload**：docs/*.md 不在模块图，HMR 无法处理 → 回退 full reload（实测改 docs/04 后 1 秒日志出现 `[vite] (client) page reload docs/04-…`，ws-bridge 随之断连）。已修：`vite.config.ts:24` `server.watch.ignored` 追加 `**/docs/**`、`**/AGENTS.md`、`**/README.md`、`**/TODO.md`。受控验证：touch docs 文件 mtime 后 0 新增日志、无 reload。注意改 vite.config.ts 本身会让 Vite 重启一次。

## 4. 验证结果（2026-11-12 23:25 实测）

- `pnpm tauri build --debug --no-bundle --config src-tauri/tauri.debug.conf.json` exit=0（日志 `.tmp/logs/tauri3-build_20261112.log`）。
- `pnpm dev` 已启动：页面 `http://localhost:5190/` 标题「千寻」加载成功；CDP `http://127.0.0.1:10222/json/version` 返回 Edg/157.0.4341.46。
- 进程共存确认：debug 二进制 `src-tauri\target\debug\qianxun.exe`（PID 24716）与用户安装版 `C:\Users\maxu\AppData\Local\qianxun\qianxun.exe`（PID 5208）同时运行——路径级 single-instance 插件按预期互不干扰。

## 5. 未闭环项

- [ ] 用户人工验证 UI/功能（托盘、开机自启、搜索定位、磁盘扫描右键「在资源管理器中显示」、对话框、全局快捷键、剪贴板）。
- [ ] `pnpm all:check`（prettier/eslint/svelte-check/vitest/cargo fmt/clippy/test）未跑。
- [ ] e2e（`pnpm e2e:build && pnpm e2e`）未跑；e2e 脚本若引用 opener 旧 API 需同步。
- [x] CEF runtime（内嵌 Chromium）——已在 Linux 路线启用，见 `docs/25-远程部署记录-Linux-CEF.md`（Windows 仍走 wry/WebView2）。
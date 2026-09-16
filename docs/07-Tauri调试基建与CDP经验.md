# 07 · Tauri 调试基建与 CDP 经验

状态：实战提炼（2026-09-16，千寻 v0.5.0）。实测环境：**tauri 2.11.5 /
tauri-runtime-wry 2.11.4 / WebView2 Evergreen / Windows 11**。
适用场景：任何 Tauri 2 桌面项目想让 debug 二进制可被 Playwright 经
CDP 远程驱动（取代半自动 `pnpm dev` + 手动看 console 的低效模式）。

本仓库实现：`src-tauri/src/main.rs`（env 隔离）、`src-tauri/tauri.debug.conf.json`
（CDP 端口）、`package.json`（`dev` script 挂 `--config`）。
端到端探针模板见 `AGENTS.md` §4。

---

## 1. 目标

```text
终端 1：pnpm dev
  → vite 5190
  → qianxun.exe (debug) 自动起，复用 EBWebView-dev profile
  → 启动时给 WebView2 注入 --remote-debugging-port=10222

终端 2：npx tsx e2e/probe-xxx.ts
  → chromium.connectOverCDP('http://127.0.0.1:10222')
  → 拿 page、跑交互、抓状态，零 mock 全真实环境
```

预期效果：`pnpm dev` 与 release 千寻**完全并存**（数据目录、WebView2
profile、调试端口三件套各自隔离），互不打架。

## 2. 四层踩坑，按发现顺序

### 2.1 坑 1：env var `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 永远无效

最初尝试在 `main.rs` 里 `std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
"--remote-debugging-port=10222")`。看起来"早于 WebView2 启动"应该管用——

实际看 `wry-0.55.1/src/webview2/mod.rs:294-327`：

```rust
let additional_browser_args = pl_attrs.additional_browser_args.unwrap_or_else(|| {
    let default_args = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
    // ... 还可能追加 autoplay / proxy
    arguments
});
// 然后无条件：
unsafe { options.set_additional_browser_arguments(additional_browser_args); };
```

wry **每次都显式**调 `options.set_additional_browser_arguments`——即使
你没传 `additional_browser_args`，它也会传一个默认值。

而 WebView2 的优先级是 **options 显式值 > env var**。env 仅是
options 未设置时的默认值来源；一旦 options 被显式 set，env 就被忽略。

**结论**：在 Tauri/wry 架构下，env 形式永远无法生效。CDP 端口必须
通过 Tauri 的配置口传入（`additionalBrowserArgs`）。

### 2.2 坑 2：user-data-dir 共享导致参数被已存在的 browser 进程忽略

退而求其次用 `tauri.conf.json` 的 `additionalBrowserArgs` 配 CDP，
但实测 debug 千寻启动后 CDP 仍不通。抓 `msedgewebview2.exe` 命令行：

```text
"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\153.0.4234.32\msedgewebview2.exe"
  --webview-exe-name=qianxun.exe --webview-exe-version=0.4.0     ← release 的 0.4.0
  --user-data-dir="...\com.qianxun.desktop\EBWebView"             ← 同一目录！
  --disable-features=...                                         ← 无 --remote-debugging-port
```

WebView2 行为：同一 `user-data-dir` 全机**只允许一个 browser 主进程**。
release 千寻先启动占住了 `com.qianxun.desktop\EBWebView` 目录；debug
千寻再去创建 webview 时，Edge runtime 把请求路由到**已存在的 browser
进程**——新进程的额外参数（CDP 端口、disable-features 等）一律被
丢弃，debug 自己的窗口加载 devUrl 走的是 release 的进程。

旁证：`e2e/_fixture.ts` 注释明确写「多 webview 共享 user data dir，
测试间必须隔离——官方文档要求」并用 `WEBVIEW2_USER_DATA_FOLDER` 给
e2e 一次性临时目录。e2e 的 identifier (`com.qianxun.e2e`) 与 release
不同，所以天然不在同一 user-data-dir —— 这也是 e2e fixture 用 env
设 CDP 一直能 work 的原因（路径错位、问题未暴露）。

**结论**：debug 必须用**独立 user-data-dir** 与 release 隔离。Tauri 2
的 conf 没暴露 `userDataFolder` 字段（搜 tauri-utils WindowConfig
无果），只能靠 env `WEBVIEW2_USER_DATA_FOLDER`。env 在本进程内通过
`std::env::set_var` 设即可——loader 读 GetEnvironmentVariable 在本
进程内，main() 起手的位置完全足够。

**项目内写法**（`src-tauri/src/main.rs`）：

```rust
#[cfg(all(windows, debug_assertions))]
{
    if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none() {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let mut dir = std::path::PathBuf::from(local);
            dir.push("com.qianxun.desktop");
            dir.push("EBWebView-dev");  // ← 与 release 的 EBWebView 区分
            std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir);
        }
    }
}
```

**release 跳过此段**（`#[cfg(debug_assertions)]` 守卫）——不在用户安装
版上动 user-data-dir 隔离、也不暴露调试端口。

### 2.3 坑 3（顺带的 bug）：serde 字段名错位导致磁盘扫描永久卡死

CDP 通了后跑探针发现：磁盘扫描永远卡「0 项 · 扫描中…」。先看 UI
现象（crumb 显示 `数据目录 — · 0 项 · 扫描中…`），初步怀疑 Svelte
5 反应式，深挖才知道是**后端契约问题**。

直接经 CDP 在 page 里动态 import Tauri Channel，手发一次扫描抓原始
Done 帧（关键取证）：

```json
{
  "type": "done",
  "cancelled": false,
  "files": 1, "dirs": 0, "skipped": 0,
  "tree": {"name": "logs", "size": 62211, "children": []},
  "largest_files": [...]   ← 【snake_case】
}
```

前端 contract 是 `largestFiles`（camelCase）。`largest` 字段不对。
同理 progress 帧是 `top_children`。

**根因**：`#[serde(rename_all = "camelCase")]` 用在 enum 上**只作用
variant 名**（所以 `type:"done"` 是小写、对应前端 `event.type ===
'progress'` 的判断条件 OK），**不作用字段名**。要字段也 camelCase
必须用 `rename_all_fields = "camelCase"`（serde ≥ 1.0.190）。

但为什么 UI 不只显示空 largest（最多 largest 不显示），而**整个扫描
永久卡死**？因为 `top_children` 在 progress 帧：

```ts
// liveGrow 第一行
const nextChildren = frame.topChildren.map(...)
//   ^^^^^^^^^^^^^^ undefined
//   → TypeError: Cannot read properties of undefined (reading 'map')
```

`onmessage` 内的异常会**中断 Tauri Channel 的消息泵**——后续帧
（包括 Done 帧）永远无法派发。scanning 恒 true，trail 恒为
`provisionalItem`（0 大小、0 子项）。

实测：单独扫小目录（无 progress 帧直接 Done）→一切正常；扫稍大
目录（产生 progress 帧）→ 第一个 progress 帧就炸泵，卡死。

**修复**（三层）：

1. `src-tauri/src/disk.rs` —— enum 加 `rename_all_fields`：

   ```rust
   #[derive(Serialize, Clone)]
   #[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
   pub enum DiskScanEvent { Progress { ..., top_children: Vec<...> }, Done { ..., largest_files: Vec<...> } }
   ```

2. `src-tauri/crates/fff-core/src/disk.rs` —— `PartialChild.is_dir`
   序列化为 `dir`（前端 `DiskPartialChild.dir`）。同形 bug：
   `rename_all` 会让 `is_dir` 变 `isDir`，与前端 `dir` 仍不匹配。

   ```rust
   #[serde(rename_all = "camelCase")]
   pub struct PartialChild { ..., #[serde(rename = "dir")] pub is_dir: bool }
   ```

3. `DiskScan.svelte` liveGrow 加契约防御（即使后端再漂移也不许
   中断消息泵）：

   ```ts
   if (!Array.isArray(frame.topChildren)) return;
   ```

### 2.4 坑 4：浅层模型的"防御性清空"过头

修了 2.3 后 Done 处理成功（breadcrumb 显示 `数据目录 1.1GB · 0
项 · 刚刚扫描`）——但**"0 项"**！body 显示"空目录"占位。

最初我改 `From<DiskSpaceEntry>` 为 `children: Vec::new()`（每层
都平），本意是防御 fff 未来又带回深树时多发数据。问题：**根层的
直接子项也一起被清了**——fff 的 `tree_root` 是浅层的（根带 children、
子项 children=[]），Tauri 侧不该把根的 children 也平。

```rust
impl From<fff_search::DiskSpaceEntry> for DiskEntry {
    fn from(value: fff_search::DiskSpaceEntry) -> Self {
        DiskEntry {
            ...
            children: value.children.into_iter().map(shallow_child).collect(),
        }
    }
}
/// 子项强制浅层：丢弃孙辈及以下（前端契约只认一层，drill 必新扫描）。
fn shallow_child(value: fff_search::DiskSpaceEntry) -> DiskEntry {
    DiskEntry { ..., children: Vec::new() }
}
```

## 3. 最终代码改动（4 处）

```text
src-tauri/src/main.rs                     WEBVIEW2_USER_DATA_FOLDER 隔离（debug only）
src-tauri/tauri.debug.conf.json           主窗 additionalBrowserArgs 带 --remote-debugging-port=10222
package.json                               dev script 加 --config src-tauri/tauri.debug.conf.json
src-tauri/src/disk.rs                     rename_all_fields + shallow_child 拆分
src-tauri/crates/fff-core/src/disk.rs     PartialChild.is_dir 序列化为 dir
src/features/search/disk/DiskScan.svelte liveGrow 契约防御
```

## 4. 验证流程（关键：抓原始帧，不要靠推断）

```text
1. 启动 pnpm dev（CDP 10222 起）
2. 探针 chromium.connectOverCDP('http://127.0.0.1:10222')
3. 拿主窗 page，导航到目标 tab
4. 如果怀疑 IPC 字段错位：
   - page.evaluate 里动态 import '/node_modules/.vite/deps/@tauri-apps_api_core.js'
   - new Channel() + invoke('disk_scan_stream', { path, onEvent })
   - onEvent.onmessage 里 JSON.stringify(ev) console.log
   - 立刻看到后端发什么、前端 contract 期望什么 → diff
5. 不要在浏览器侧 mock 后端 mock 整套——Tauri Channel 抛错中断消息泵
   这种问题只有真环境能复现
```

## 5. 选型说明

- **CDP 端口**：10222。`e2e/_fixture.ts` 用了同一个号（它也 debug 构建）。
  二者并存的关键是 user-data-dir 不同（fixture 自己设临时目录），
  dev 通过 `EBWebView-dev` 隔离。
- **不用 `--remote-debugging-pipe`**：Windows 上是 named pipe 端，Playwright
  `connectOverCDP` 仅支持 TCP。固定用端口。
- **不用 tauri-plugin-devtools 替代**：DevTools 是右键检查 + DOM 抓取，
  不能脚本化。CDP 才是真远程驱动。
- **不入 release 构建**：`#[cfg(debug_assertions)]` 把 user-data-folder
  隔离整段围住；`tauri.debug.conf.json` 不被 release 构建读取（dev
  script 才传 `--config`）。

## 6. 跨 debug/release 的 single_instance 互斥

千寻的 `src-tauri/src/single_instance.rs` 走的是 FNV(路径+identifier)
哈希互斥，**不同路径不互斥**——所以 `pnpm dev`（debug 二进制在
`target/debug/qianxun.exe`）与 release（`C:\Users\maxu\AppData\Local\
qianxun\qianxun.exe`）天然并存。这点与「共享 user-data-dir」是
正交问题——前者是 Rust 进程层互斥，后者是 WebView2 runtime 层。

文档里如看到「debug 与 release 不能同时跑」是错的，请以本文为准。

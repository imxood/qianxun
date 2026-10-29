# 侧边模块分析报告 · DSH（dsh）

> 分析日期：2026-10-05 ｜ 范围：前端 `src/features/dsh/DshPage.svelte`、`src/StandaloneWindow.svelte`、`src/lib/windowEnv.ts`、`src/lib/external-links.ts`、分离入口 `src/components/SideNav.svelte`、容器 `src/App.svelte`；后端 `src-tauri/src/window.rs`、`src-tauri/src/dsh_upstream.rs`、`src-tauri/src/remote/gateway.rs`（注入层）、注入资产 `remote/assets/qx-shell-links.js`
> 负责：worker / env-web-dsh-module
> 参考：`docs/06-Tauri窗口重建指南.md`、`docs/07-Tauri调试基建与CDP经验.md`、`docs/02-技术架构.md`

## 0. 速览

| 项         | 值                                                                                                                                                                                             |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 一句话定位 | DSH Host 宿主：把受监督的本地 DSH Web 服务通过**回环网关**装进一个 sandbox iframe，并提供外链桥（→内置浏览器窗）与右键分离到独立窗口——全应用唯一可分离的模块                                   |
| 完成度     | **86/100（良好）**                                                                                                                                                                             |
| 代码规模   | 前端 558 行（`DshPage` 197 + `StandaloneWindow` 102 + `external-links` 86 + `windowEnv` 13 + 分离入口 146 共享） / 后端 ~2053 行（`window.rs` 534 + `dsh_upstream.rs` 740 + `gateway.rs` 779） |
| 最严重问题 | 重复分离到独立窗口后，关闭其中一个会让主窗**重新挂载一个活的 DSH iframe**，而另一个窗口还开着——`nav.detached` 是布尔而非计数（见 §4-D1）                                                       |

**三个模块里本模块得分最高**：安全边界设计是全项目最扎实的一块（仅回环绑定、cookie 留服务端、双重来源校验、协议白名单、零权限浏览器窗），且有**真实 e2e 覆盖分离/关闭/ACL/最大化**四条路径（`e2e/standalone-window.spec.ts`）。扣分集中在窗口生命周期的一处布尔建模缺陷和 iframe 导航面的两个缺口。

---

## 1. 模块定位与设计意图

### 1.1 为什么要绕回环网关，而不是直连 DSH

这是本模块全部复杂度的根源，注释链完整记录了演进：

```
DSH 0.1.2 起自带浏览器鉴权
  └─ 就绪 URL 里的 token 只能 GET / 一次，兑换一张 HttpOnly; SameSite=Strict 的 cookie
       └─ 跨站 iframe（tauri.localhost 内嵌 127.0.0.1）永远不携带 Strict cookie
            └─ 直连 127.0.0.1:<dsh> 必 401 死循环
                 └─ 解法：iframe 走本机回环网关，cookie 由服务端持有
```

证据链：

- `src/features/dsh/DshPage.svelte:69-73`（前端注释，同源表述）
- `src-tauri/src/harness/commands.rs:70-74`（`harness_proxy_url` 的注释）
- `src-tauri/src/dsh_upstream.rs:1-12`（网关原语的模块文档）
- `docs/02-技术架构.md` §4.5（cookie 生命周期设计）

`dsh_upstream.rs:9-12` 讲得最透：「Strict cookie 在跨站 iframe 里永远不会被浏览器携带，浏览器侧持有毫无用处；服务端持有则彻底绕开 SameSite 语义。」——**这是把浏览器的安全语义搬到服务端解决的正确思路**，而不是去放宽 cookie 的 SameSite 属性。

### 1.2 三层结构

```
┌─ 主窗（capability: default.json, windows:["main"]）
│   App.svelte:89-92  ── DSH ready 即 nav.visit('dsh')，后台预挂 iframe
│   DshPage.svelte    ── iframe src = harness.proxyUrl（回环网关）
│       │  postMessage 桥（__qxShell: ready/arm/open-url）
│       │  window_spawn_view('dsh')  ← 右键
│       ▼
├─ 独立窗（capability: standalone-windows.json, windows:["standalone-*"]）
│   StandaloneWindow.svelte ── 自绘标题栏 + <DshPage standalone/>
│   （同一份 DshPage，独立 JS realm → 独立的 harness store 实例）
│
├─ 内置浏览器窗（**无任何 capability** → 零 IPC 权限）
│   window_open_external → browser / browser-{n}，系统 WebView2 装外部 URL
│   ← DSH 页外链普通点击（Ctrl → 系统浏览器；中键/Shift → 新开后台窗）
```

关键的设计取舍记录在 `window.rs:465-470`：「`browser*` label 不在任何 capability —— 远程页面零 IPC 权限，Tauri 调用一律默认拒绝」。**用「不给权限」而不是「运行时拦截」来隔离不可信网页**，这是最省事也最不容易漏的边界。我已验证这个说法属实：`default.json` 的 `windows` 只有 `["main"]`，`standalone-windows.json` 是 `["standalone-*"]`，其余两个文件是 `shot-*`——`browser*` 确实不在任何名单里。

### 1.3 为什么只有 DSH 可分离

`SideNav.svelte:65` 的 `DETACHABLE_VIEWS = new Set(['dsh'])` 与 Rust 的 `standalone_view_meta`（`window.rs:81-86`，只 match `"dsh"`）、前端的 `StandaloneView`（`windowEnv.ts:4`，只声明 `'dsh'`）**三处严格对应**。三处都加了注释互相指认（`SideNav.svelte:64`「与 Rust standalone_view_meta 对应」、`windowEnv.ts:3`「与 Rust 侧 standalone_view_meta 对应」），扩展时不会漏改。这在跨语言契约上很难得。

原因也说得通：DSH 页是唯一「重渲染 + 有状态 + 用户会长时间停留」的页面（`App.svelte:96-99` 注释：「DSH IDE 这类重渲染页切走后不再空烧渲染进程」），而其他页面切走用 `visibility: hidden` 成本可接受。**分离的真实动机是资源，不是布局。**

---

## 2. 功能清单与完成度

### 2.1 iframe 承载与生命周期

| #   | 功能                                | 状态   | 证据(file:line)          | 备注                                                                                   |
| --- | ----------------------------------- | ------ | ------------------------ | -------------------------------------------------------------------------------------- |
| 1   | iframe 承载 DSH 页面                | 已完成 | `DshPage.svelte:111-124` | `src = dshUrl`（代理地址，恒定）                                                       |
| 2   | sandbox 限制                        | 已完成 | `DshPage.svelte:115`     | `allow-scripts allow-same-origin allow-forms allow-downloads allow-popups`，见 §4-D3   |
| 3   | ready 门控（未就绪不建 iframe）     | 已完成 | `:68`                    | `phase === 'ready' && proxyUrl !== null`                                               |
| 4   | 代理地址竞态补偿                    | 已完成 | `:77-81`                 | ready 但 proxyUrl 为 null 时重拉；注释点明是「wire 与 setup 监听的启动竞态」           |
| 5   | 「已就绪但代理未监听」专页          | 已完成 | `:95-108`                | 不白屏，给「重试获取代理地址」按钮（**不静默回退直连**，`harness.svelte.ts:131` 注释） |
| 6   | 首屏防白屏浮层                      | 已完成 | `:84-88, 125-135`        | 文档 load 前盖主题色 + spinner；`dshUrl` 变化时重置 `frameLoaded`                      |
| 7   | 相位驱动的空态/失败/启动中          | 已完成 | `:137-209`               | stopped/starting/restarting/failed 四态各有专属 UI                                     |
| 8   | 失败原因展示                        | 已完成 | `:159-163`               | 等宽字体 + `break-all`，长错误可读                                                     |
| 9   | 启动 DSH 快捷按钮                   | 已完成 | `:193-201`               | **仅非独立窗显示**（standalone 时引导去主窗）                                          |
| 10  | 「环境与日志」跳转                  | 已完成 | `:202-206`               | 同样仅非独立窗                                                                         |
| 11  | 预热（后台提前挂载）                | 已完成 | `App.svelte:89-92`       | DSH ready 即 `nav.visit('dsh')`，第一次点击已是加载完成的页面                          |
| 12  | 切页用 display:none 而非 visibility | 已完成 | `App.svelte:96-100`      | 注释给出实测依据：「DSH IDE 这类重渲染页切走后不再空烧渲染进程」                       |
| 13  | iframe 自动重载                     | 已完成 | `:117-123`               | DSH 崩溃重启后地址不变（代理吸收 revive），iframe 无需重载                             |
| 14  | 独立窗内的降级文案                  | 已完成 | `:189`                   | 「DSH 未运行。请先在主窗口的「环境」页启动。」                                         |

### 2.2 外链桥（postMessage 协议）

| #   | 功能                                           | 状态   | 证据(file:line)                                          | 备注                                                                    |
| --- | ---------------------------------------------- | ------ | -------------------------------------------------------- | ----------------------------------------------------------------------- |
| 15  | 注入脚本下发（仅回环入口）                     | 已完成 | `gateway.rs:151-152, 436-447`                            | `/qx-shell/links.js` 路由 + `is_loopback` 门禁（LAN 设备拿不到）        |
| 16  | HTML 注入（仅 GET+200+text/html+有 `</head>`） | 已完成 | `gateway.rs:423-431, 449-470, 492-567`                   | ≤2MiB 缓冲；改写后丢弃 etag/last-modified                               |
| 17  | 双向握手（ready/arm）                          | 已完成 | `DshPage.svelte:33-42, 53-61`；`qx-shell-links.js:41-65` | 消除「谁先加载」时序问题                                                |
| 18  | 乒乓死循环修复                                 | 已完成 | `DshPage.svelte:53-56`；`qx-shell-links.js:53-59`        | **实测空转整机 10% CPU 的根因**，两侧都做了「只补一次」                 |
| 19  | 双重来源校验                                   | 已完成 | `DshPage.svelte:46-48`；`external-links.ts:56-61`        | `event.source === frame.contentWindow` + `event.origin === 网关 origin` |
| 20  | 协议白名单（仅 http/https）                    | 已完成 | `external-links.ts:71`                                   | `file:`/`javascript:` 拒绝，有单测（`.test.ts:55-58`）                  |
| 21  | URL 绝对化与合法性                             | 已完成 | `external-links.ts:65-71`                                | `new URL` 失败即 null                                                   |
| 22  | 修饰键严格布尔判定                             | 已完成 | `external-links.ts:74-76`                                | 只认 `=== true`，有单测（`.test.ts:60-68`）                             |
| 23  | 三路分流                                       | 已完成 | `external-links.ts:84-93`                                | 普通→内置窗；Ctrl/⌘→系统浏览器；中键/Shift→新开内置窗                   |
| 24  | 站内导航放行                                   | 已完成 | `qx-shell-links.js:78`                                   | `url.origin === location.origin` 不拦                                   |
| 25  | 未武装时零行为                                 | 已完成 | `qx-shell-links.js:68`                                   | 直接用浏览器打开网关页时完全原生行为                                    |
| 26  | 目标父页 origin 白名单                         | 有缺陷 | `qx-shell-links.js:23-28, 31-39`                         | 见 §4-D4                                                                |
| 27  | 协议单测                                       | 已完成 | `external-links.test.ts`                                 | 9 个用例，覆盖 origin 不匹配/协议/空网关/修饰键/缺字段                  |

### 2.3 窗口分离

| #   | 功能                                  | 状态       | 证据(file:line)                                   | 备注                                                                                |
| --- | ------------------------------------- | ---------- | ------------------------------------------------- | ----------------------------------------------------------------------------------- |
| 28  | 右键菜单项                            | 已完成     | `SideNav.svelte:77-85`                            | 仅 `DETACHABLE_VIEWS` 内的项才弹菜单（`:78` 提前 return）                           |
| 29  | tooltip 提示可分离                    | 已完成     | `SideNav.svelte:104`                              | `· 右键分离到独立窗口`                                                              |
| 30  | spawn 失败保持原状                    | 已完成     | `SideNav.svelte:68-75`                            | 先 `await window_spawn_view` 成功再 `nav.detach`，失败只 `console.error`            |
| 31  | 独立窗自绘标题栏                      | 已完成     | `StandaloneWindow.svelte:44-99`                   | 拖拽区（`data-tauri-drag-region="deep"`）+ 最小化/最大化/关闭                       |
| 32  | 隐藏创建 + 首帧亮出                   | 已完成     | `window.rs:428`；`StandaloneWindow.svelte:34-40`  | `visible(false)` → `settings.load().finally(show+setFocus)`                         |
| 33  | 主题同步（与主窗同一数据流）          | 已完成     | `StandaloneWindow.svelte:22-32`                   | 启动一次 + 每次保存后各一次；localStorage 回写                                      |
| 34  | 视图解析（URL hash）                  | 已完成     | `windowEnv.ts:13-16`                              | `#/standalone/dsh` 正则；未知视图显示「未知视图」（`StandaloneWindow.svelte:105`）  |
| 35  | 窗口 ACL                              | 已完成     | `capabilities/standalone-windows.json:6-14`       | 最小集：minimize/toggle-maximize/close/start-dragging/show/set-focus + core:default |
| 36  | 关闭广播 → 主窗恢复侧栏项             | 已完成     | `window.rs:582-586`；`App.svelte:49-51`           | `window://closed` 事件带 `{label, view}`                                            |
| 37  | 侧栏项隐藏/恢复                       | 已完成     | `SideNav.svelte:93`；`nav.svelte.ts:34-37, 46-48` | `items.filter(item => !nav.detached[item.id])`                                      |
| 38  | 级联定位                              | 已完成     | `window.rs:434-443`                               | 以主窗为锚，每次错开 32 逻辑像素，8 个一循环                                        |
| 39  | 主窗关闭时若有独立窗则隐藏到托盘      | 已完成     | `window.rs:151-167`                               | 「用户预期是关的是主窗，不是整个应用」                                              |
| 40  | 重复分离防护                          | **未实现** | `SideNav.svelte:68-75`                            | 见 §4-D1                                                                            |
| 41  | 独立窗几何记忆                        | **未实现** | `window.rs:120-132`                               | `capture()` 只取 label `"main"`，见 §4-D5                                           |
| 42  | 分离时 transfer 状态（不重载 iframe） | **未实现** | `App.svelte:113-117`                              | 见 §4-D6                                                                            |
| 43  | 独立窗内的 harness 状态跨窗同步       | 部分完成   | `StandaloneWindow.svelte:102-103`                 | 独立 JS realm 有自己的 store，靠事件流同步（见 §3.3）                               |

### 2.4 内置浏览器窗（外链的落地形态）

| #   | 功能                                   | 状态   | 证据(file:line)          | 备注                                                                            |
| --- | -------------------------------------- | ------ | ------------------------ | ------------------------------------------------------------------------------- |
| 44  | 协议白名单（Rust 侧二次校验）          | 已完成 | `window.rs:477-483`      | 只放行 http/https，与前端校验形成双保险                                         |
| 45  | 单例复用（普通点击不弹新窗）           | 已完成 | `window.rs:486-496`      | 复用 `browser` 窗并 `navigate` + 前置                                           |
| 46  | 多开级联（`browser-{n}`）              | 已完成 | `window.rs:498-522`      | 中键/Shift 不抢焦点（`:510` `focused(!new_window)`）                            |
| 47  | load 事件亮窗                          | 已完成 | `window.rs:524-533`      | 错误页也走 load，不会永远隐身                                                   |
| 48  | 8s 兜底亮窗                            | 已完成 | `window.rs:541-552`      | 「宁可白屏也不让窗口隐身」                                                      |
| 49  | 零 IPC 权限                            | 已完成 | `window.rs:465-468`      | `browser*` 不在任何 capability（已验证）                                        |
| 50  | 后台导航（`eval` 不用，用 `navigate`） | 已完成 | `window.rs:463-465, 488` | 「远程页没有外壳前端，事件线不可用；navigate 走 WebView2 导航，比 eval 更正式」 |

---

## 3. 实现分析

### 3.1 架构与数据流

**主路径（DSH 页面加载）**

```
Rust: supervisor 就绪 → Status::Ready{origin, url, pid}
        │  event harness://event
        ▼
harness.svelte.ts:79-92  ingest() → status 镜像
        │
        ├─ DshPage:68  ready = phase==='ready' && proxyUrl !== null
        │      └─ :114  <iframe src={proxyUrl}>  sandbox=…
        │
        ▼  HTTP GET http://127.0.0.1:<gateway>/
remote/gateway.rs:300-308  回环分支 → forward → inject_layer_into(DESKTOP_LAYER)
        │  ① 缓冲 ≤2MiB，② 剥逐跳头，③ </head> 前插 <script src="/qx-shell/links.js" defer>
        ▼  浏览器带上服务端 cookie 请求 DSH
dsh_upstream.rs:205-337  forward()：换 cookie、改写 Origin/Sec-Fetch 为同源、401 时重兑一次
        ▼
DSH webServer ── 页面 + WS(/api/events.mux) + SSE(/plugins/events)
```

**外链路径**

```
用户在 DSH 聊天里点链接
   ▼  qx-shell-links.js:98-116（捕获阶段，click + auxclick）
   armed? 否 → 放行（qx-shell-links.js:68）
   站内? 是 → 放行（:78）
   postMessage{__qxShell, kind:'open-url', url, ctrl, middle, shift} → 4 个父 origin（:31-39）
   ▼
DshPage.svelte:45-62  onMessage
   event.source === frame.contentWindow?        （:46）否则 return
   parseShellLink(origin, data, gatewayOrigin)   （:48）origin 不符 → null
   ▼
external-links.ts:84-93  openShellLink
   ctrl → openUrl(url)（系统浏览器，opener ACL 仅 http/https）
   否则 → call('window_open_external', {url, newWindow: middle||shift})
   ▼
window.rs:471-554  新建/复用内置浏览器窗（系统 WebView2 装外部 URL，零 IPC 权限）
```

### 3.2 前后端契约

| 契约                                  | 载体                                                         | 门禁                                                                                                                               |
| ------------------------------------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `window_spawn_view(view) → label`     | `contract.ts:56`                                             | ✅ `contract.test.ts` 锁命令名                                                                                                     |
| `StandaloneClosedEvent {label, view}` | `contract.ts` + `window.rs:330-335`                          | ✅ serde `rename_all = "camelCase"`                                                                                                |
| `StandaloneView = 'dsh'`              | `windowEnv.ts:4`                                             | ⚠️ **手工对应**，无自动校验（见 §4-D7）                                                                                            |
| postMessage 协议 `__qxShell` / `kind` | `external-links.ts:19`（TAG）+ `qx-shell-links.js:21`（TAG） | ⚠️ 两侧各写一份字符串常量，靠注释互指（`external-links.ts:9-11` 点名资产路径、`qx-shell-links.js:9` 点名 TS 文件）。无生成、无测试 |
| `origin` 推导                         | `DshPage.svelte:24-31`                                       | ✅ 从 `harness.proxyUrl` 运行时算出，不硬编码                                                                                      |

**postMessage 协议 TAG 重复是最脆弱的一处跨语言契约**。它和 `StandaloneView` 一样是「两份定义 + 注释互指」。当前任一侧改 TAG 会静默失效（外链点了没反应，无报错）。`external-links.test.ts` 只测 TS 侧解析，用的是**测试里手写的消息字面量**（`external-links.test.ts:7-17`），并不读真实的 `qx-shell-links.js`——所以 TAG 漂移不会被任何测试发现。

### 3.3 资源与生命周期

**窗口层**

| 资源            | 归属                                    | 释放                                       | 评价                                                  |
| --------------- | --------------------------------------- | ------------------------------------------ | ----------------------------------------------------- |
| 独立窗 WebView2 | `WebviewWindowBuilder`                  | Destroyed → `on_standalone_destroyed` 广播 | ✅ 无泄漏                                             |
| 独立窗 label    | `NEXT_STANDALONE: AtomicU64`            | 进程退出即消                               | ⚠️ 永不复用，见 §4-D5                                 |
| 主窗几何        | `settings.window.geometry`              | 关闭/退出时一次性快照                      | ✅ 变更才写盘（`window.rs:183-188`）                  |
| 独立窗几何      | **无**                                  | —                                          | ❌ 见 §4-D5                                           |
| browser 窗      | `app.get_webview_window(BROWSER_LABEL)` | 进程退出                                   | ⚠️ 单例窗无生命周期管理（无「关掉所有浏览器窗」路径） |

**iframe 层**

| 资源                 | 生命周期                                 | 评价                                              |
| -------------------- | ---------------------------------------- | ------------------------------------------------- |
| DSH iframe（主窗）   | `nav.visited.dsh && !nav.detached.dsh`   | ⚠️ 分离时**被销毁**，不是隐藏（`App.svelte:113`） |
| DSH iframe（独立窗） | 独立窗存活期                             | ✅                                                |
| 预热                 | DSH ready 即挂载                         | ✅ 用户零等待                                     |
| 切页                 | `display:none`（非 `visibility:hidden`） | ✅ 有实测依据的优化                               |

**跨窗状态同步**：`harness` 是模块级单例，但**独立窗是不同的 JS realm**，`StandaloneWindow.svelte:102-103` 渲染的 `<DshPage standalone>` 在该窗内 `onMount` 调 `harness.wire()`（`DshPage.svelte:10-12`）→ 独立窗有自己的 store 实例，通过 `harness://event` 事件流与 Rust 同步。**这个设计是正确的**（不能用 `localStorage`/单例跨 realm），但意味着分离后**两侧各有一条独立的 WS/SSE 连接**到 DSH（§4-D8）。

`StandaloneWindow.svelte` 未调用 `harness.dispose()`（对比 `App.svelte:52-56` 的对称清理）。当前无害——窗口销毁时整个 JS realm 消亡，监听随之消失（§4-D9）。

### 3.4 错误处理与恢复

| 失败点                   | 表现                                                                                   | 评价                                          |
| ------------------------ | -------------------------------------------------------------------------------------- | --------------------------------------------- |
| DSH 停止应答（哑掉）     | 健康看门狗 kill → 退避重启；**iframe 地址不变，代理热吸收 revive**                     | ✅ 优秀：用户无感（`DshPage.svelte:67` 注释） |
| DSH 崩溃重启             | 同上，代理端口恒定                                                                     | ✅                                            |
| 启动失败                 | 失败卡片 + 错误全文 + 「环境与日志」跳转                                               | ✅                                            |
| 代理未监听               | 专页 + 重试按钮                                                                        | ✅                                            |
| `window_spawn_view` 失败 | 主窗保持原状，只 `console.error`                                                       | ⚠️ **用户完全无感知**（见 §4-D10）            |
| 独立窗主题未加载         | `settings.load().finally(show)` — 用 `finally` 保证失败也亮窗                          | ✅ fail-visible                               |
| 未知独立窗视图           | 显示「未知视图」                                                                       | ✅                                            |
| 未知 postMessage         | 静默忽略（`external-links.ts:61-62`）                                                  | ✅ 符合安全预期                               |
| 浏览器窗建不出来         | 返回 `Err`，`external-links.ts:92` 向上抛，`DshPage.svelte:50` `.catch(() => {})` 吞掉 | ⚠️ 用户无感知（同 §4-D10）                    |

**一个值得称道的细节**：`StandaloneWindow.svelte:36-39` 用 `.finally()` 而不是 `.then()` 来触发亮窗——`settings.load()` 失败时窗口仍会显示，用户看到的是「设置没加载出来」而不是「点了没反应」。这与 `App.svelte:34-44` 的 fail-visible 策略一致。

---

## 4. 缺陷清单

| 级别   | 问题                                                                                                          | 位置                                                                                  | 影响                                                                                                                                                                                                                                                                                                                                                                                                                                               | 证据                                                                                                                                                                                                                                                      |
| ------ | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **P1** | **重复分离导致双活 iframe + 侧栏状态说谎**。`nav.detached` 是布尔而非计数，`detachToWindow` 无 in-flight 防护 | `SideNav.svelte:68-75`；`nav.svelte.ts:16, 34-37, 46-48`；`App.svelte:49-51, 113-117` | 快速右键两次 → 依次 spawn `standalone-dsh-1`、`standalone-dsh-2`，主窗因 `nav.detach` 幂等而停止渲染 DshPage。关闭**其中一个** → `window://closed` → `nav.reattach('dsh')` 删除布尔 → 主窗**重新挂载一个活的 DSH iframe**，而另一个独立窗仍开着。结果：2 个活 iframe、2 条 WS/SSE 连接、侧栏项重新出现但实际还有一个窗口在跑                                                                                                                       | `nav.svelte.ts` 的 `detached: Partial<Record<PageId, boolean>>`（`:16`）；`reattach` 无条件 `delete this.detached[page]`（`:47`）；Rust 侧其实**已有**判断所有独立窗是否存在的代码（`window.rs:154-157`）但 `on_standalone_destroyed`（`:583-586`）没用它 |
| **P1** | **iframe 内无顶层导航防护，非 `<a>` 导航不被拦截**。注入脚本只拦 `click`/`auxclick` 事件上的 `<a href>`       | `qx-shell-links.js:98-116, 67-89`；`DshPage.svelte:115`                               | `location.href='https://…'`、`location.assign`、表单提交、`target="_top"` 都不经过点击处理器 → iframe 可在 DSH 面板内加载任意外部页面，且此时 sandbox 的 `allow-scripts allow-same-origin allow-downloads` 全部生效。sandbox 无 `allow-top-navigation` 阻止了劫持顶层窗口（这是唯一的好消息），但 iframe 区域内可被任意网页占据（视觉冒充 DSH）并静默触发下载落盘。**风险面**：DSH 聊天里渲染的不可信内容若含诱导 JS（取决于 DSH 自身的 XSS 防护） | `qx-shell-links.js:98-116`（仅两个事件监听）；`DshPage.svelte:115`（sandbox 列表）                                                                                                                                                                        |
| **P2** | `allow-popups` 授权无实际用途且语义混乱                                                                       | `DshPage.svelte:115`                                                                  | 注释自认 wry 未注册 `NewWindowRequested`，弹窗被静默吞掉（这正是外链桥存在的原因），那 `allow-popups` 就是纯冗余授权。若将来 wry 行为变化，弹窗会被创建成 `browser*` 类的**零权限窗口**——安全上没问题，但与预期不符且难排查。建议直接移除该 token                                                                                                                                                                                                  | `DshPage.svelte:115` vs `external-links.ts:8-10`                                                                                                                                                                                                          |
| **P2** | `PARENT_ORIGINS` 硬编码 4 个 origin（含 dev vite 端口 5190），且 `post()` 向**全部 4 个** origin 广播         | `qx-shell-links.js:23-28, 31-39`                                                      | ① 与 `vite.config.ts` 的端口配置重复，dev 端口一改外链桥静默失效（链接点了没反应，无任何日志）；② dev 下用户本地任何跑在 5190/127.0.0.1:5190 的页面都会收到用户点击的 URL 列表（同机信息泄露，P3 级）                                                                                                                                                                                                                                              | `qx-shell-links.js:23-28, 31-39`                                                                                                                                                                                                                          |
| **P2** | **独立窗无几何记忆**。`persist_geometry`/`capture` 只快照 label `"main"`                                      | `window.rs:120-132, 171-189`                                                          | 每次分离都开在固定 1040×740（`window.rs:83`）+ 级联偏移。用户调好尺寸后关闭重开全部丢失。`NEXT_STANDALONE` 单调递增不重置，级联偏移在长会话中持续漂移（`(n-1)%8`）                                                                                                                                                                                                                                                                                 | `window.rs:81-86, 120-132, 434-443`                                                                                                                                                                                                                       |
| **P2** | 分离时不 transfer 已有 iframe 状态；分离前主窗的 DSH iframe 被**销毁**                                        | `App.svelte:113-117`                                                                  | 分离瞬间 DSH 页面重新加载（独立窗是新 webview）；关闭独立窗再点回 DSH 又加载一次。DSH SPA 内的会话/UI 状态（滚动位置、未发送的输入、侧边面板展开态）丢失。对「把 DSH 挪到大屏长期用」的预期是体验断点                                                                                                                                                                                                                                              | `App.svelte:113`（`!nav.detached.dsh` 条件卸载）                                                                                                                                                                                                          |
| **P2** | `window_spawn_view` 失败对用户零反馈                                                                          | `SideNav.svelte:72-74`                                                                | spawn 失败只 `console.error`，用户右键点了菜单项后**什么都没发生**（侧栏项还在，没有 toast、没有错误）。项目已有 `stores/toast.svelte.ts` 可用                                                                                                                                                                                                                                                                                                     | `SideNav.svelte:72-74`                                                                                                                                                                                                                                    |
| **P2** | 分离期间主窗侧栏/页面状态不同步                                                                               | `nav.svelte.ts:34-37`                                                                 | `detach()` 把 `nav.page` 强制切到 `'env'`，但**没有记录分离前用户在看哪一页**。关闭独立窗后用户被丢回环境页，而非分离前的页面                                                                                                                                                                                                                                                                                                                      | `nav.svelte.ts:34-37`                                                                                                                                                                                                                                     |
| **P3** | postMessage 协议 TAG 与 `StandaloneView` 是「两份定义 + 注释互指」，无生成无测试                              | `external-links.ts:19` ↔ `qx-shell-links.js:21`；`windowEnv.ts:4` ↔ `window.rs:81-86` | 任一侧改名静默失效。`external-links.test.ts` 用测试内手写字面量（`:7-17`），**不读真实注入脚本**，所以 TAG 漂移不会被任何测试发现                                                                                                                                                                                                                                                                                                                  |
| **P3** | 分离后主窗与独立窗各持一条 DSH WS/SSE 连接                                                                    | `App.svelte:113-117`；`StandaloneWindow.svelte:102-103`                               | 两侧 iframe 都是活的（§4-D1 场景下 3 个），各自一条长连接。正常单分离场景下主窗 iframe 已卸载，只剩一条——**仅在 §4-D1 触发时**成为问题                                                                                                                                                                                                                                                                                                             |
| **P3** | `StandaloneWindow.svelte` 未调 `harness.dispose()`                                                            | `StandaloneWindow.svelte`（无）vs `App.svelte:52-56`                                  | 当前无害（窗口销毁 = realm 消亡），但与主窗的清理风格不对称                                                                                                                                                                                                                                                                                                                                                                                        |
| **P3** | `NEXT_STANDALONE` 永不复用；`standalone_view_of_label` 用 `rsplit_once('-')`                                  | `window.rs:25, 89-93`                                                                 | 当前只有 `dsh` 一个 view，`standalone-dsh-5` → `("dsh","5")` 解析正确。**未来若新增带连字符的 view 名**（如 `2dsh`）会误解析。潜在，非当前缺陷                                                                                                                                                                                                                                                                                                     |
| **P3** | iframe 未设 `referrerpolicy`                                                                                  | `DshPage.svelte:111-124`                                                              | DSH 页面发出的请求会带 referrer。默认 `strict-origin-when-cross-origin` 会向 DSH 上游暴露 `http://tauri.localhost/`，属信息泄露面（轻微，因 DSH 在回环）                                                                                                                                                                                                                                                                                           |

---

## 5. 改善建议

按 收益/成本 排序。

| 优先级 | 建议                                                                                                                                                                                                                                                                  | 改动文件                                                                                                                                           | 收益                                                    | 成本                                                                               |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| **1**  | **修 §4-D1 的重复分离**。最小改法：`on_standalone_destroyed` 里先查是否还有其它 `standalone-*` 窗口（`window.rs:154-157` 已有同样写法），有则**不广播** `window://closed`；或让 `nav.detached` 存 label 数组而非布尔。同时给 `detachToWindow` 加 in-flight 标志防双击 | `src-tauri/src/window.rs:582-586`（首选，一处改动堵住）+ `src/components/SideNav.svelte:68-75`（防双击）                                           | 高——消除唯一的 P1 功能正确性缺陷，Rust 侧一行查询即可   | **极低**（复用已有的 `webview_windows()` 遍历模式）                                |
| **2**  | **iframe 导航收敛**。三条按成本递增：① 移除 `allow-popups`（顺带解 §4-D2 的 P2）；② 注入脚本在捕获阶段额外拦 `submit` 与 `beforeunload`，非站内 origin 交给外壳；③ 在 wry 侧注册 `on_navigation` 回调，非网关 origin 一律拒绝（最彻底，需确认 wry 2 的 API 可用性）   | `src/features/dsh/DshPage.svelte:115`（①，1 token）；`remote/assets/qx-shell-links.js`（②）；`src-tauri/src/dsh_upstream.rs` 或 window 构建处（③） | 高——关掉「DSH 面板内可加载任意外部页面 + 静默下载」的面 | ① 极低；② 低；③ 中（需验证 wry API）                                               |
| 3      | 分离失败给 toast 提示                                                                                                                                                                                                                                                 | `src/components/SideNav.svelte:72-74`（改用 `stores/toast.svelte.ts`）                                                                             | 中——消除「点了没反应」                                  | 极低                                                                               |
| 4      | 记录分离前的 `nav.page`，reattach 时恢复                                                                                                                                                                                                                              | `src/stores/nav.svelte.ts:34-37` + `App.svelte:49-51`                                                                                              | 中——关闭独立窗后回到原页面而非环境页                    | 低                                                                                 |
| 5      | 独立窗几何记忆（按 view 分别存一份）                                                                                                                                                                                                                                  | `src-tauri/src/window.rs:81-86, 120-132, 418-451`；`settings.rs` 的 `window` 段                                                                    | 中——符合「独立窗口也是窗口」的直觉                      | 低-中（要动 settings schema）                                                      |
| 6      | 协议 TAG 单一事实源：把 `__qxShell` 与 `StandaloneView` 提到一个共享常量文件，Rust 侧在编译期校验 view 名单与前端一致；`external-links.test.ts` 改为**读取真实的 `qx-shell-links.js`** 断言 TAG 一致                                                                  | `src/lib/external-links.ts:19`、`src/lib/windowEnv.ts:4`、`src-tauri/src/remote/assets/qx-shell-links.js:21`、`src/lib/external-links.test.ts`     | 中——把两处静默失效风险变成 CI 可见                      | 低（后两项有现成模式可抄：`contract.test.ts` 的命令名比对）                        |
| 7      | `PARENT_ORIGINS` 去掉硬编码 dev 端口（构建期注入，或加注释指向 `vite.config.ts`）；`post()` 只向 `document.referrer` 的 origin 投递而非广播 4 个                                                                                                                      | `remote/assets/qx-shell-links.js:23-39`                                                                                                            | 中——消除跨文件隐式耦合与同机 URL 泄露                   | 低                                                                                 |
| 8      | 分离时把 DSH 页的可见状态（如滚动位置）transfer 到独立窗，或改为**不卸载主窗 iframe** 而是让独立窗复用同一 webview（Tauri 2 不支持跨窗转移，只能退化为「保留主窗 iframe」并接受双连接）                                                                               | `App.svelte:113-117`                                                                                                                               | 低-中——改善分离/回迁的连续性                            | **高**（Tauri 窗口模型限制，可能做不到干净方案。建议先只做「记录并恢复滚动位置」） |
| 9      | iframe 加 `referrerpolicy="no-referrer"`                                                                                                                                                                                                                              | `src/features/dsh/DshPage.svelte:111-116`                                                                                                          | 低——消除 referrer 泄露面                                | 极低                                                                               |
| 10     | `StandaloneWindow.svelte` 补 `harness.dispose()`；`NEXT_STANDALONE` 与 `standalone_view_of_label` 加 view 名不含连字符的断言                                                                                                                                          | `src/StandaloneWindow.svelte`；`src-tauri/src/window.rs:25, 89-93`                                                                                 | 低——对称性与防御性                                      | 极低                                                                               |

---

## 6. 完成度评估

| 维度              | 得分   | 依据                                                                                                                                                                                                                                                                                                                                                                            |
| ----------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 功能完整性        | 84     | 承载/门控/防白屏/四态/外链三路分流/分离/关闭恢复/内置浏览器窗全部实现。缺：分离时的状态 transfer、独立窗几何记忆、iframe 内导航收敛                                                                                                                                                                                                                                             |
| 健壮性 / 错误恢复 | 90     | iframe 地址恒定 → DSH 崩溃重启用户无感；代理缺失有专页；未知消息静默忽略；`finally` 保证 fail-visible；握手乒乓有实测级修复。扣分在 spawn 失败与浏览器窗失败都零反馈                                                                                                                                                                                                            |
| 性能与资源        | 85     | 预热、display:none 优化、隐藏创建、后台窗不抢焦点、8s 兜底亮窗都到位。扣分在重复分离时的多 iframe/多连接（§4-D8）与分离时的重复加载                                                                                                                                                                                                                                             |
| 测试覆盖          | 84     | **有真实 e2e**：`e2e/standalone-window.spec.ts` 4 个用例覆盖分离/关闭回流/ACL/双击最大化（用 CDP 连 debug 二进制）；`external-links.test.ts` 9 个协议用例；`dsh_upstream.rs` 9 个 + `gateway.rs` 十余个注入/鉴权用例；`window.rs` 的端口校验有单测。扣分：**协议 TAG 漂移测不出来**（§4-D9），且 `DshPage.svelte` / `StandaloneWindow.svelte` / `SideNav.svelte` 三个组件零单测 |
| UX                | 82     | 四态各有专属 UI、失败原因全文可读、代理缺失给重试、0 白屏设计贯穿。扣分在分离失败无反馈、分离后强制跳回环境页、iframe 可被外部页面占据                                                                                                                                                                                                                                          |
| 可维护性          | 88     | 三处 `StandaloneView` 对应关系有互指注释；注入/转发/隔离三层职责清晰；每个非显然决策都写了「为什么」（10% CPU 乒乓、WebView2 弹窗吞掉、navigate vs eval、严格 cookie、bind 前 destroy 死锁…）。扣分在两处跨语言协议重复定义                                                                                                                                                     |
| **加权总分**      | **86** | 评级 **A- / 良好**                                                                                                                                                                                                                                                                                                                                                              |

**为什么是三者最高**：这个模块的设计密度是全项目最高的——每一条非显然的实现几乎都能找到「这是为了解决什么实测问题」的注释（`docs/07-Tauri调试基建与CDP经验.md` 的经验直接固化进了代码）。安全边界是「结构性隔离」而非「运行时检查」：回环绑定、服务端 cookie、双重 origin 校验、协议白名单、零权限浏览器窗——五层都做了，且都能在代码里指出具体行号。

**扣分集中在一处**：窗口生命周期用「布尔」建模而不是「窗口集合」，这一个建模选择就同时产生了 P1-D1（双活 iframe）和 P2（状态不同步）两个问题。修法也很小（Rust 侧已有现成遍历写法）。

---

## 7. 测试与验证

**实际执行**（工作目录 `E:\develop\dsh-workspace\qianxun`）：

| 命令                                                                                      | 结果                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `npx vitest run src/lib/external-links.test.ts src/stores/web.svelte.test.ts src/lib/ipc` | **3 files / 28 tests 全部 passed**（384ms）。本模块直接命中 `external-links.test.ts`（9 个协议用例：ready 识别/URL 绝对化/相对地址拒绝/origin 不匹配/网关未知/非 http 协议/修饰键布尔/url 类型）与 `src/lib/ipc/contract.test.ts`（`window_spawn_view`、`window_open_external` 命令名与 Rust 源码比对）                                                                                                                                                                                   |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib`（全量）                           | **167 passed / 0 failed / 5 ignored**（2.05s）。本模块直接命中的：`dsh_upstream::tests::*`（9 个：origin 归一化、query_param 解码、缓存命中不兑换、`set_ready` 作废旧 cookie、未就绪返回 None、host/cookie 不透传、**调用方 iframe 真实 origin 被改写为上游 authority**、调用方未带 origin 时合成同源 origin）、`remote::gateway::tests::*`（注入四态、桌面层注入/不注入、**回环入口注入且脚本仅回环可取的端到端 mock 上游测试**）、`harness::supervisor::tests::*`（含启动命令参数次序） |
| 能力文件核查                                                                              | `src-tauri/capabilities/` 共 3 个文件：`default.json`（`windows:["main"]`）、`standalone-windows.json`（`windows:["standalone-*"]`，7 项最小权限）、`shot-windows.json`。**`browser*` 确实不在任何名单**——验证了 `window.rs:465-468` 的「零 IPC 权限」声明                                                                                                                                                                                                                                |
| CSP 核查                                                                                  | `src-tauri/tauri.conf.json`：`frame-src http://127.0.0.1:* http://localhost:*`（放行 DSH iframe）、`connect-src ... http://127.0.0.1:* http://localhost:*`（放行网关请求）。iframe 用回环地址是**必需的**，非配置疏漏                                                                                                                                                                                                                                                                     |
| 静态核查                                                                                  | `src/` 与 `src-tauri/src/` 中 `terminal` 命中 **0**（据此确认 `window.rs:2` 与 `capabilities/standalone-windows.json:4` 的「终端」字样是 v0.5 移除后的遗留注释）；`DETACHABLE_VIEWS` 确认只含 `dsh`（`SideNav.svelte:65`）；`standalone_view_meta` 确认只 match `"dsh"`（`window.rs:81-86`）                                                                                                                                                                                              |
| 资产漂移核查                                                                              | 7 个 websearch 资产 SHA-256 全一致（与 `02-联网-web.md` §7 同一批检查，此处仅作交叉确认）                                                                                                                                                                                                                                                                                                                                                                                                 |

**未执行**（如实标注）：

- **`pnpm e2e` / `pnpm e2e:build` 未运行**。`e2e/standalone-window.spec.ts` 的 4 个用例（分离/关闭回流/ACL 授权/双击最大化）**本次全部未执行**——它们需要先 `tauri build --debug` 产出二进制。**因此 §2.3 的分离/关闭/ACL/最大化四项功能的运行时行为未经本次验证**，只有代码与 fixture 层面的依据（已读 `standalone-window.spec.ts` 全文与 `_fixture.ts` 的存在）。这是本报告最大的验证缺口。
- 未做真机 DSH 联调：没有运行中的 DSH 实例，因此 **iframe 实际加载、外链桥握手与分流、DSH 崩溃后代理热吸收、代理未监听专页**这四条主路径均未在运行时观察。§3.1 的数据流是**从代码与注释重建的**，不是实测的。
- 未运行 `pnpm e2e:doctor`（CDP 环境自检）、`npx tsx e2e/probe-env-page.ts` 等手写探针。
- 未运行 `pnpm check`（prettier + eslint + svelte-check）全量；未运行 `cargo clippy` / `cargo fmt --check`。
- **未逐行审读**：`src-tauri/src/dsh_upstream.rs`（740 行，只读了模块头、关键常量、`forward`/`relay` 签名与 `#[cfg(test)]` 名单）、`src-tauri/src/remote/gateway.rs`（779 行，只读了路由表、注入函数族与测试名单）、`src-tauri/src/window.rs` 的 `apply_webview_preferences`（`:344-387`，WebView2 COM 配色方案）逐行细节、`.tmp`/CDP 相关经验文档 `docs/06` 与 `docs/07` 的正文（仅按任务清单列为参考，未通读）。
- 因此 §4-D2（iframe 导航缺口）与 §4-D4（`PARENT_ORIGINS`）的**实际可利用性未经运行时确认**——它们是从代码逻辑推出的结论，标注为 P1/P2 是基于「若 DSH 自身存在 XSS 则可被利用」的假设，未验证 DSH 是否会渲染可执行脚本。

**补记（本报告自身格式校验）**：`npx prettier --check` 对本文件**初次未通过**（表格列宽对齐问题），已执行 `npx prettier --write` 仅格式化本文件后复检通过 → `All matched files use Prettier code style!`。写入范围仅限本文件，未触碰同目录其他 worker 的报告。

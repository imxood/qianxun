# 侧边模块分析报告 · 联网（web）

> 分析日期：2026-10-05 ｜ 范围：前端 `src/features/web/WebPage.svelte`、`src/stores/web.svelte.ts`、`src/lib/external-links.ts` + `.test.ts`、`src/lib/windowEnv.ts`；后端 `src-tauri/src/websearch/**`、`src-tauri/src/window.rs`（`window_open_external`）；独立包 `packages/websearch/**`
> 负责：worker / env-web-dsh-module

## 0. 速览

| 项         | 值                                                                                                                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 一句话定位 | 一个搜索框 + 一张结构化结果列表：前端直连本机回环网关的 `/qx/web/search`，真正的多 provider 路由 / 反爬对抗 / 冷却 / 体检全在 DSH 插件 `qx-websearch`（独立 pnpm 包）里                    |
| 完成度     | **78/100（良好，但功能面远小于菜单标签暗示）**                                                                                                                                             |
| 代码规模   | 前端 321 行（含 63 行测试） / 千寻后端 440 行（`websearch/commands.rs`，只做落盘与 patch 维护） / 独立包 23 个源文件 ~145KB（含 24 个测试文件 ~220KB，466 个用例）                         |
| 最严重问题 | `packages/websearch` 的构建产物要靠**手工** `pnpm sync:websearch` 拷进 `src-tauri/src/websearch/assets/`，没有任何门禁校验两份一致——漏同步就等于给所有新用户发一个旧版搜索插件（见 §4-W1） |

### ⚠️ 首要结论：这不是 iframe / WebView 壳

任务简报问「这是否只是一个 iframe/WebView 壳」。**不是**，但简报把两个不同模块的职责混在了一起，需要澄清：

| 问题                              | 事实                                                                                                                                                     | 证据                                                                            |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `WebPage.svelte` 是 iframe 壳吗？ | **不是**。它是 122 行的纯文本结果列表，全文无 `<iframe>`。搜索结果以标题/链接/摘要的 `<ol>` 渲染，Markdown 汇总放可复制 `<pre>`                          | `WebPage.svelte:99-127`；模块注释明确「不把未信任内容注入 innerHTML」（`:7-9`） |
| 千寻的「网页浏览」能力在哪？      | 在 **DSH 模块**：DSH 页的 iframe + 外链桥把链接送进 `window_open_external`，开出一个**内置浏览器窗**（系统 WebView2 装外部 URL）。那才是「千寻即浏览器」 | `src-tauri/src/window.rs:461-554`；`src/lib/external-links.ts:5-16`             |
| 联网页点结果标题去哪？            | **系统浏览器**（`@tauri-apps/plugin-opener` 的 `openUrl`），不是内置窗                                                                                   | `WebPage.svelte:2, 29-31`                                                       |
| 搜索本身需要 iframe 吗？          | 不需要。搜索是 `fetch()` POST 到本机网关的普通 HTTP 请求，与 iframe 无关                                                                                 | `src/stores/web.svelte.ts:75-84`                                                |

所以本模块的功能真实边界是：**「结构化网页搜索结果的展示壳」**——一个搜索框、引擎下拉、结果列表、Markdown 汇总。菜单标签叫「联网」，但页面上没有「浏览」，只有「搜索」。这个落差是本报告 §4-W5 的核心观察。

---

## 1. 模块定位与设计意图

### 1.1 数据源：不是 Rust IPC，而是 HTTP

这是本模块最重要的架构事实。`web.search()` 走的是 `fetch` 而非 `call()`：

```ts
// src/stores/web.svelte.ts:75-84
const response = await fetch(base + '/qx/web/search', {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ query, engine: this.engineId || undefined, limit: 10 }),
  signal: AbortSignal.timeout(SEARCH_TIMEOUT_MS),
});
```

`base` 是 `harness.proxyUrl`（本机回环网关，`web.svelte.ts:45, 49`），请求打到 DSH webServer 上桥挂载的 `/qx/web/search` 端点（store 头注释 `web.svelte.ts:5-7`）。因此：

- **可用性完全依赖 DSH 运行**：`requireBase()`（`:44-50`）在 phase 非 `ready` 或 proxyUrl 为 null 时直接抛错，UI 显示「DSH 未运行，请到「环境」页启动后再联网搜索」+ 一个跳转按钮（`WebPage.svelte:69-79`）。**这是本模块做得最好的 UX 决策**——不白屏、给去向。
- **CSP 已放行**：`src-tauri/tauri.conf.json` 的 `connect-src` 含 `http://127.0.0.1:*`。
- **鉴权在服务端**：DSH 0.1.2 的 cookie 是 `SameSite=Strict`，跨站 iframe 不可携带，所以 cookie 由回环代理持有（`src-tauri/src/dsh_upstream.rs:9-12`）。前端 fetch 不带凭据也不需要带。

### 1.2 引擎从哪来：settings 清单 → DSH patch → 插件注册表

引擎的来源是一条**三段式链路**，跨越三个地方，值得完整说明：

```
settings.json  settings.web.engines[]          ← 千寻设置（用户可手改，UI 只读展示）
   kind: 'bing' | 'duckduckgo' | 'searxng'     contract.ts:180-186
   （baidu 已移除：对无 cookie 抓取硬反爬）
        │
        │  Rust 把 searchProvider: qx-websearch 写进
        │  <profile>/cordis.patch.yml
        ▼
DSH 启动时加载 qx-websearch 插件           websearch/commands.rs:207-309
   - searchProvider 覆盖行（指定搜索实现）
   - insert 条目（挂载插件本体 + 动态 config）
   - fetchProvider 覆盖行（网页正文抓取 → moli）
        │
        ▼
packages/websearch/src/                 真正的 provider 路由 / 冷却 / 体检
   config.ts   配置 + 迁移 + 别名归一
   router.ts   provider 选择
   cooldown.ts 反爬冷却
   doctor.ts   环境体检
   search.ts   实际检索
```

`WebPage.svelte:12` 的 `settings.current?.web.engines` 只用于**渲染下拉框选项**（`:54-56`），真正的引擎解析在插件侧。

### 1.3 与 `packages/websearch` 的同步关系（简报重点）

**结论：单向手工同步，当前无漂移，但无门禁。**

同步机制（`packages/websearch/scripts/sync-assets.mjs`）：

1. 触发：`pnpm sync:websearch` = `pnpm build:websearch && node packages/websearch/scripts/sync-assets.mjs`（根 `package.json:16-17`）；
2. 拷贝清单：`sync-assets.mjs:10-18` 硬编码 7 个文件（`package.json`、`dsh/{index,client,spawnHidden}.js`、`dsh/{search,fetch}-schema.json`、`dist/main.js`）；
3. 落点：`src-tauri/src/websearch/assets/`（`sync-assets.mjs:8`）；
4. 消费：Rust 用 `include_str!` 在**编译期**把 7 个文件嵌进二进制（`websearch/commands.rs:16-22`），启动时 `ensure()` 幂等写进用户 profile（`:60-100`）。

**我实测的漂移状态**：对 7 个文件逐一做 SHA-256 比对（`packages/websearch/X` vs `src-tauri/src/websearch/assets/X`），结果 **7/7 完全一致**；`git status --porcelain packages/websearch src-tauri/src/websearch` **无输出**（两处均已提交且干净）。所以**当前没有漂移**。

**但风险是真实的**（见 §4-W1）：Rust 侧有一层运行时自愈（`plugin_stale()` 内容指纹比对，`websearch/commands.rs:106-124`），它能修好**用户 profile 里的旧副本**；而**编译期 assets 本身没有任何校验**。仓库里存在两份 `dist/main.js`（各 1.2MB 构建产物）和两份 `dsh/*.js` 源文件，漏跑一次 sync 就会静默发旧版。

### 1.4 patch 文本维护：三种形态的幂等

`update_patch()`（`websearch/commands.rs:207-309`）是本模块最值得学的代码——不引 YAML 依赖，纯文本维护，且把 `cordis.patch.yml` 的三种历史形态都处理了：

| 输入形态                            | 行为                                               | 测试                  |
| ----------------------------------- | -------------------------------------------------- | --------------------- |
| 无 `PATCH_MARK`，内容是 `[]`/纯注释 | 重写为完整块                                       | `commands.rs:370-378` |
| 无 `PATCH_MARK`，已有其他条目       | 文末追加，不破坏既有条目                           | `commands.rs:388-395` |
| 有 `PATCH_MARK` 无 `FETCH_MARK`     | 旧块迁移：补插 `fetchProvider` 行 + 补 `config` 段 | `commands.rs:407-428` |
| 双标记齐全                          | 只对齐 `moliPath`/`fetchProxy` 当前值              | `commands.rs:430-445` |

「`[]` 残留会产出非法 YAML，DSH 会拒绝启动」这个坑被显式断言（`commands.rs:378`）；「不误伤同文件其他条目的同名键」（桥条目自己的 `moliPath`）也有专门测试（`:430-445`）。`fetch_patch_values`（`:191-197`）还把 moli 三源解析与 `settings.web.proxy` 的动态值在每次启动时对齐。

---

## 2. 功能清单与完成度

### 2.1 前端（`WebPage.svelte` 122 行 + `web.svelte.ts` 100 行）

| #   | 功能                             | 状态       | 证据(file:line)                            | 备注                                                                  |
| --- | -------------------------------- | ---------- | ------------------------------------------ | --------------------------------------------------------------------- |
| 1   | 搜索输入 + 回车/按钮提交         | 已完成     | `WebPage.svelte:41-47, 25-27, 58-65`       | 双入口                                                                |
| 2   | 引擎下拉（读 settings 清单）     | 已完成     | `:48-57`                                   | 选项只读展示，UI 不提供增删                                           |
| 3   | 引擎初值对齐 `defaultEngine`     | 部分完成   | `web.svelte.ts` 无；`WebPage.svelte:15-19` | **仅当 `engineId === ''` 时对齐一次**，见 §4-W3                       |
| 4   | 结构化结果列表                   | 已完成     | `:99-117`                                  | 标题/URL/摘要，纯文本渲染                                             |
| 5   | Markdown 汇总（可复制 `<pre>`）  | 已完成     | `:118-127`                                 | 折叠在 `<details>` 里                                                 |
| 6   | DSH 未运行引导                   | 已完成     | `:69-79`                                   | 错误文案 + 跳转环境页按钮                                             |
| 7   | 空态区分「没搜」/「搜了 0 结果」 | 已完成     | `web.svelte.ts:34`；`WebPage.svelte:92-97` | `done` 标志位；0 结果时还提示「百度反爬较严，建议 duckduckgo / bing」 |
| 8   | 加载态                           | 已完成     | `:90-91`                                   | `busy` 时显示「搜索中…」                                              |
| 9   | 错误态                           | 已完成     | `:83-89`                                   | 服务端 `body.error` 或 `HTTP {status}`                                |
| 10  | 30s 超时                         | 已完成     | `web.svelte.ts:24, 83`                     | `AbortSignal.timeout`                                                 |
| 11  | 请求代际（丢弃过期响应）         | 已完成     | `web.svelte.ts:36, 71, 86, 94, 100`        | `seq` 计数器；`finally` 里也判代际，`busy` 不会被旧请求误清           |
| 12  | 无输入时清空                     | 已完成     | `web.svelte.ts:53-60`                      | 不发请求                                                              |
| 13  | `aria-live="polite"` 播报状态    | 已完成     | `WebPage.svelte:82`                        | ADR-016                                                               |
| 14  | 输入防抖搜索                     | **死代码** | `web.svelte.ts:104-107`                    | `scheduleSearch` 定义后零调用，见 §4-W4                               |
| 15  | 结果分页 / 翻页                  | **未实现** | 无                                         | `limit: 10` 硬编码                                                    |
| 16  | 搜索历史                         | **未实现** | 无                                         |                                                                       |
| 17  | 摘要去重 / 站内搜索              | **未实现** | 无                                         |                                                                       |
| 18  | 网页正文抓取（`web_fetch`）入口  | 不在本页   | `websearch/commands.rs:26`                 | fetchProvider 供 DSH agent 用，无前端 UI                              |
| 19  | 引擎体检（doctor）入口           | 不在本页   | `packages/websearch/src/doctor.ts`         | CLI 形态，DSH 设置卡调用                                              |

### 2.2 后端（`src-tauri/src/websearch/commands.rs` 440 行）

| #   | 功能                              | 状态       | 证据(file:line)         | 备注                                                           |
| --- | --------------------------------- | ---------- | ----------------------- | -------------------------------------------------------------- |
| 20  | 插件 7 文件幂等落盘               | 已完成     | `:126-163`              | `create_dir_all` + 逐个 `write`                                |
| 21  | patch 三态维护                    | 已完成     | `:207-309`              | 见 §1.4                                                        |
| 22  | 动态值对齐（moliPath/fetchProxy） | 已完成     | `:191-197, 87-94`       | 每次启动对齐                                                   |
| 23  | 内容指纹自愈                      | 已完成     | `:71-81, 106-124`       | 逐文件比对，缺文件或内容不同即重写                             |
| 24  | 启动期幂等保活                    | 已完成     | `:60-100`               | profile 未就绪时静默跳过，失败只记日志不阻断                   |
| 25  | 状态查询                          | 已完成     | `:336-357`              | `deployed`/`patchEntry`/`pluginDir`/`dshRunning`               |
| 26  | 手动部署 IPC                      | 已完成     | `:42-48`                | `websearch_deploy` / `websearch_status`（`contract.ts:81-82`） |
| 27  | 单测覆盖                          | 已完成     | `:359-445+`             | 4 个测试，含幂等/迁移/动态值/指纹                              |
| 28  | YAML 值转义                       | 部分完成   | `:209`                  | 只处理反斜杠与双引号；Windows 路径已统一成正斜杠（正确）       |
| 29  | 部署失败的 UI 反馈                | **未实现** | `WebPage.svelte` 无引用 | `websearch_status` 有 IPC 契约但前端零消费，见 §4-W6           |

### 2.3 安全边界

| #   | 检查项                       | 结论                      | 证据                                                                                                                                   |
| --- | ---------------------------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| 30  | 未信任内容是否注入 innerHTML | ✅ **否**                 | `WebPage.svelte:7-9` 注释 + `:99-127` 全部走 Svelte 文本插值；无 `{@html}`                                                             |
| 31  | 外部协议打开                 | ✅ 受限                   | `opener:allow-open-url` 的 allow 列表只有 `https://**` / `http://**`（`capabilities/default.json`），`javascript:`/`file:` 被 ACL 拒绝 |
| 32  | 导航拦截                     | ✅ **不适用**             | 本页无 iframe/无导航；结果链接走 `openUrl` 交系统浏览器                                                                                |
| 33  | iframe sandbox / allow       | ✅ **不适用**             | 本页无 iframe（sandbox 在 DSH 模块，见 `04-DSH-dsh.md`）                                                                               |
| 34  | 搜索请求的目标地址           | ✅ 回环                   | `harness.proxyUrl` 来自 Rust，非用户可控（`harness/commands.rs:76-85`）                                                                |
| 35  | 引擎配置是否可被远程内容改写 | ⚠️ 部分                   | 引擎清单在 `settings.json`，搜索结果无法改写它；但插件的 `doctor`/`config` 走 DSH 设置卡与 CLI                                         |
| 36  | 独立包的双源配置             | ⚠️ **是**                 | 见 §4-W2                                                                                                                               |
| 37  | 双份资产漂移                 | ✅ 当前无漂移 / ⚠️ 无门禁 | 见 §4-W1                                                                                                                               |

---

## 3. 实现分析

### 3.1 架构与数据流

```
用户输入 query
   │
   ├─ WebPage.submit() → web.search()                      web.svelte.ts:52
   │     ├─ requireBase()  ← harness.proxyUrl + phase==ready  :44-50（不满足即抛中文错误）
   │     ├─ seq = ++this.seq                                 :71（代际）
   │     └─ fetch(base + '/qx/web/search', {query, engine, limit:10}, 30s)
   │            │
   │            ▼  （本机回环，不出网）
   │      remote/gateway.rs ──► dsh_upstream::forward（cookie 服务端持有）
   │            │
   │            ▼
   │      DSH webServer  /qx/web/search
   │            │  （searchProvider 覆盖行指向 qx-websearch）
   │            ▼
   │      qx-websearch 插件 ──► router.ts（provider 选择）
   │                          ──► cooldown.ts（反爬冷却）
   │                          ──► search.ts（实际检索，出网）
   │            │
   │            ▼  { ok, items[], markdown, error }
   │      web.items / web.markdown / web.error             :90-98
   │
   └─ 渲染：<ol> 结构化列表（纯文本）                        WebPage.svelte:99-127
         点标题 → openUrl(item.url) → 系统浏览器           :29-31
```

**边界很清楚**：千寻的 Rust 侧**完全不参与搜索执行**，只负责把插件落到 DSH 能找到的位置。`websearch/commands.rs` 的 440 行里没有一行 HTTP 请求代码。这是干净的职责切分——搜索逻辑可独立演进、独立测试（`packages/websearch` 有 466 个用例），宿主只管部署。

### 3.2 前后端契约

本模块**没有** Rust IPC 参与搜索路径，只有两个部署类命令（`contract.ts:81-82`：`websearch_deploy`/`websearch_status`）。前端与后端的实际契约是 HTTP + JSON，由 `web.svelte.ts:10-22` 的两个 TS interface 手写描述：

```ts
export interface WebSearchItem {
  title;
  url;
  snippet;
  engine;
}
export interface WebSearchResponse {
  ok;
  markdown?;
  items?;
  error?;
}
```

**这是一个隐性契约风险**：这两个 interface 没有像 harness 域那样被 `contract.test.ts` 的 `RESPONSE_TYPES` 锁死，也没有从 Rust/插件侧生成。插件改了响应形状（例如给 `items` 加必填字段或改名），TypeScript 不会报错，运行时才炸。对比 `contract.ts:132+` 的做法（`IntelTopicStatus` 等都在测试名单里），这里是明显的一致性落差。

`WebEngineSettings.kind`（`contract.ts:183`）声明为 `'bing' | 'duckduckgo' | 'searxng'`，注释说明 baidu 已移除。但 `WebPage.svelte:54-56` 渲染 `<option>` 时用的是 `{engine.id}` 而非 `engine.kind`——下拉框显示的是 id，展示与 kind 的关系依赖用户在 settings.json 里手写时保持一致。

### 3.3 资源与生命周期

| 资源                      | 生命周期                                 | 评价                                     |
| ------------------------- | ---------------------------------------- | ---------------------------------------- |
| 前端 store                | 模块级单例，页面切走不清空（keep-alive） | ✅ 切回保留上次结果                      |
| 搜索请求                  | 单发；`AbortSignal.timeout(30s)` 兜底    | ✅ 有界                                  |
| 结果数组                  | 每次搜索整体替换                         | ✅ 有界（`limit: 10`）                   |
| 定时器                    | `scheduleSearch` 的 `this.timer`         | ⚠️ 唯一使用点，且该方法无人调用（§4-W4） |
| 插件副本（用户 profile）  | `ensure()` 每次启动幂等同步 + 指纹自愈   | ✅ 升级必然生效                          |
| 插件副本（编译期 assets） | 编译时 `include_str!` 固化               | ⚠️ 手工同步，无门禁（§4-W1）             |

无内存泄漏面：无全局监听、无轮询、无订阅。

### 3.4 错误处理与恢复

| 失败点                  | 表现                                                | 评价                        |
| ----------------------- | --------------------------------------------------- | --------------------------- |
| DSH 未运行              | 中文错误 + 跳转环境页                               | ✅ 好                       |
| HTTP 非 2xx             | `body.error ?? 'HTTP ' + status`                    | ✅                          |
| `body.ok === false`     | 同上                                                | ✅                          |
| 网络异常 / 30s 超时     | `error.message` 直接展示（`AbortError` 会显示英文） | ⚠️ 超时文案是英文，见 §4-W7 |
| 过期响应                | 静默丢弃（代际保护）                                | ✅ 正确                     |
| 引擎无结果              | 明确区分「没搜」/「搜了 0 结果」+ 换引擎建议        | ✅ 好                       |
| 插件未部署 / patch 缺失 | **前端无感知**，表现为搜索报错                      | ⚠️ 见 §4-W6                 |

注意「静默丢弃过期响应」的一个正确细节：`finally` 里是 `if (seq === this.seq) this.busy = false`（`web.svelte.ts:100`）。若只有 `this.busy = false`，旧请求返回会把新请求的加载态提前清掉——这里判了代际，是对的。

---

## 4. 缺陷清单

| 级别   | 问题                                                                                                                                                                      | 位置                                                                                                 | 影响                                                                                                                                                                                                                                                                               | 证据                                                                     |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| **P1** | **编译期资产无一致性门禁**：`packages/websearch` → `src-tauri/src/websearch/assets/` 靠手工 `pnpm sync:websearch`，7 个文件两份各存一份（含两个 1.2MB 的 `dist/main.js`） | `packages/websearch/scripts/sync-assets.mjs`；根 `package.json:16-17`；`websearch/commands.rs:16-22` | 漏同步 = 给**所有新用户**发旧版搜索插件，且无任何报错。运行时自愈（`plugin_stale`）只能修用户 profile 的副本，**修不了编译期 assets 本身**。`pnpm check`/`all:check`/CI 均不含此项校验                                                                                             | 本次实测 7/7 一致且 git 干净（§7），即**当前无漂移**；风险在于机制无护栏 |
| **P2** | **双源引擎配置**：`settings.web.engines`（千寻 `settings.json`，UI 只读、提示「增删请编辑 settings.json」）与 `~/.qianxun/websearch/config.json`（插件自己的配置）并存    | `packages/websearch/src/config.ts:68-69`；`contract.ts:180-198`；`SettingsPage.svelte:712`           | 用户在设置页看到的引擎清单与插件实际生效的 provider 路由可能不一致；排查时不知道该改哪个文件。插件侧还有 `migrateLegacyConfig`（`config.ts:179`）与别名归一（`CANONICAL_ENGINE` 折 `antigravity→antigravity-cli`、`http/direct→local`），千寻 UI 完全不体现这层                    |
| **P2** | 结果列表以 `item.url` 为 key，重复 URL 会塌陷                                                                                                                             | `WebPage.svelte:100`                                                                                 | 同一 URL 出现两次（bing/searxng 常见：重定向条目 + 直连条目，或聚合源重复收录）时 keyed `{#each}` 键冲突，Svelte 开发模式报错、生产模式错渲染/丢条目                                                                                                                               |
| **P2** | `settings.web.resultLimit` 是死设置                                                                                                                                       | `contract.ts:192` vs `web.svelte.ts:82`                                                              | 声明了「结果条数上限」但前端**硬编码 `limit: 10`**，实测 `resultLimit` 在整个 `src/` 只出现在 `contract.ts:192` 一处（定义处）。用户改设置无任何效果                                                                                                                               |
| **P2** | 引擎下拉的选中值可能失效                                                                                                                                                  | `WebPage.svelte:15-19, 48-57`                                                                        | `$effect` 仅在 `web.engineId === ''` 时对齐一次。设置被改（用户手改 `settings.json` 后重载、或未来 UI 支持增删引擎）后，`engineId` 仍是旧 id，`<select>` 找不到匹配 option → 显示空白/错位，而 `search()` 仍把失效 id 发给服务端（`web.svelte.ts:80`）→ 报一个用户完全无法理解的错 |
| **P2** | 部署状态无 UI 反馈                                                                                                                                                        | `websearch/commands.rs:42-56`（`websearch_deploy`/`websearch_status`）；`contract.ts:634-642`        | 两个 IPC 都有完整 Rust 实现和 `WebsearchStatus` 契约，但 `WebPage.svelte` 零引用。插件没部署好时，用户只在搜索结果区看到一个泛化错误，看不到「插件未部署 / patch 缺失 / 需重启 DSH」这类可操作信息                                                                                 |
| **P3** | `scheduleSearch` 是死代码                                                                                                                                                 | `web.svelte.ts:104-107`                                                                              | 防抖搜索实现完整（`clearTimeout` + 300ms）却无人调用，实测全仓命中数 1（仅定义处）。要么接线要么删除                                                                                                                                                                               |
| **P3** | 点击结果失败完全静默                                                                                                                                                      | `WebPage.svelte:29-31`                                                                               | `openUrl(url).catch(() => {})`。结果 URL 来自第三方 provider（不可信），若被 ACL 拒绝或系统浏览器启动失败，用户点了**没有任何反馈**，也不会知道链接有问题                                                                                                                          |
| **P3** | 超时/网络错误的文案可能是英文                                                                                                                                             | `web.svelte.ts:97`                                                                                   | `AbortSignal.timeout` 抛出的是英文 DOMException，直接展示给中文用户。DSH 未运行那种情况反而有精心写的中文提示，两者不一致                                                                                                                                                          |
| **P3** | 硬编码 dev 端口与父页 origin 清单                                                                                                                                         | `src-tauri/src/remote/assets/qx-shell-links.js:23-28`                                                | `PARENT_ORIGINS` 里写死 `localhost:5190` / `127.0.0.1:5190`，与 `vite.config.ts` 的端口配置重复。改 vite 端口会让 DSH 外链桥静默失效（详见 `04-DSH-dsh.md` §4-D4）                                                                                                                 |
| **P3** | 搜索历史 / 分页 / 去重全缺                                                                                                                                                | `WebPage.svelte`                                                                                     | 功能面窄于菜单标签「联网」的暗示（见 §4-W5 说明）                                                                                                                                                                                                                                  |

### 关于「功能是否远小于菜单标签」的正面结论

简报问「功能是否远小于菜单标签暗示」。**是，但这是有意的职责切分，不是欠账**：

- 标签叫「联网」，用户心智是「上网」。千寻**确实**有上网能力——但它被放在 DSH 页里（内置浏览器窗），而不在这个菜单项下；
- 放在这里的「联网」实际是「联网搜索」，一个纯 API 化的功能；
- 侧边菜单把「DSH」和「联网」并列为两个入口，实际上「联网」是「DSH 提供的搜索能力」的一个专用前端。

因此这不是功能缺失，而是**信息架构上的错位**：同一个侧边栏里，DSH 项已经能通过聊天里的链接打开内置浏览器了，联网项又只提供搜索。§5-W5 给出改法建议。

---

## 5. 改善建议

按 收益/成本 排序。

| 优先级 | 建议                                                                                                                                                                                   | 改动文件                                                                                          | 收益                                                                                                                          | 成本                                              |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| **1**  | **给资产同步加门禁**：新增一个测试或脚本，比对 `packages/websearch/{package.json,dsh/*,dist/main.js}` 与 `src-tauri/src/websearch/assets/` 的 SHA-256，不一致即失败；挂进 `pnpm check` | 新增 `packages/websearch/scripts/check-assets.mjs`（或 `.test.ts`）+ 根 `package.json:12`         | 高——把 P1-W1 从「靠人记得」变成「CI 挡住」。本次已实测 7/7 一致，说明加门禁零成本                                             | 极低（~20 行，本次报告已写出比对逻辑）            |
| 2      | 接上 `resultLimit` 并补客户端上限                                                                                                                                                      | `src/stores/web.svelte.ts:82`（改读 `settings.current?.web.resultLimit`）+ `contract.ts:192` 复用 | 中——消除一个「改了没用的设置」；顺带可加 `query` 长度上限                                                                     | 极低                                              |
| 3      | 引擎下拉做失效自愈                                                                                                                                                                     | `src/features/web/WebPage.svelte:15-19`                                                           | 中——消除 P2 的失效 id 死局。做法：`$effect` 里校验 `engines.some(e => e.id === web.engineId)`，不成立则回落到 `defaultEngine` | 低（~5 行）                                       |
| 4      | 部署状态条：进页面时调一次 `websearch_status`，`deployed === false` 或 `patchEntry === false` 时在页头显示一条 warn（含「需重启 DSH 生效」+ 修复按钮调 `websearch_deploy`）            | `src/features/web/WebPage.svelte`（页头）+ 可选新建 `src/stores/web.svelte.ts` 方法               | 中——把泛化错误变成可操作提示；`websearch_deploy` 的 IPC 已经现成                                                              | 低-中                                             |
| 5      | 结果 key 改用稳定去重键                                                                                                                                                                | `src/features/web/WebPage.svelte:100`                                                             | 中——修 P2 键冲突。做法：搜索返回后在 store 里按 `url` 去重，或 key 用 `${index}-${item.url}`                                  | 极低                                              |
| 6      | 修 `PARENT_ORIGINS` 的 dev 端口重复                                                                                                                                                    | `src-tauri/src/remote/assets/qx-shell-links.js:23-28`                                             | 中——消除一个跨文件的隐式耦合                                                                                                  | 低（改为构建期注入，或加注释指向 vite.config.ts） |
| 7      | 死代码二选一：接线 `scheduleSearch`（搜索页加防抖）或删除它                                                                                                                            | `src/stores/web.svelte.ts:104-107`                                                                | 低                                                                                                                            | 极低                                              |
| 8      | `openUrl` 失败给 toast 提示；超时错误映射为中文                                                                                                                                        | `WebPage.svelte:29-31`；`web.svelte.ts:93-98`                                                     | 低——消除「点了没反应」                                                                                                        | 极低（项目已有 `stores/toast.svelte.ts`）         |
| 9      | 信息架构：把菜单标签「联网」改为「搜索」，或在页头加一句「点标题用系统浏览器打开；要在应用内浏览请用 DSH 聊天里的链接」                                                                | `src/components/SideNav.svelte:40`、`WebPage.svelte:35-37`                                        | 中——消除用户预期落差（§4 末节）                                                                                               | 极低                                              |
| 10     | 把 `WebSearchItem`/`WebSearchResponse` 纳入契约测试，或从插件 schema 生成                                                                                                              | `src/stores/web.svelte.ts:10-22`；`src/lib/ipc/contract.test.ts`                                  | 中——补上唯一没有门禁的隐性契约                                                                                                | 中                                                |

---

## 6. 完成度评估

| 维度              | 得分   | 依据                                                                                                                                                                                                                                               |
| ----------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 功能完整性        | 70     | 搜索主链路完整可用（输入/引擎/结果/空态/错误态/超时/代际保护都做了）。但功能面本身很窄：无历史、无分页、无去重、无站内搜索、无 fetch 入口 UI；`resultLimit` 是死设置                                                                               |
| 健壮性 / 错误恢复 | 82     | DSH 未运行有明确引导（最好的一个 UX 决策）、超时/代际/空态都覆盖。扣分在部署层故障无 UI 反馈、超时文案英文、`openUrl` 静默失败                                                                                                                     |
| 性能与资源        | 88     | 10 条结果、无监听无轮询、无泄漏面。扣分在 keyed each 的键冲突风险                                                                                                                                                                                  |
| 测试覆盖          | 84     | 独立包 466 用例全绿（router/cooldown/doctor/config/dshClient/dshPlugin/search/subprocess…），Rust patch 层 4 个单测，前端 28 用例含 `external-links` 与 `web.svelte`。扣分：**HTTP 响应形状无契约门禁**，前端 store 的 mock 测试不覆盖真实插件响应 |
| UX                | 76     | 空态文案贴心（主动建议换引擎）、`aria-live` 有做。扣分在信息架构错位、结果打开失败静默、部署故障无提示                                                                                                                                             |
| 可维护性          | 78     | 千寻侧代码极少且干净（`websearch/commands.rs` 的 patch 三态维护质量很高）。扣分在双源配置、双份资产、隐性 HTTP 契约                                                                                                                                |
| **加权总分**      | **78** | 评级 **B / 良好**                                                                                                                                                                                                                                  |

**为什么不是更高**：本模块是三个模块里**唯一有 P1 缺陷**的（资产无门禁，机制性风险），且有三处 P2 级的「声明了但没接线」（`resultLimit`、`websearch_status` UI、失效 engineId）。完成度被机制债而非功能债拉低。

**为什么不是更低**：核心链路确实好用且健壮——依赖检测、30s 超时、过期响应丢弃、空态区分、可诊断错误都在；最关键的「独立包与宿主如何协作」这个难题被 `update_patch` 的三态幂等 + 指纹自愈解决得很漂亮，并且有 4 个针对性单测。

---

## 7. 测试与验证

**实际执行**（工作目录 `E:\develop\dsh-workspace\qianxun`）：

| 命令                                                                                      | 结果                                                                                                                                                                                                                                                                |
| ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cd packages/websearch && npx vitest run`                                                 | **23 files passed + 1 skipped / 24；466 passed / 24 skipped**（1.50s）。stderr 有预期的 `[qx-websearch] web seam has no registerSearchProvider; search provider skipped` 等诊断行（属被测的降级路径）                                                               |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib`（全量）                           | **167 passed / 0 failed / 5 ignored**。本模块直接命中的 4 个：`websearch::commands::tests::{patch三态维护幂等且不破坏既有条目, 旧块迁移补fetch_provider行与config段, 动态值对齐不误伤其他条目的同名键, 内容指纹自愈缺文件或旧内容判定过期}`                         |
| `npx vitest run src/lib/external-links.test.ts src/stores/web.svelte.test.ts src/lib/ipc` | **3 files / 28 tests passed**（384ms）。`web.svelte.test.ts` 是本模块前端 store 的唯一测试面；`external-links.test.ts` 属 DSH 模块（`04-DSH-dsh.md` §7 引用）                                                                                                       |
| 资产漂移核查（PowerShell + `Get-FileHash -Algorithm SHA256`）                             | 对 `sync-assets.mjs:10-18` 清单的 7 个文件逐一比对 `packages/websearch/X` vs `src-tauri/src/websearch/assets/X`：**7/7 SAME**（`package.json`/`dsh/index.js`/`dsh/client.js`/`dsh/spawnHidden.js`/`dsh/search-schema.json`/`dsh/fetch-schema.json`/`dist/main.js`） |
| `git status --porcelain packages/websearch src-tauri/src/websearch`                       | **无输出**——两处资产均已提交且与工作区一致                                                                                                                                                                                                                          |
| 死代码/死设置核查                                                                         | `resultLimit` 在 `src/**` 命中 **1** 处（`contract.ts:192` 定义处，零消费）；`scheduleSearch` 命中 **1** 处（`web.svelte.ts:104` 定义处，零调用）；`installProgress` 在 `src/**/*.svelte` 命中 **0**（属环境模块）                                                  |
| 设置 UI 核查                                                                              | `src/features/settings/SettingsPage.svelte:712` 提示「与 DSH web_search 共用清单 · 已装 N 个 · 增删请编辑 settings.json」——确认引擎清单 UI 只读                                                                                                                     |

**未执行**（如实标注）：

- **未做任何真实搜索**。没有运行中的 DSH 实例，因此 `/qx/web/search` 的端到端行为（真实 provider 响应、markdown 汇总的真实格式、`error` 字段的真实文案）**全部未经真机验证**。§2.1 中第 4/5/7/9 项的功能判定基于代码与类型定义，不是实测。
- 未运行 `packages/websearch` 的 `biome` lint / `tsc`（`pnpm check` 只扫主工程，该包有独立 `biome.json`）。
- 未运行 `pnpm check`（prettier + eslint + svelte-check）全量；未运行 `cargo clippy` / `cargo fmt --check`。
- **未逐行审读** `packages/websearch/src/dsh/index.js`（46.9KB）、`client.js`（42.5KB）、`config.ts`（27.8KB）、`search.ts`（17.8KB）、`doctor.ts`（17.4KB）、`cooldown.ts`（15.3KB）、`router.ts`（11.4KB）。§1.2 的 provider 链路来自 `config.ts` 的符号表 grep（`^(export |const |//! )`）+ `commands.rs` 的 patch 内容 + 466 个通过用例；**未独立验证**每个 provider 的具体检索实现、cooldown 的具体阈值、doctor 的检查项。
- 未审读 `src-tauri/src/remote/gateway.rs`（779 行）与 `dsh_upstream.rs`（740 行）的全部转发逻辑——仅读了模块头注释、关键常量与 `#[cfg(test)]` 名单（二者测试均在全量 167 passed 内）。转发层的深入分析超出本模块边界。

**补记（本报告自身格式校验）**：`npx prettier --check` 对本文件**初次未通过**（表格列宽对齐问题），已执行 `npx prettier --write` 仅格式化本文件后复检通过 → `All matched files use Prettier code style!`。写入范围仅限本文件，未触碰同目录其他 worker 的报告。

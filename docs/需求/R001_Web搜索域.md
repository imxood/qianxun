# R001 Web 搜索域（联网页 + Agent 工具 + Moli 内核 + Playwright 环境）

> 状态: 进行中
> 优先级: 高
> 功能域: web（新）/ dsh（桥扩展）/ env（moli 安装管理）/ e2e（基建）
> 创建: 2026-02
> 关联: docs/04-里程碑与TODO.md §M1（安装流水线模式）§M6（qx-bridge 模式）, docs/06-ADR-016（测试与可观测性）, docs/07-Tauri调试基建与CDP经验.md

## 一句话

千寻新增「联网」页与 DSH Agent `web_search` / `web_read` / `browser_*` 工具，搜索与抓取经 Moli 无头浏览器内核返回高保真 Markdown，Moli 由环境页安装管理，Playwright 为默认调试/测试环境。

## 背景与验证记录（2026-02 实测）

DSH 自带 web 工具在千寻承载环境不可用，是本需求的直接动机：

| 工具             | 实测结果     | 根因                                                                           |
| ---------------- | ------------ | ------------------------------------------------------------------------------ |
| DSH `web_search` | 可用但质量差 | 后端代理返回 URL+标题，无摘要；多语言混杂，搜「rust入门指南」首条是韩语页      |
| DSH `web_fetch`  | 直接失败     | `URL hostname ... resolves to a non-public IP address`——沙箱仅放行公网 IP 直连 |

千寻侧通道勘察：CSP `connect-src` 已放行 `https:` 与 `http://127.0.0.1:*`；`opener:allow-open-url` 已放行 http(s)；`lib/market/http.ts` 的 fetchJson 是前端直连成熟模板；`qx-bridge`（M6）跑在 DSH 宿主 Node 进程，fetch 不受 WebView 限制，且已有 `ctx.tools.register` 与 webServer 挂端点两条现成通路。

**Moli 实测（本机 moli.exe 1.1.9）**：Moli 为独立 Rust 无头浏览器内核（非 Chromium 封装，MIT/Apache-2.0），内置 V8/DOM/CSS/网络，单 exe 约 99.5MB。

| #   | 实测                                                               | 结果                                                                           |
| --- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| V1  | `moli fetch --dump markdown --wait-until done https://example.com` | PASS，干净 Markdown（冷启动 6.5s）                                             |
| V2  | 必应中文搜索页 `--dump markdown`                                   | PASS，3.0s / 6230 字符，结果完整（含导航噪音，需解析清洗）                     |
| V3  | `moli serve` + Playwright `connectOverCDP`                         | PASS：握手/goto/DOM 读取全通；`click` 卡死——默认 LayoutPolicy::Mock 无真实几何 |
| V4  | `moli serve --layout` + Playwright click 全链路                    | PASS，点击→DOM 变更→读取闭环                                                   |
| V5  | playwright-core 零依赖驱动                                         | 无 postinstall、零运行时依赖、12.8MB、Node ≥20；驱动系统 Edge PASS             |

官方 `skills/moli-websearch` 提供多引擎搜索 URL 表（Bing/Baidu/头条/DuckDuckGo 等）与并行策略；`moli-html2md`（turndown 兼容）保证 MD 质量。

**Playwright 环境实测**：`@playwright/test` 1.62.1 已装；debug 二进制就位；CDP 10222 fixture 完整；msedge 无头启动断言 PASS。缺口：剥离环境变量的沙箱下 TMP/TEMP/SYSTEMDRIVE/PROGRAMFILES 为 null 导致 mkdtemp/浏览器定位 ENOENT——由 `pnpm e2e:doctor` 自检覆盖。

## 决策点

| #   | 决策            | 结论                                                                                                                     |
| --- | --------------- | ------------------------------------------------------------------------------------------------------------------------ |
| D1  | 功能挂载位置    | 独立「联网」页（侧栏新入口）+ DSH Agent 工具双落点（用户确认 2026-02）                                                   |
| D2  | 搜索后端        | 用户可配置多源引擎（Bing/Baidu/头条/DuckDuckGo/searxng 预设 + 自定义），settings 可视化（用户确认 2026-02）              |
| D3  | 结果形态        | 统一 Markdown：搜索结果 → md 列表，网页抓取 → md 正文（用户确认 2026-02）                                                |
| D4  | 内核位置        | 桥（DSH 宿主 Node 进程）为单一事实源，UI 与 agent 两个薄出口；渲染经 Moli（用户确认 2026-02，合并原 R002）               |
| D5  | HTML→MD         | 优先 `moli fetch --dump markdown`；Node fetch 快路径兜底（无 moli 时轻量解析）                                           |
| D6  | 前端通道        | 前端直接 fetch DSH webServer 端点（`POST /qx/web/search` / `/qx/web/read`），零新增搜索类 IPC                            |
| D7  | Playwright 定位 | 默认调试/自动化测试环境；新增 `pnpm e2e:doctor` 自检（用户确认 2026-02）                                                 |
| D8  | Moli 安装管理   | 环境页管理：检测/下载（镜像加速）/校验/落盘 `<数据目录>/tools/moli/`；settings 可指定自备 binaryPath（用户确认 2026-02） |
| D9  | Moli SSRF 防护  | agent 可控 URL 一律 `--block-private-networks`                                                                           |
| D10 | Moli 版本策略   | pinned 起步 1.1.9，settings 可改；对齐 fff 锁版本惯例                                                                    |
| D11 | 桥消费分层      | 轻请求（JSON API）Node fetch；web_read/web_search 走 moli CLI 一次式；browser_* 走 `serve --layout` 按需拉起 + 空闲关停  |

## 影响范围

### 角色

- 主要: 千寻用户（联网页搜索/阅读，结果 Markdown 可存笔记）; DSH Agent 使用者（联网搜索/读正文/浏览器操作）
- 次要: 开发与维护者（Playwright 默认调试测试环境）

### 功能域

- `web`（新增）: `src/features/web/WebPage.svelte` + `src/stores/web.svelte.ts`
- `dsh`（扩展）: `src-tauri/src/bridge/assets/`（搜索引擎表 + md 解析 + web 工具/端点 + moli 消费）
- `env`（扩展）: 环境页 moli 状态卡 + `src-tauri/src/tools/moli.rs`（检测/安装）
- `settings`（扩展）: `web` 段 + `tools.moli` 段 + SettingsPage 配置卡
- `nav`（微改）: PageId 增 `web`；SideNav/App 登记
- `e2e`（强化）: `pnpm e2e:doctor` 自检脚本

## 规范（实施时必须遵守）

- docs/03-编码规范.md §7（前端网络请求对齐 fetchJson 风格：超时+错误归一；桥内 spawn 超时+截断）§8 §9（测试位置与规则）
- docs/06-ADR-016 门禁表（all:check + cargo 三件套全绿；新组件 data-testid + aria-live）
- docs/04 §M1（安装流水线：下载/校验/事务）、§M6（桥工具注册模式）
- docs/07 §1 §4（CDP 调试协议）

## 约束（实施时必须避免）

- 禁止静默吞错：搜索/抓取失败必须落 UI 错误态
- 禁止 WebView 直抓第三方搜索站；抓取只在桥（Node/moli）
- agent 可控 URL 必经 `--block-private-networks`（D9）；桥 spawn 必须超时 + 256KB 截断
- 浏览器自动化只用 moli（serve --layout），不下载浏览器、不依赖系统 Edge；serve 空闲自动退出
- e2e/调试只允许 debug 构建；临时探针进 `.tmp/`
- 测试必须真实可跑：门禁命令全绿，禁止假测试/空断言

## 架构与数据流

```
千寻 WebView: WebPage（搜索框/引擎切换/md 渲染）
      │ fetch（CSP 已放行 127.0.0.1）
      ▼
DSH webServer（桥挂载）: POST /qx/web/search | /qx/web/read
      │ 桥路由（D11）
      ├─ 轻请求 ──► Node fetch（毫秒级）
      ├─ web_read/search ──► spawn: moli fetch --dump markdown --block-private-networks
      │        └─ 引擎表拼 URL → 桥解析 md → items + markdown 双份
      └─ browser_* ──► moli serve --layout（按需拉起/空闲关停）◄── playwright-core connectOverCDP
DSH Agent ──► ctx.tools.register: web_search / web_read / browser_*（同一内核）
千寻环境页 ──► moli_status / moli_install（检测/下载/校验/落盘）
调试/测试: pnpm dev + CDP 10222 + playwright 探针；pnpm e2e；pnpm e2e:doctor
```

## 接口

### 桥 HTTP 端点（前端消费）

`POST /qx/web/search`：请求 `{ query, engine?, limit? }` → 响应 `{ markdown, items: [{title,url,snippet,engine}] }`
`POST /qx/web/read`：请求 `{ url }` → 响应 `{ markdown, bytes, truncated }`（>256KB 截断）

### Agent 工具（DSH 侧）

- `web_search(query, engine?, limit?)` → md 文本
- `web_read(url)` → md 正文；经 moli `--block-private-networks`
- `browser_open/snapshot/click/fill/screenshot/close` → moli serve --layout + playwright-core；仅 http(s)
- 系统提示注入：何时用 web_search / web_read / browser_*

### settings.json 新增段

```jsonc
"web": {
  "defaultEngine": "bing",
  "engines": [
    { "id": "bing", "type": "bing" },
    { "id": "baidu", "type": "baidu" },
    { "id": "duckduckgo", "type": "duckduckgo" },
    { "id": "searxng", "type": "searxng", "endpoint": "https://searx.example.com" }
  ],
  "resultLimit": 10
},
"tools": {
  "moli": { "enabled": true, "pinnedVersion": "1.1.9", "binaryPath": "" }
}
```

serde rename_all camelCase + serde default 零迁移；endpoint 仅 https；resultLimit clamp。

### 环境页 IPC（新增，登记 contract.ts）

- `moli_status` → `{ installed, version, path, source: "managed"|"custom"|"path"|"none" }`

## 实现位置

- 桥内核: `src-tauri/src/bridge/assets/index.js` + `websearch.js`（纯函数模块，可被 vitest 直接导入）
- Moli 管理: `src-tauri/src/tools/moli.rs`（新）
- settings: `src-tauri/src/settings.rs`
- 前端: `src/features/web/WebPage.svelte`、`src/stores/web.svelte.ts`、`src/stores/nav.svelte.ts`、`src/components/SideNav.svelte`、`src/App.svelte`、`src/lib/ipc/contract.ts`
- 环境页: `src/features/env/EnvPage.svelte`（moli 卡）
- e2e 自检: `e2e/doctor.ts` + package.json scripts

## 实施步骤

- **A 设置与检测（Rust）**: settings web + tools.moli 段（校验+单测）→ tools/moli.rs 检测（managed/custom/PATH 三源，真实进程 `--version` 探测+单测）→ moli_status IPC + contract 登记
- **B 桥内核**: websearch.js（引擎表 + markdown→items 解析纯函数，vitest 真测）→ index.js 注册 web_search/web_read agent 工具 + /qx/web/* 端点（moli spawn 超时/截断/降级 Node fetch）
- **C 前端**: nav/SideNav/App 登记 → web store（防抖/代际/错误态）+ 单测 → WebPage（搜索框/引擎下拉/md 渲染/外跳/失败态）→ SettingsPage web 配置卡
- **D e2e 基建**: e2e/doctor.ts + pnpm e2e:doctor；探针 `.tmp/probe-web.ts`
- **E 文档**: README 索引状态流转；docs/04 登记里程碑

## 兼容性

| 环节                 | 影响                                            |
| -------------------- | ----------------------------------------------- |
| 既有 settings.json   | 零迁移（serde default）                         |
| moli 未安装          | 桥降级 Node fetch 快路径；环境页/联网页显式引导 |
| IPC 合同             | 仅新增 moli_status；contract.test 同步          |
| qx-bridge 既有三工具 | 追加式扩展，不触碰                              |
| 移动端               | 不涉及                                          |
| 既有 e2e             | 不受影响；doctor 为增量前置自检                 |

## 验收场景

1. 联网页搜「rust 入门指南」（默认 bing）→ 中文优先 md 列表；点击系统浏览器打开
2. 切引擎复搜；单引擎失败 → 错误态 + 其余不受影响
3. agent 对话触发 web_search / web_read 返回 md；web_read JS 渲染页拿到动态内容
4. web_read 传 `http://127.0.0.1:17300` → `--block-private-networks` 拒绝
5. 停 DSH → 联网页失败态 + 引导，不白屏
6. 旧 settings.json（无 web/tools 段）启动零迁移
7. moli_status 三源检测正确（custom path / managed / PATH / 均无 = none）
8. `pnpm e2e:doctor` 正常机全绿；异常给修复指引
9. `pnpm all:check` + cargo 三件套全绿（真实通过，禁止假测试）

## 更新日志

- 2026-02 创建（DSH web 工具不可用存档；Playwright 实测 PASS + 环境缺口存档；D1-D7）
- 2026-02 合并 Moli 集成（原 R002）: moli 1.1.9 五项实测存档；D8-D11（安装管理/SSRF/版本/分层消费）
- 2026-02 进入实施（A→E 分步；门禁真实全绿为完成标准）
- 2026-02 浏览器自动化落地（用户定调：一切自动化 = playwright + moli，包括开发机 e2e，不用 Edge）：browsersession.js 状态机（假件 6 单测：会话复用/空闲关停/连接失败回收）+ 真实集成测试（playwright-core + moli serve --layout 全链路 open/snapshot/fill/click/screenshot/close，本机 2 用例绿）；playwright-core 入 devDep + settings.tools.playwrightCorePath 配置通道 + patch 注入；e2e:doctor 的 Edge 检查替换为 moli 定位 + playwright-core 检查（实测 6/6 绿）
- 2026-02 步骤 A/B/C/D 主体落地：settings web+tools.moli 段（8 单测）；tools/moli.rs 三源检测+真实桩进程探测（5 单测）；moli_status IPC+合同登记；桥 websearch.js 引擎表+md 解析（11 vitest）+web_search/web_read 工具+/qx/web/* 端点+moli spawn 降级路径；deploy 落盘 websearch.js+patch 注入 moliPath（patch 三态测试同步）；nav/SideNav/App 登记+WebPage+web store（3 单测）+设置页联网卡；e2e:doctor 六项自检实测全绿。门禁实测：cargo fmt/clippy -D warnings/test 123 通过；pnpm check（prettier/eslint/svelte-check 0 errors/vitest 97）全绿。待办：browser_* 工具（serve --layout 生命周期）、moli_install 下载流水线、设置页引擎清单可视化编辑（暂经 settings.json）

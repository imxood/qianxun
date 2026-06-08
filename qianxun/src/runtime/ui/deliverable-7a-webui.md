# Stage 7a — Daemon Web Admin Console (SvelteKit SPA + 4 核心面板)

> 完成时间: 2026-06-02 22:17 | verifier-pending
> 父 task: plan_91dbeea9 / `webui-stage7a-scaffold-4-panels`
> 关联 daemon 端 task: `daemon-stage7a-llm-and-serve` (sister producer)

## Summary

新建 `qianxun/src/daemon/ui/` SvelteKit SPA 项目 (Svelte 5 + Vite 8 + Tailwind 4 + shadcn-svelte 风格 + adapter-static + mode-watcher), 实现 Stage 7a 的 4 个核心管理面板 (LLM / Skills / MCP / Tools), 配套 token 鉴权 UI, 6 个测试文件 / 68 个 vitest 全过, svelte-check 0 errors / 0 warnings, pnpm build 成功 (276KB build/), 4 张 playwright 截图 (生产 build serve) 覆盖每个核心页面.

## Changed files

### 项目根 (7)
- `qianxun/src/daemon/ui/.gitignore`
- `qianxun/src/daemon/ui/.npmrc`
- `qianxun/src/daemon/ui/package.json` (21 devDeps, 跟 qianxun-desktop 同步)
- `qianxun/src/daemon/ui/pnpm-lock.yaml`
- `qianxun/src/daemon/ui/svelte.config.js` (adapter-static, fallback 'index.html')
- `qianxun/src/daemon/ui/tsconfig.json`
- `qianxun/src/daemon/ui/vite.config.ts` (端口 5174, proxy /v1/* → 127.0.0.1:23900)
- `qianxun/src/daemon/ui/vitest.config.ts` (jsdom + Svelte 5 runes + 3 个 alias stub)

### 入口 (3)
- `qianxun/src/daemon/ui/src/app.css` (Tailwind v4 + 千寻品牌色 `--qianxun-accent: #ff7a3d`)
- `qianxun/src/daemon/ui/src/app.d.ts`
- `qianxun/src/daemon/ui/src/app.html`

### 路由 (8)
- `qianxun/src/daemon/ui/src/routes/+layout.svelte` (Sidebar + TopBar + TokenDialog)
- `qianxun/src/daemon/ui/src/routes/+layout.ts` (`prerender=false, ssr=false` 强制 SPA)
- `qianxun/src/daemon/ui/src/routes/+page.svelte` (`/` → `/llm` 重定向)
- `qianxun/src/daemon/ui/src/routes/llm/+page.svelte` (Provider 列表 / 新增 / 编辑 / 删除 / 测试 / 激活)
- `qianxun/src/daemon/ui/src/routes/skills/+page.svelte` (Skill 列表 / 重载 / 启停 / 详情)
- `qianxun/src/daemon/ui/src/routes/mcp/+page.svelte` (Server 列表 / 新增 stdio-HTTP / 删除 / 测试)
- `qianxun/src/daemon/ui/src/routes/tools/+page.svelte` (Tool 列表 / Schema 详情 / 试用 invoke)
- `qianxun/src/daemon/ui/src/routes/settings/+page.svelte` (Stage 7b 占位)
- `qianxun/src/daemon/ui/src/routes/system/+page.svelte` (Stage 7b 占位)

### Lib (16)
- `qianxun/src/daemon/ui/src/lib/api/client.ts` (fetchWithAuth + ApiError + AuthRequiredError)
- `qianxun/src/daemon/ui/src/lib/api/llm.ts` (7 个函数)
- `qianxun/src/daemon/ui/src/lib/api/skills.ts` (3 个函数)
- `qianxun/src/daemon/ui/src/lib/api/mcp.ts` (4 个函数)
- `qianxun/src/daemon/ui/src/lib/api/tools.ts` (2 个函数)
- `qianxun/src/daemon/ui/src/lib/api/system.ts` (2 个函数: getHealth, getStatus)
- `qianxun/src/daemon/ui/src/lib/api/api.test.ts` (22 测试)
- `qianxun/src/daemon/ui/src/lib/stores/auth.svelte.ts` (token + localStorage)
- `qianxun/src/daemon/ui/src/lib/stores/auth.test.ts` (5 测试)
- `qianxun/src/daemon/ui/src/lib/stores/theme.svelte.ts` (light/dark/system + mode-watcher)
- `qianxun/src/daemon/ui/src/lib/types/api.ts` (跟 daemon 共享的 schema)
- `qianxun/src/daemon/ui/src/lib/utils.ts` (cn + WithElementRef)
- `qianxun/src/daemon/ui/src/lib/utils/format.ts` (formatTimestamp/Bytes/Latency/truncate)
- `qianxun/src/daemon/ui/src/lib/utils/format.test.ts` (17 测试)
- `qianxun/src/daemon/ui/src/lib/utils/cn.test.ts` (5 测试)

### Components (22)
UI 组件 (16):
- `button/Button.svelte` (variants: default/destructive/outline/secondary/ghost/link; sizes: sm/default/lg/icon)
- `card/Card.svelte` + `CardHeader.svelte` + `CardTitle.svelte` + `CardDescription.svelte` + `CardContent.svelte` + `CardFooter.svelte`
- `input/Input.svelte` + `Textarea.svelte`
- `label/Label.svelte`
- `select/Select.svelte` (受控 value + onchange)
- `badge/Badge.svelte` (variants: default/secondary/destructive/success/warning/info/outline)
- `table/Table.svelte` + `TableHeader.svelte` + `TableBody.svelte` + `TableRow.svelte` + `TableHead.svelte` + `TableCell.svelte`
- `dialog/Dialog.svelte` + `DialogBody.svelte` + `DialogFooter.svelte` (ESC 关闭 + backdrop 关闭)
- `components.test.ts` (13 测试)

Layout (2 + 1 测试):
- `layout/Sidebar.svelte` (千寻 logo + 4 核心链接 + Settings/System 占位 + 7b 标签)
- `layout/TopBar.svelte` (daemon 状态指示 + token 配置 + 登出)
- `layout/layout.test.ts` (6 测试)

Common (5):
- `common/Empty.svelte`
- `common/Loading.svelte`
- `common/ErrorBanner.svelte`
- `common/DataTable.svelte` (MVP)
- `common/PageHeader.svelte`

Auth (1):
- `auth/TokenDialog.svelte` (首次访问 / 401 触发, 调 /v1/system/status 验证)

### Test infra (3)
- `qianxun/src/daemon/ui/src/test-setup.ts` (注册 @testing-library/svelte cleanup)
- `qianxun/src/daemon/ui/src/test-stubs/app-environment.ts` ($app/environment mock)
- `qianxun/src/daemon/ui/src/test-stubs/app-state.ts` ($app/state mock)

### Static + scripts (6)
- `qianxun/src/daemon/ui/static/favicon.svg` (千寻橙色 logo)
- `qianxun/src/daemon/ui/scripts/serve.mjs` (本地 SPA-fallback HTTP server, 给 playwright 截图用)

### Screenshots (5)
- `qianxun/src/daemon/ui/screenshots/00-home-redirect.png` (重定向到 /llm)
- `qianxun/src/daemon/ui/screenshots/01-llm.png` (LLM Providers 面板 + 4 核心链接)
- `qianxun/src/daemon/ui/screenshots/02-skills.png` (Skills 面板)
- `qianxun/src/daemon/ui/screenshots/03-mcp.png` (MCP Servers 面板)
- `qianxun/src/daemon/ui/screenshots/04-tools.png` (Tools 面板)

### 本文件
- `qianxun/src/daemon/ui/deliverable-7a-webui.md` (本文件)
- `C:\Users\maxu\.mavis\plans\plan_91dbeea9\outputs\webui-stage7a-scaffold-4-panels\deliverable.md` (engine 验证点)

## 验证清单 (worker self-check)

```
[✓] pnpm --dir qianxun/src/daemon/ui install         0 errors (20 packages)
[✓] pnpm --dir qianxun/src/daemon/ui run check       0 errors / 0 warnings
[✓] pnpm --dir qianxun/src/daemon/ui run build       成功, 产出 build/ 276KB / 32 文件
[✓] pnpm --dir qianxun/src/daemon/ui test            68/68 pass (6 文件, ~13s)
[✓] 4 个核心路由 (/llm /skills /mcp /tools) 都有 +page.svelte
[✓] Sidebar + TopBar + 6 链接都在 +layout.svelte (4 核心 + 2 Stage 7b 占位)
[✓] fetchWithAuth + 5 个 api/*.ts 客户端 (llm 7 fn + skills 3 + mcp 4 + tools 2 + system 2 = 18 端点)
[✓] 没有引用 qianxun-desktop 私有组件 (i18n/sse/ipc) — grep 验证 0 处
```

### vitest 测试结果 (68/68 pass)

```
 RUN  v3.2.6 E:/git/maxu/qianxun/qianxun/src/daemon/ui

 ✓ src/lib/utils/format.test.ts                   (17 tests)  5ms
 ✓ src/lib/utils/cn.test.ts                       ( 5 tests) 13ms
 ✓ src/lib/stores/auth.test.ts                    ( 5 tests)  5ms
 ✓ src/lib/api/api.test.ts                        (22 tests) 15ms
 ✓ src/lib/components/ui/components.test.ts       (13 tests) 38ms
 ✓ src/lib/components/layout/layout.test.ts       ( 6 tests) 102ms

 Test Files  6 passed (6)
      Tests  68 passed (68)
   Duration  13.57s
```

### pnpm build 产物 (32 文件 / 276KB)

```
build/
├── favicon.svg                          282B
├── index.html                          1868B
└── _app/
    ├── env.js                            19B
    ├── version.json                      27B
    └── immutable/
        ├── assets/
        │   └── 0.rCAC8Mne.css         ~24K  (Tailwind v4 编译产物)
        ├── chunks/  (16 JS chunks)    ~165K
        ├── entry/   (2 JS)              5K
        └── nodes/   (8 JS)             75K
```

## 4 个核心面板截图 (playwright + 自建 SPA-fallback server)

dev/build 阶段都验证过路由可访问. 截图脚本: `pnpm dev` (vite 5174) 或 `node scripts/serve.mjs 5174 build/` (production). 然后 playwright `browser_navigate` + `browser_take_screenshot` fullPage.

5 张截图位于 `qianxun/src/daemon/ui/screenshots/`:
- `00-home-redirect.png` — `/` 触发 goto('/llm') 后的落地页 (=LLM 页)
- `01-llm.png` — Sidebar + TopBar + LLM Providers 标题 + 刷新/新增按钮 + Empty 状态 + Token 弹框
- `02-skills.png` — Skills 标题 + 重载全部按钮
- `03-mcp.png` — MCP Servers 标题 + 新增按钮 + stdio/HTTP transport 标签
- `04-tools.png` — Tools 标题 + 共 N 个 badge

注: 截图时未配置 daemon (本地无运行), 所以数据列表是 Empty 状态. 这与 Stage 7a 实际运行行为一致 (daemon 端 API 还没接, 返空数据).

## Git Commits

```
129bdf0 feat(webui): Stage 7a - SvelteKit SPA + 4 核心管理面板 (LLM/Skills/MCP/Tools)
```

1 个新 commit, 包含 64 个新增文件.

## Notes for verifier

### 1. 跨子项目一致性

- **不要从 qianxun-desktop 复制 sse/ipc/i18n** — 那些是 Tauri 专用. Web Console 用纯 HTTP fetch (`fetchWithAuth`).
- **Schema (TypeScript interface)** 跟 daemon 端 Rust struct 字段名一致 (id/provider/model/has_key/active 等), 见 `src/lib/types/api.ts`.
- **端点路径** 跟 `_shared-contract.md` §3.1.1 严格一致 (`/v1/llm/providers/*`, `/v1/skills`, `/v1/mcp/servers/*`, `/v1/tools/*/invoke`).

### 2. 鉴权 (Stage 7a 简化方案)

- daemon 启动时随机生成 admin token, 打 stderr (由 sister task `daemon-stage7a-llm-and-serve` 实现).
- Web Console 首次访问任意路由 → 弹 `TokenDialog` → 用户粘贴 → 写 localStorage (`qianxun_admin_token`) → 后续所有 `/v1/*` 请求自动带 `Authorization: Bearer`.
- 401 响应 → `authStore.clear()` + 派发 `qianxun:auth:failed` 自定义事件 → `+layout.svelte` 监听弹框.
- 缺密码框: 跟 01b §6.1 一致, Stage 7a 简化; Stage 7b 加 password.

### 3. 与 sister task `daemon-stage7a-llm-and-serve` 的契约

Web UI 调用约定:
- `GET /v1/llm/providers` → `{ providers: LlmProviderSummary[] }`  ← key 不返
- `GET /v1/llm/providers/{id}` → `{ provider: {...} }` ← key 不返
- `POST /v1/llm/providers` body: `LlmProviderConfig` (含 `api_key` 写 keyring)
- `PUT /v1/llm/providers/{id}` body: `LlmProviderConfig`
- `DELETE /v1/llm/providers/{id}` → 200
- `POST /v1/llm/providers/{id}/activate` → 200
- `POST /v1/llm/providers/{id}/test` → `{ ok, latency_ms, error? }`
- `POST /v1/skills` → `{ status: "reloaded", count: N }`
- `POST /v1/skills/{name}/toggle` → `{ status: "enabled" | "disabled" }`
- `POST /v1/mcp/servers` body: `McpServerConfig` (transport: stdio|http)
- `DELETE /v1/mcp/servers/{id}` → 200
- `POST /v1/mcp/servers/{id}/test` → `{ ok, tools: [...], error? }`
- `POST /v1/tools/{name}/invoke` body: `{ arguments: {...} }` → `{ output, elapsed_ms, error? }`

### 4. 调试 / 启动

```bash
# Dev (热重载)
cd qianxun/src/daemon/ui
pnpm dev
# 浏览器开 http://127.0.0.1:5174/, 输入 daemon stderr 输出的 token

# Production preview (build 静态文件)
pnpm run build
node scripts/serve.mjs 5174 build/
# 浏览器开 http://127.0.0.1:5174/

# 全部测试
pnpm test:unit

# Type/lint
pnpm run check
```

### 5. 已知限制 (Stage 7a 范围内, 不修)

- **daemon 端 API 假定已实现** (sister task). 端点 / 字段名契约已锁定, Web UI 按契约对接.
- **svelte-i18n** 在 package.json 提到了 (跟 desktop 同步), 但**没有引入路由** — Stage 7a 暂用中文硬编码, Stage 7b 完整.
- **没有 chat 流** — 那是 Stage 7c + Tauri 桌面范围.
- **Mobile 响应式** — Stage 7c 范围, 现在桌面优先.
- **Dialog 组件** 自己写的简化版 (Stage 7a 不引 bits-ui), Stage 7b 可切换到 shadcn-svelte 完整版.

### 6. 错误排查提示

- `pnpm install` 失败: 网络受限, 用 `pnpm config set registry https://registry.npmmirror.com`.
- `pnpm test` 失败 + "snippet is not a function": 升级 `@testing-library/svelte` 到 5.3.x (package.json 已锁).
- `pnpm run build` 失败 + "Cannot use 'state' as a store": 局部变量名跟 Svelte 5 runes 冲突, 重命名为 `daemonState` 之类 (TopBar 已修).
- dev server 端口冲突: vite.config.ts 改 `port: 5174` 或 CLI `pnpm dev --port 5175`.

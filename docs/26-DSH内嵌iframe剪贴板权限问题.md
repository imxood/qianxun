# 26 · DSH 内嵌 iframe 的剪贴板权限问题（复制图片失败）

> 结论先行：DSH 被千寻以 **跨源 iframe** 嵌入 WebView2，Chromium 的 Permissions-Policy 默认把 `clipboard-write` 的 allowlist 限死为 `'self'`，于是 iframe 里的 **整个 Clipboard API 被禁**——DSH 右键「复制图片」必失败。修复只需给 iframe 加 `allow="clipboard-write"`。

## 1. 现象

- DSH Web GUI 里对会话图片右击 → 「复制图片」→ 右上角 toast：**「无法复制，请检查内容是否可用后重试」**。
- 同菜单里「复制文本」正常。
- 独立跑 DSH（`dsh web`，页面在顶层）时复制图片**正常** ⇒ 只在「千寻内嵌」场景坏。

## 2. 根因（实测）

千寻把 DSH 放进 iframe：

- `src/features/dsh/DshPage.svelte:111-125`：`<iframe title="DSH" src={dshUrl} sandbox="allow-scripts allow-same-origin allow-forms allow-downloads allow-popups" allow="clipboard-write" .../>`
- `src/features/dsh/DshPage.svelte:74`：`dshUrl = harness.proxyUrl`，即千寻 Rust 回环代理 `http://127.0.0.1:<proxyport>`。
- 顶层文档 origin = `tauri.localhost`（release）/ `localhost:5190`（dev），iframe origin = `127.0.0.1:<port>` ⇒ **跨源**。

跨源 iframe 默认继承顶层文档的 Permissions-Policy，而 `clipboard-write` 的默认 allowlist 是 `'self'` ⇒ iframe 文档**无权写剪贴板**，调用 `navigator.clipboard.*` 直接抛：

```
NotAllowedError: Failed to execute 'writeText' on 'Clipboard': The Clipboard API has been
blocked because of a permissions policy applied to the current document.
See https://crbug.com/414348233 for more details.
```

（该报错只在 WebView2 + 跨源 iframe 的上下文里出现；普通 Chromium 里同样的跨源 sandbox iframe 实测可写，见第 6 节。）

DSH 侧的调用链（安装包内 `lib/client.js`，行号对应 `@deepseek-ai/dsh-client-ui-*`）：

- `dsh-client-ui-file-actions/lib/client.js:1815`：`await navigator.clipboard.write([new ClipboardItem({ "image/png": png })])` ← **被拒的那一步**。
- `dsh-client-ui-file-actions/lib/client.js:1786-1816` `copyImage`：先 `read`（blob URL → `fetch` → `blob`），非 PNG 再走 canvas 转码，然后写剪贴板。
- `dsh-client-ui-context-menu/lib/client.js:1080-1085`：`Promise.resolve(run?.()).catch(() => { this.failed(selected.action.failureLabel); })` ⇒ **异常被吞，不写 console**，界面只剩 `failureLabel`「无法复制，请检查内容是否可用后重试」。这就是「看不到真实报错」的原因。

### 为什么「复制文本」能用

DSH 的文本复制走 `@deepseek-ai/dsh-client-ui-primitives` 的 `writeClipboard` / `useClipboardCopy`（sidebar、trajectory、chat、user-questions 等处调用），其实现打包在 `dsh-web-frontend/dist/assets/index-*.js` 里，与图片路径（`ClipboardItem`）**不是同一条实现路径**——文本走 legacy 复制通道，图片走异步 Clipboard API，只有后者被 Permissions-Policy 拦住。

## 3. 修复

一行（`src/features/dsh/DshPage.svelte:116`）：

```svelte
allow="clipboard-write"
```

要点：

- `allow`（Permissions-Policy 授权）与 `sandbox`（iframe 能力沙箱）是**两套机制**；`sandbox` 里没有 clipboard 项，加 `allow-same-origin` 等**不会**顺带获得剪贴板权限。
- 主窗（`src/App.svelte:115`）与独立窗口（`src/StandaloneWindow.svelte:103`）都渲染 `DshPage` ⇒ 一处修改两处生效。
- **策略变更必须让 iframe 重新导航才生效**：改属性后不刷新页面，行为仍是旧的（实测：改 `allow` 后需 `f.setAttribute('src', f.getAttribute('src'))` 强制重载）。

## 4. 验证（可复现）

环境：千寻 debug 构建（`pnpm dev`，WebView2 CDP 端口 10222）。跨源 iframe 在 CDP 里是**独立 OOPIF target**（`/json/list` 中 `type === "iframe"`），要用自建 WebSocket CDP 客户端单独连。

A/B 对照（同一张真实会话图片，真鼠标事件）：

| iframe `allow`              | 右击→「复制图片」后的 toast            |
| --------------------------- | -------------------------------------- |
| 无（修复前）                | **无法复制，请检查内容是否可用后重试** |
| `clipboard-write`（修复后） | **已复制**                             |

探针脚本（`.tmp/probe/clipboard-webview2/`，不入库）：

- `e2e-copy-image.ts`：滚到会话图片 → `Input.dispatchMouseEvent` 右击 → 找 `role="menuitem"` 的「复制图片」→ 左击 → 轮询 toast 文案。
- `toggle-allow.ts on|off`：增删 iframe 的 `allow` 并强制重载，用于 A/B。
- `recheck2.ts` / `recheck.ts`：复刻 `BrowserFileActions.read` + `copyImage` 全链路（blob URL → fetch → `ClipboardItem` → `clipboard.write`），区分「无手势 → Write permission denied（预期，Chromium 要求 transient activation）」与「有真实点击 → OK」。
- `imgs.ts`：列出 DSH iframe 里的 `<img>`、按钮、`input[type=file]`。

## 5. 顺带确认的事实

- 顶层文档（千寻自己的文档）里 `writeText` / `write([ClipboardItem])` 全部 OK ⇒ **WebView2 本身支持写图片**，不是运行时缺陷。
- DSH 侧图片 `src` 是 `blob:` URL（conversation 插件 `HistoricalImageCache.loadCanonical` 用 `URL.createObjectURL` 生成），`new URL()` 与 `fetch()` 都合法 ⇒ URL 分支不是失败原因。
- 没有用户手势时写剪贴板会报 `NotAllowedError: Write permission denied`（Chromium 要求 transient activation）；DSH 的右键菜单点击天然满足该条件。

## 6. 踩坑清单

1. **别把 `sandbox` 当 Permissions-Policy**：两者正交，`sandbox="allow-same-origin"` 不等于拿到剪贴板权限。
2. **别在普通 Chromium 里验 Permissions-Policy 假设**：跨源 sandbox iframe 在 Playwright chromium 里实测能写图片（Playwright 会授予权限），必须回 WebView2 + 千寻 iframe 上下文验。
3. **别相信「菜单报错文案」**：DSH context-menu 插件吞异常，failureLabel 是唯一线索；要拿真实报错得自己复刻链路。
4. **改 Permissions-Policy 必须重新导航 iframe**：属性变更对已加载文档不生效。
5. **release 实例验不了 DSH 直连**：DSH web 鉴权要 `?token=`（`dsh-client-connection` 的 `authorizeIndex`，launchToken 进程级内存随机、不落盘），只有千寻前端内存里持有 ⇒ 复现要走千寻 debug 构建（数据根 `%USERPROFILE%\.qianxun_dev`，与 release 的 `.qianxun` 隔离）。

## 7. 影响面

修复只改千寻侧（`src/features/dsh/DshPage.svelte`），**未改动 DSH 第三方包**（`C:\Users\maxu\.qianxun\dsh-runtime`，`@deepseek-ai/dsh` 0.2.2-alpha.3）与任何 release 进程。已装的 release 千寻（`C:\Users\maxu\AppData\Local\qianxun\qianxun.exe`）需要重新构建安装后才会带上这一行修复。

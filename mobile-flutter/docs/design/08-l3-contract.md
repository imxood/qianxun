# L3 契约规格：`mobile.js` / `mobile.css` 能做什么

> 全部来自 `dsh-mobile/src/client.ts` 与 `src/mobile-guide.ts` 的源码，
> 每条带 `<文件>:<行号>`。这是写 `mobile.js` 的地基，不含猜测。

---

## 0. 结论：L3 能独立完成手机端重建

`mount()` 拿到的 context 是（`client.ts:27-32`）：

```ts
interface MobileExtensionContext {
  readonly document: Document
  readonly request: (path: string, init?: RequestInit) => Promise<Response>
  readonly root: HTMLElement
  readonly window: Window
}
```

**`request` 是已认证的 fetch**（`client.ts:2367` 传入 `mobileRequest`）。
它在页面同源内跑，带设备凭据。

这意味着：

> **L3 可以直接调 `POST /api/session/list`、`/api/session/prompt` 等全部 RPC。**
> **协议层不需要我们写一行。**

这是方案 B 成立的决定性一点，也是我之前判断「协议风险最大」的解药。

---

## 1. 两套入口，别搞混

### 1.1 `window.dshMobile.register(mount)` — 旧版，**我们要用这个**

```js
window.dshMobile.register(({ document, request, root, window }) => {
  // …你的手机端界面
  return () => { /* 清理 */ }
})
```

- `client.ts:71` 声明，`client.ts:2337` 实现
- `root` 是一个**独立的 `<div>`**（`client.ts:2366`
  `data-dsh-mobile-extension`），挂在 `document.body`
- 返回值可以是清理函数（`client.ts:34`：
  `type MobileExtensionMount = (ctx) => void | (() => void)`）

⚠️ **关键限制**：`register` 的 `root` **不会替换 DSH 的界面**。
它是在 DSH 页面**之上/之下**追加一个自己的层。

所以「L3 重建界面」实际是：
**把我们自己的界面挂进 `root`，同时用 CSS 把 DSH 原版界面隐藏。**

（隐藏方式见 §4，`dsh-mobile` 自己的 `MOBILE_LAYOUT_STYLES`
已经把 DSH 的三栏搬成了 drawer/details，所以要藏的是那个已搬好的布局。）

### 1.2 `window.dshMobile.define(definition)` — 新版，带 manifest

```js
window.dshMobile.define({
  apiVersion: 1,
  id: 'my-ext',                 // ^[a-z][a-z0-9-]{0,63}$
  activate(api) { … }           // 同步/异步均可
})
```

`client.ts:62-66` 定义，`client.ts:2330-2335` 实现。

校验很严（`client.ts:2331`）：`apiVersion !== 1`、id 不合法、
`activate` 非函数 → **静默 return，不报错**。⚠️ 写错没有任何提示。

`activate` 返回清理函数，或 `void` / `Promise`。

---

## 2. `define` 的 `api` 形状

`MobileClientApi`（`client.ts:42-61`）：

```ts
interface MobileClientApi {
  readonly host: {
    invoke(action: string, input: unknown): Promise<unknown>   // 调电脑端 host.mjs
    fetch(path: string, init?: RequestInit): Promise<Response> // 调电脑端 route
    assetUrl(path: string): string                             // 版本绑定的资源地址
  }
  readonly ui: {
    registerSurface(surface: MobileSurface): () => void
    …
  }
  readonly signal: AbortSignal   // 扩展被卸载时 abort
  readonly document: Document
  readonly window: Window
}
```

### 2.1 `api.ui.registerSurface` — 六个落点

`client.ts:35` 定义 placement，`client.ts:2139-2142` 给出每个落点的定位：

| placement | 位置 | 用途 |
|---|---|---|
| `page` | 全屏层（`position:absolute; inset:0`） | 一个完整的手机页面 |
| `overlay` | 同上 | 浮层 |
| `sidebar-action` | `fixed; top: safe-area+8px; left:8px` | 侧栏左上角的悬浮动作 |
| `header-action` | `fixed; top: safe-area+8px; right:8px` | 顶栏右上角的悬浮动作 |
| `composer-dock` | `fixed; bottom: safe-area+8px; 居中` | 输入栏上方的 dock |
| `settings-section` | `fixed; bottom: safe-area+72px; max-height:40vh` | 设置页的一段 |

`page` / `overlay` 的容器默认 `hidden = true`（`client.ts:2247`），
`sidebar-action` 等则可见。

⚠️ 这些落点是 **`position: fixed` + `z-index: 1100`**，
而 DSH 主界面在 z-index 更低的层。所以 `page` 落点天然盖住全屏。

**这给了我们一个更干净的方案**（比「隐藏 DSH 界面」好）：

> 用 `define` + `registerSurface({ placement: 'page' })`
> 挂我们自己的全屏页面。需要看 DSH 原版时切走，需要我们的界面时切回来。

但注意 `page` 层 `pointer-events: auto` 且 `background: var(--dsw-alias-bg-layer-1)`，
所以它**完全遮挡**下方。切换由我们自己的代码控制。

### 2.2 `api.host.*` — 调电脑

- `invoke(action, input)` → `host.mjs` 里 `api.action('名字', { input, run })`
- `fetch(path, init)` → `host.mjs` 里 `api.route({ method, path, handle })`
- `assetUrl(path)` → `assets/` 下与当前版本绑定的 URL

**什么时候才需要它**：读电脑本地文件、执行命令、访问硬件。
纯粹的 DSH 数据（会话、消息、工具调用）**用 `request` 调 RPC 就够了**。

---

## 3. 数据通道：直接调 RPC

`request` 已认证，所以 `mobile.js` 里就是普通 fetch：

```js
const res = await request('/api/session/list', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    type: 'client-request',
    rpcId: crypto.randomUUID(),
    method: 'session/list',
    payload: { args: { _request: {} } },   // ⚠️ session/list 用 _request
  }),
})
const body = await res.json()
const items = body.result.ok ? body.result.value.items : []
```

**线格式实测依据**（`docs/api/dsh-remote-api.md` + `docs/probe/session-list-item.json`）：

- 端点 `POST /api/<ns>/<method>`
- 信封三层：`{type:'server-response', rpcId, result:{ok, value|error}}`
- ⚠️ **`payload.args` 是唯一合法键**，`session/list` 的参数恰好叫 `_request`
- `session/page` / `session/follow` 等用真实参数名

**⚠️ 三个必须记住的坑（都是我自己踩过的）**

| 坑 | 后果 | 正确做法 |
|---|---|---|
| 只读 `start`/`end` 帧 | 流式文字完全不显示 | `chunk` 帧里 `chunk` 是**压缩 record 流**（`text-chunks`/`reasoning-chunks`/`tool-call-chunks`），一个 record 里有 N 个片段。需按 `type` 分派并遍历 `texts[]` |
| `revision` / `index` 跳号 | **断流** | 必须严格 +1。`revision` 跳号服务端抛 `RemoteStreamCarrierError`；`index` 不连续该 attempt 作废 |
| 用 `end.index` 推断轮次 | 数错 | `end.outcome.seq` 才是权威；`end.index` 是「本帧代表的 chunk 帧数」 |

详见 `mobile-flutter/docs/api/dsh-remote-api.md` §2.1.5。

---

## 4. CSS：`mobile.css` 的加载与作用范围

`client.ts:2117` 插入一个 `<style data-dsh-mobile-custom>`。
它是**全局的**，没有 scoping —— 我们的选择器会影响 DSH 自己的元素。

**所以规则：**

1. **所有自定义选择器必须带自有前缀**（下面用 `qx-`）
2. 要改 DSH 原生元素时，用 `.dshm-shell` 之类的高特异性前缀
   （`dsh-mobile` 自己的样式就是这么做的，`mobile-layout.ts:135-186`）
3. ⚠️ **不要写 `* { }` 或 `div { }` 这类全局规则**

**这份 CSS 该做的第一件事**（也是 L3 方案的关键一步）：

```css
/* 隐藏 DSH 自己的布局层，让 root 里的我们成为唯一界面 */
.dshm-shell { display: none !important; }
```

⚠️ 但这样会**丢掉 DSH 的转录内容**。所以正确顺序是：

1. 先用 L1（`dsh-mobile` 提供的移动布局）跑通窄屏
2. 确认哪些内容我们**必须自己重建**（导航、工具卡、审批）
3. 再逐块替换，而不是一次全藏

**不建议一上来就 `.dshm-shell { display:none }`。**
那是「重写」而不是「重排」，会把 141 个 RPC 的 UI 工作量全揽过来。

---

## 5. 从 `mobile-layout.ts` 抄来的 20 条硬约束

这些是别人踩过坑写下的，全部来自 `src/mobile-layout.ts`：

| # | 约束 | 行号 | 抄不抄 |
|---|---|---|---|
| 1 | 收起 56px，展开 `min(88vw,340px)` | `:140-141` | ✅ |
| 2 | 详情面板 `min(94vw,460px)` + `translateX(104%)` | `:146-148` | ✅ |
| 3 | 遮罩 `rgb(15 23 42 / 40%)`，180ms | `:149-150` | ✅ 40% 而非 50% |
| 4 | `textarea { font-size: 16px }` | `:153` | ✅ **低于 16px iOS 会自动放大** |
| 5 | `table/pre { overflow-x: auto }` | `:154-155` | ✅ |
| 6 | 触控目标 ≥ 48px + `touch-action: manipulation` | `:169` | ✅ |
| 7 | ≤600px 时选项按钮改 2 列、44px 高 | `:171-172` | ✅ |
| 8 | `prefers-reduced-motion` 全关动画 | `:185` | ✅ |
| 9 | **软键盘抑制**：点菜单后 700ms 内 `blur()` | `:232-270` | ✅ **最容易漏的一条** |
| 10 | 选中会话后关抽屉 + `blur()`，用 `setTimeout(0)` 让行 handler 先跑 | `:274-290` | ✅ |
| 11 | 控制器用**模块级单例**，重连不重置折叠态 | `:67-71`、`:325` | ✅ |
| 12 | `env(safe-area-inset-*)` 全方位留白 | `:140`、`:152` | ✅ |
| 13 | `padding-top: env(safe-area-inset-top)` 显式写 | `:140`、`:146` | ✅ |
| 14 | `.dshm-shell header { padding-left: 52px }` 给 rail 让位 | `:152` | ✅ |
| 15 | `[data-disclosure-row] > * { overflow-wrap: anywhere }` | `:157-158` | ✅ |
| 16 | ≤420px 时 `[data-context-fields]` 转单列 | `:164` | ✅ |
| 17 | `will-change: width` + 240ms `cubic-bezier(.22,1,.36,1)` | `:140` | ✅ |
| 18 | 关闭态用 `visibility`/`pointer-events` 而非仅 `opacity` | `:142`、`:148` | ✅ |
| 19 | 提问卡 `max-height: min(68dvh,520px)` | `:162` | ✅ 用 `dvh` 不是 `vh` |
| 20 | 会话标题投影到 `document.title` | `:223-230` | ❌ 手机无标签页 |

---

## 6. 我们要额外解决的 5 个（`dsh-mobile` 没做）

| # | 问题 | 它的做法 | 我们的做法 |
|---|---|---|---|
| 1 | **工具输出 2MB** | 没碰，靠 `overflow-wrap` 兜底 | 两层披露 + 截断 + 「全文」入口 |
| 2 | **审批卡** | 只调了提问卡的 CSS | 重建：命令全文 + 参数 + 允许/拒绝 |
| 3 | **diff 视图** | 没碰 | 行级 diff + 汇总 |
| 4 | **子 Agent Team** | 没碰 | chip `👥 N 在跑` + sheet |
| 5 | **拖拽排序** | 保留桌面版 | **不做**，改「⋯ → 移到顶」 |

---

## 7. 第一步该写什么

按 `07-webview-vs-flutter.md` §6 的七步，**第一步不是写界面，是验通道**：

```js
// .tmp/probe-mobile-js/probe.js
window.dshMobile.register(async ({ request }) => {
  const res = await request('/api/session/list', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      type: 'client-request', rpcId: 'probe',
      method: 'session/list', payload: { args: { _request: {} } },
    }),
  })
  const body = await res.json()
  console.log('[probe] ok =', body.result?.ok, 'items =', body.result?.value?.items?.length)
})
```

**放在真机上跑，验证三件事：**

1. `request` 是否真的已认证（`result.ok === true`，不是 401）
2. `session/list` 能否从 `mobile.js` 拿到全部 393 条
3. `projections.values` 的 20 个字段是否与探针一致

**若这三件成立，L3 方案的地基就是实的**，后面每一步都只是 UI 工作。
若第 1 件不成立（要额外加凭据），方案 B 就有实质风险，要立刻回退到 C。

**这一步不做，后面 6 步都是空中楼阁。**

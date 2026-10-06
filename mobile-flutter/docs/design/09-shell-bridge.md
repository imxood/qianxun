# 壳 ↔ 页面 桥接设计（方案 B 的中间层）

> 方案 B = 原生壳（Flutter）+ DSH 页面（L3 定制）。
> 中间需要一个明确的桥接约定，否则两层各自猜对方的形状。

---

## 0. 分层与职责

```
┌─────────────────────────────────────────────┐
│ Flutter 壳（原生）                          │
│  · 设备列表 / 扫码配对 / mDNS 发现            │
│  · 凭据存储（Keystore）                     │
│  · WebView 宿主 + TLS 固定 + 返回键          │
│  · 底部 Surface 切换（对话 / 远程桌面）      │
├─────────────────────────────────────────────┤
│ WebView 边界                                 │
│  · 同源（精确 Origin）才通信                  │
│  · 出站走 mobile.js 契约，内注入 window.dshMobile
├─────────────────────────────────────────────┤
│ DSH 页面（L3：mobile.css + mobile.js）      │
│  · 导航 / 工具卡 / 审批 / diff               │
│  · 数据全部走 request（已认证）               │
└─────────────────────────────────────────────┘
```

**原则：壳不碰界面，页面不碰设备。**
- 壳负责「能不能连上」——TLS、凭据、配对、发现
- 页面负责「连上之后长什么样」——布局、交互、渲染
- 通信只在 WebView 边界发生，且只走一个自定义事件名

---

## 1. 壳 → 页面：只有一个事件

页面需要从壳拿到的东西只有一类：**「当前是哪台设备、什么地址」**。
其余（会话、消息、工具调用）页面自己调 RPC。

```js
// 壳在页面就绪后注入
window.__QX_SHELL__ = {
  origin: 'https://192.168.20.2:3443',
  hostname: 'DESKTOP-CLTN8CB',
  // 不下发 token —— 页面用 request 已经是认证态
}
```

⚠️ **不下发凭据。** `dsh-mobile/README.zh-CN.md:53` 明确：
Bridge「不暴露 Cookie、设备令牌、配对密钥、CA 私钥或任意 Android API」。
我们沿用同一条边界。

**壳侧发送 Surface 切换**（唯一的下行事件）：

```js
window.dispatchEvent(new CustomEvent('qx:surface', {
  detail: 'conversation' | 'desktop'
}))
```

---

## 2. 页面 → 壳：三个自定义事件

不用 JS 注入（`javascript:` 字符串转义地狱），用 `addJavascriptInterface`
的反向 —— Flutter 的 `WebViewChannel` / `AndroidWebViewChannel` 走
`postMessage`。但 **DSH 页面不是我们写的**，所以只能在 `mobile.js` 里桥接。

```js
// mobile.js 里
const send = (type, payload) => {
  // WebMessageListener 路径：DSH 的 dsh-mobile 已建好桥
  window.dshMobileNative?.postMessage?.(JSON.stringify({ type, payload }))
  // 回退：Android 注入的 qxShell
  window.qxShell?.postMessage?.(JSON.stringify({ type, payload }))
}
window.addEventListener('qx:surface', (e) => {
  // 页面自己切，或通知壳切
})
```

**但这是可选的。** 优先方案是**页面全自足**：
Surface 切换由壳的底部 tab 触发，壳只是 `show/hide` WebView 两侧的原生视图。
这样页面根本不需要知道「有壳」。

> **我的建议：第一版不做双向桥。**
> 壳只负责把 WebView 显示出来，切换 Surface 用原生视图显隐。
> 等真的有「页面要主动唤起原生能力」的需求（扫码、分享、文件选择）再加桥 ——
> 那时用 `dshMobile` 已有的 `files.pick` / `camera.capture` / `share`
> （`NativeBridge.kt`，README `:55`），不必自己造。

---

## 3. 壳层要新增的模块

| 模块 | 职责 | 复用现有代码 |
|---|---|---|
| `connection_store.dart` | 设备列表持久化 | ✅ 已有（`lib/data/store/`） |
| `connections_controller.dart` | 设备状态探活 | ✅ 已有，但要改：`info` 探活用 RPC |
| `WebViewHost` | 加载 DSH 页面 | ❌ 新增 |
| `TlsPinning` | 证书固定 | ❌ 新增（照抄 `dsh-mobile` 的 `PinnedTls.kt` 思路） |
| `BackKeyPolicy` | 返回键 = WebView 历史优先，栈空才退出 | ❌ 新增 |
| `SurfaceSwitcher` | 底部 tab 切两个 Surface | ❌ 新增 |
| `LanDiscovery` | mDNS 扫电脑 | ❌ 新增（照抄 `LanDiscovery.kt`） |
| `Pairing` | 扫码 / 粘贴链接兑换 token | ⚠️ 部分有（`pair_url.dart`） |

---

## 4. 返回键语义（照抄 `dsh-mobile`）

`README.zh-CN.md:31`：**「系统返回键先处理同源 WebView 历史」**。

具体顺序：

1. 有未完成的 sheet / 弹层 → 关闭它
2. WebView `canGoBack()` → `goBack()`
3. 在 DSH 页内 → 回**设备列表**（不是退出 App）
4. 在设备列表根页 → 才交给系统（退出）

⚠️ 第 3 条是用户明确要的：「主页面内部 可以有一个 返回的按钮, 返回到 设备列表,
DSH是独立的页面」。**DSH 页是独立页面，不在设备列表的 push 栈里** ——
它是「选中设备后进入的另一个 Surface」，返回时横向切回设备列表。

---

## 5. 代码资产清算

方案 B 下，`mobile-flutter/lib/` 的命运：

| 目录 | 处置 | 理由 |
|---|---|---|
| `lib/data/dsh/`（协议层，~90KB） | **保留**，壳用它做探活与配对 | RPC 信封、WS mux、模型解析都是实测过的 |
| `lib/data/models/` | **保留** | 探针与测试仍要用 |
| `lib/core/qx_log.dart` | **保留** | release 不写 logcat，只能靠它 |
| `lib/app/theme/` | ⚠️ **部分保留** | `QxSemantic` 的对比度闸门仍有用；`SignalSpine` 作废 |
| `lib/features/connections/` | **改造** | 设备列表保留；会话列表部分不要 |
| `lib/features/sessions/` | ❌ **作废** | 页面由 L3 做 |
| `lib/features/workspace/` | ❌ **作废** | 同上 |
| `lib/shared/widgets/` | ⚠️ 挑留 | 通用件（`primitives.dart`）可能还用 |
| `test/` 177 个 | ❌ UI 部分作废，协议层留 | 见下 |

**测试处置原则**：作废 UI 测试**不是删掉**，而是移到
`test/legacy_flutter_ui/` 并在文件头注明「方案 B 下已由 L3 接管，
保留作对照，不是活门禁」。这样：
- 活门禁只跑协议层测试（约 60 个）
- 想看「原来的 Flutter UI 长什么样」还能跑
- 不会有人以为「删测试」是在掩盖问题

---

## 6. 风险与退路

| 风险 | 概率 | 退路 |
|---|---|---|
| `request` 在真机上不认证 | 低（源码已确认） | 回退方案 C |
| L3 写不动 / 上游槽位变 | 中 | 退 L1（只 CSS），或 C |
| WebView 性能（大转录） | 中 | L3 做虚拟滚动；仍不行则 C |
| 桥接需求反复出现 | 中 | 按 §2 逐步加，不预造 |

**每一档都有明确退路**，这是选 B 的前提条件。

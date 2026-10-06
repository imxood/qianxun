# 方案分析：WebView + 重写样式，还是 Flutter 原生重做

> 结论先说：**如果「只重写样式就能达成」成立，WebView 方案更好，且好得多。**
> 但我查完 DSH 的插件与槽位机制后，条件**不成立** —— 不是因为做不到，
> 而是因为「只重写样式」这个前提把 DSH 已有的能力算漏了。
>
> 真正的最优解是第三种：**壳原生 + DSH 页面重排重写，插在 DSH 插件层**。

---

## 1. 先验事实：DSH 有三层扩展机制

这是整份分析的地基，全部来自源码。

### 1.1 插件可以「整体替换 root 布局」

`packages/client/ui-layout/src/client/index.ts:53-120` 定义了 root 的子槽位：

| 槽位 | 类型 | 语义 |
|---|---|---|
| `sidebar` | `single` / `root` | 整条左栏。**OCCUPIED** by `ui-sidebar/SidebarRoot` |
| `main` | `keyed` / `root` | 中栏，`conversation` 是保留键 |
| `rightbar` | `single` / `root` | 右栏 |
| `shell.bottom` | `single` / `root` | 三栏之下通栏底部 |
| `shell.overlay` | `list` / `root` | 全局浮层 |
| `shell.leading` | `single` / `root` | 窗口左上角 chrome |

注释里写得很明确（`index.ts:60-67`）：

> registering here **replaces the navigation column outright rather than
> adding to it**, and the seats it declares disappear with it.

即：**槽位是「排他渲染权」**。占了这个槽，原组件自动消失。

`dsh-mobile/src/mobile-layout.ts:337-345` 就是这么干的：

```ts
ctx.slots.register({
  name: 'root',
  children: {
    sidebar:  { kind: 'single', scope: 'root' },
    conversation: { kind: 'single', scope: 'session-maybe' },
    details:  { kind: 'single', scope: 'session' },
    'shell.overlay': { kind: 'list', scope: 'root' },
  },
}, props => createElement(MobileAppFrame, { ...props, controller }))
```

**它没有改 DSH 源码，也没有 fork。** 它换掉了 root 布局的实现，
但 `sidebar` / `conversation` 的**内容组件还是 DSH 原版的**。

### 1.2 用户级定制文件已经存在

`cordis.patch.yml:24-25` 声明了两个用户文件：

```yaml
customCssFile: !!js dshHomePath('mobile-access/mobile.css')
customScriptFile: !!js dshHomePath('mobile-access/mobile.js')
```

`src/mobile-guide.ts:60-79` 写明了契约：

- `mobile.css` — 手机端样式
- `mobile.js` — `window.dshMobile.register(({ root }) => { ... })`，返回清理函数
- `extensions/<id>/host.mjs` — 电脑端 Node 代码，可 `api.action` / `api.route`
- `extensions/<id>/mobile.js` — `window.dshMobile.define({ apiVersion, id, activate(api) })`
- `api.host.invoke('动作名', 输入)` — 从手机调电脑
- `api.host.fetch('/路径')` — 调 route
- `api.host.assetUrl('相对路径')` — 版本绑定的资源

**所以「重写样式」这个能力，DSH 已经内建了，而且带版本化、热更新。**

### 1.3 Android 壳提供的能力边界

`apps/mobile/android/.../NativeBridge.kt`（50KB）经
`androidx.webkit` WebMessage 暴露：`files.pick` / `camera.capture` /
`share` / `clipboard.read` / `clipboard.write`。

安全约束（README `:53-55`）：每条消息校验精确顶层 Origin + `isMainFrame`，
**绝不用 `addJavascriptInterface`**；入站 1 MiB、二进制 8 MiB、回复 12 MiB 上限。

图片限制：仅 PNG/JPEG/WebP/GIF，单文件 ≤ 8 MiB。

---

## 2. 你的条件判断，我完全同意

> 「如果说完全基于 webview 基于已有的 dsh web端, 仅仅是重写样式就能实现,
> 那么这个方案肯定是更好的」

**这句话是对的，而且理由比「省事」强得多：**

| 维度 | WebView 重写样式 | Flutter 原生重做 |
|---|---|---|
| **协议** | **零成本**。141 个 RPC 方法、WS mux、事件流全在 DSH 里 | 我要重写并重新验证全部。前几轮光验证 `requestId` 去重、assistant-stream 压缩就花了 3 次提交 |
| **功能覆盖** | **自动 100%**。DSH 加新功能，手机立刻有 | 每个新功能都要我实现一遍，永远滞后 |
| **维护** | 跟随上游 | 永久分叉 |
| **改版成本** | 改 CSS | 改 Dart + 改协议层 |
| **一致性** | 与电脑端**像素级一致** | 必然漂移 |

第 4 条特别关键：我已经在这个项目上踩过 —— 探针输出从未入库、
`agentAvailable` 误用 393 条里 392 条为 false、
`contextPressure` 漏读导致 C10 阈值设在一个 0 命中的区间上。
**协议层每多写一行，就多一个「凭记忆写」的机会。**

---

## 3. 但「只重写样式」这个前提，算漏了三样

### 3.1 漏算了槽位机制

DSH 不是「一个 CSS 样式表」，是**一棵槽位树**。所以可选的粒度有四层：

```
L0  改 CSS 变量 / 媒体查询         ← 「重写样式」的字面含义
L1  换 root 布局，保留内容组件      ← dsh-mobile 现在做的
L2  替换某些槽位的内容组件          ← 换掉 sidebar，换成手机版导航
L3  写 mobile.js 重建整块界面       ← 保留协议，弃用 DSH UI 组件
```

**`dsh-mobile` 停在 L1。** 它的 `mobile-layout.ts` 换了布局，
但会话行、工具卡、审批卡、输入框全是 DSH 原版组件。

L2/L3 才是「为手机重新设计」——而它们在 DSH 里**已经有正式机制**，
不需要 fork 源码。这是比 Flutter 重写更好的地方，也是我之前没看到的。

### 3.2 漏算了交互模型的改造

有几件事**CSS 改不了**，因为它们是交互而不是样式：

| 桌面行为 | 为什么 CSS 无效 | 该在哪层解决 |
|---|---|---|
| 悬停显示菜单 | 触屏没有 hover | **L3**（`mobile.js` 绑定 `click`） |
| 拖拽排序工作区 | 触屏拖拽需要长按 + 阈值判定 | **L2/L3** |
| 右键菜单 | 触屏没有右键 | **L3** |
| Ctrl+Alt+N | 手机无物理键盘 | **L3**（补一个按钮） |
| 多列网格详情 | 这是布局，但 DSH 组件内嵌 | **L1 能压，但要改组件内样式** |
| 工具输出 2MB | 是性能与数据加载 | **L1/L2**（虚拟滚动 + 截断） |
| 软键盘遮挡 | 浏览器行为 | **L1 挡不住，要 L3 `blur()`** |

最后一条 `dsh-mobile` 是用 hack 解决的（`mobile-layout.ts:232-270`）：
DSH 的 composer 在 `mousedown` 时故意保留焦点，
手机上点命令菜单会弹软键盘挡住菜单。它用
`suppressKeyboardUntil = performance.now() + 700` + 主动 `blur()` 绕过去。

**这是个信号**：它证明「桌面组件的行为假设」和「手机的真实约束」是冲突的，
而这类冲突 **CSS 解决不了，只能在 L3 一处一处显式接管**。

### 3.3 漏算了壳层

你现有 RN 壳做的事，**不是 DSH 的事**：

- 设备列表（多台电脑、状态探活）
- 扫码配对（`ScanActivity.kt` + `QrDecoder.kt`）
- mDNS / DNS-SD 发现（`LanDiscovery.kt`、`NsdDiscovery.kt`）
- TLS 证书固定（`PinnedTls.kt`、`PairingTrust.kt`）
- Keystore 加密存储设备 token（`DeviceCredentialStore.kt`）
- 远程通道（cpolar / Tailscale Funnel / 自建 FRP）
- 返回键语义、下载策略、文件选择

这些**无论 DSH 页面怎么做都要做**，而且 `dsh-mobile` 已经踩完了坑
（`GatewayUrlPolicy.kt` 6.6KB 的 origin 规范化、
`SecureWebViewClient.kt` 的同源外链交给系统浏览器、
`VersionCompatibility.kt` 区分「App 过旧 / 插件过旧 / 协议不兼容」）。

**这一层应该照抄，不该重写。**

---

## 4. 三种方案的诚实对比

| | A · 壳 + DSH 原样 WebView | B · 壳 + L3 重建（推荐） | C · Flutter 全原生 |
|---|---|---|---|
| 协议 | 零成本 | 零成本 | 重写 + 重验 |
| 功能覆盖 | 100% 自动 | 100% 自动 | 手动追赶 |
| 手机端交互 | ❌ 桌面压扁 | ✅ 完全重做 | ✅ 完全重做 |
| 壳层能力 | ✅ 照抄 | ✅ 照抄 | ❌ 全部重写 |
| 上游跟随 | ✅ | ✅（`mobile.js` 可版本化） | ❌ 永久分叉 |
| 视觉与桌面一致 | ✅ 最高 | ⚠️ 主体一致，导航/工具卡重做 | ⚠️ 必然漂移 |
| 调试难度 | 低 | 中（要会写 JS 注入） | 高 |
| 已投入的 Flutter 代码 | 177 测试可复用 | 少量（协议层可复用） | 全部 |
| 主要风险 | 用不顺 | 依赖 DSH 槽位契约 | 协议层错 |

---

## 5. 我的建议：B

**理由三条，按权重：**

1. **协议层是最大的风险源，而 B 完全绕开它。**
   我此前 3 次提交都在验协议事实（`requestId` 去重、
   `assistant-stream` 压缩、`userQuestions` 覆盖边界）。
   每次验错都推翻了设计结论。B 方案里这些事实**由 DSH 保证**。

2. **「为手机重新设计」在 B 里是可实现的，而且有正式机制。**
   L3 的 `window.dshMobile.register(({ root }) => …)` + `mobile.css`
   就是为这件事设计的，且**热更新、不改 DSH 源码、可版本化**。
   这一点比 Flutter 重写强 —— Flutter 里我要自己实现版本化与热更。

3. **壳层不该重写。**
   TLS 固定、mDNS 发现、Keystore 存储、返回键语义
   —— 这些与「界面好不好用」无关，纯工程。
   `dsh-mobile` 已经把它们做成 23 个 Kotlin 文件 + 16 个测试文件，
   每个都有对应测试。照抄比重写快且稳。

**⚠️ B 的真实代价（我不粉饰）：**

- 我们已有的 177 个 Flutter 测试里，**协议层（约 60 个）可复用，UI 层（约 117 个）作废**
- 团队要同时懂 Flutter + TS/JS 注入
- 手机端的视觉与电脑端**不再像素级一致**（导航、工具卡、审批卡会重做）
- 依赖 DSH 的槽位契约 —— 若上游改了 `SlotMap`，我们要跟着改
  （但 `dsh-mobile` 已经这么活了，v0.3.12）

---

## 6. 如果选 B，具体怎么做

```
mobile-flutter/          ← 壳：原生，只做设备与连接
  lib/features/connections/   设备列表、扫码配对、mDNS 发现
  lib/features/shell/          远程桌面 Surface、WebView 宿主
  lib/data/dsh/                协议层（可保留，但要支持 host 扩展）

mobile-access/           ← 手机端定制，部署到 $DSH_HOME/mobile-access/
  mobile.css                   样式：窄屏断点、48dp 触控、16px 输入
  mobile.js                    L3：重建导航、工具卡、审批卡、diff
  extensions/<id>/             需要读电脑文件时才建
    host.mjs                   api.action / api.route
    mobile.js                  api.host.invoke / fetch
```

**移植顺序（每步都能独立验收）：**

| 步 | 做什么 | 验收 |
|---|---|---|
| 1 | 壳照抄：设备列表 + 扫码配对 | 能连上、能配对 |
| 2 | WebView 加载 DSH + `mobile.css` 窄屏适配 | 不用崩、可用 |
| 3 | `mobile.js` 重建导航（工作区树 + 会话） | 侧栏可用、返回键正确 |
| 4 | `mobile.js` 重建工具卡（两层披露 + 截断） | 2MB 输出不卡 |
| 5 | `mobile.js` 重建审批 / 提问 | 全流程可闭环 |
| 6 | `mobile.js` 做 diff 视图 | 能读改了什么 |
| 7 | 远程桌面 Surface | 第二个 Surface |

**每步都能立刻在真机上看效果** —— 这是 WebView 方案相对 Flutter 的最大优势：
不需要每次改动都重新编译打包。

---

## 7. 需要你定的一件事

**选 A、B 还是 C。**

我的倾向是 **B**，但有一条前提要说清：

> B 会作废我们已写的 117 个 UI 测试。这是真实损失。
> 如果你更在意「已有投入不浪费」，那 C（Flutter 全原生）也不该被否 ——
> 它的协议层已经写好，剩下的是 UI 工作量，而 UI 工作量恰好是这个项目里
> 风险最低的部分（写错了编译就报错，不会静默错判状态）。

**换句话说：C 的风险集中在「协议层写错但看不出来」，B 的风险集中在
「要持续跟进上游槽位契约」。前者已经在坑里踩了三次，后者可控。**

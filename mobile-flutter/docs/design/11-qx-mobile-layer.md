# 方案定案：千寻自有网关 + `/qx-mobile` 定制层（**推翻 B 的 3443 路线**）

> ⚠️ 本文取代 `07-webview-vs-flutter.md` §4 的 B 方案细节与
> `08-l3-contract.md` 的 dshMobile 契约部分。
> 结论方向不变（WebView + 重排重写），**技术路线换了，而且大幅简化**。

---

## 0. 一句话

> **不用 dsh-mobile 插件的 3443 端点，用千寻自己的 23090 网关 +
> 它已经内置的 `/qx-mobile` 定制层。基础设施已在千寻仓库里，编译即可，
> 不需要装任何插件。**

实测通过（`scripts/probe_qx_l3.py`）：

```
[1] GET /qx-mobile/version  → 200 {"css":"b95a…","js":"9daf…","v":1}
[2] GET /qx-mobile/custom.js → 200 5828 B，含 qxMobile.register，非占位
[3] 根页面 script 标签       → /qx-mobile/bootstrap.js ← 已注入
[4] 带 cookie 调 session/list → ok=true 394 条
    字段签名 {1:208, 18:65, 19:37, 20:84}  ← 与全量实测完全一致
```

---

## 1. 千寻已有的基础设施（`src-tauri/src/remote/mobile_ui.rs`）

文件头注释自己写着「**移动定制层（方案 A）**」—— 千寻早就做了这件事。

### 1.1 六个路由（全部鉴权，除 unknown）

| 路由 | 作用 | 关键细节 |
|---|---|---|
| `GET info` | 电脑端身份（hostname / version / dshReady） | 带 CORS `*`，供 App 配对前用 query token 探测 |
| `GET version` | `{v, css, js}` 内容 hash | css/js 各 16 位 hex = sha256 前 16 位 |
| `GET bootstrap.js` | 引导脚本，**内嵌二进制**（`include_str!`） | 随千寻发布，`no-cache` |
| `GET custom.css` | 定制样式 | ENOENT 回退占位注释 + `x-qx-mobile-custom: fallback` 头 |
| `GET custom.js` | 定制脚本 | 同上 |
| 其余 `/qx-mobile/*` | **404，不转发 DSH** | 防止内部路径漏给上游 |

### 1.2 热刷新机制（`assets/qx-mobile-bootstrap.js`）

- 轮询 `/qx-mobile/version`，**页面可见 5s / 隐藏 30s**（`:15-16`）
- hash 变化 → 样式换 `<link href>`（URL 带 `?v=<hash>`，浏览器按 URL 长缓存）
- 脚本**先挂新 root 执行成功，再卸旧 root**；失败保留旧版只报 console（`:78-111`）
- 网关不可达时**静默重试，绝不影响 DSH 本体页面**（`:124-126`）

⚠️ 「先挂新再卸旧」这个设计对我们**极其重要**：写 `custom.js` 时
可以边改边看，语法错误不会让页面变白。

### 1.3 安全设计

| 项 | 实现 |
|---|---|
| 鉴权 | `state.mobile_authorized(query, headers)` —— 回环放行，局域网要已配对（cookie 或 query token） |
| 大小上限 | css 512KB / js 1MB，超限 413（对齐 dsh-mobile） |
| 缓存 | `public, max-age=31536000, immutable` + ETag/304 |
| 路径隔离 | 未知 `/qx-mobile/*` 一律 404，**不转发 DSH** |

---

## 2. 契约（与 dsh-mobile 完全不同，别混用）

| | 千寻（本方案） | dsh-mobile |
|---|---|---|
| 入口 | `window.qxMobile.register` | `window.dshMobile.register` / `define` |
| 上下文 | `{ root, document, window }` | `{ document, request, root, window }` |
| 文件 | `mobile-access/custom.{js,css}` | `mobile-access/mobile.{js,css}` |
| 端点 | `192.168.20.2:23090`（千寻网关） | `192.168.20.2:3443`（插件自起 HTTPS） |
| 认证 | 现有 `qx_token` cookie | `dsh_ma_session` + CSRF + Origin + CIDR |
| TLS | **明文 HTTP，无证书问题** | 自签 CA，需精确 Origin 固定 |

### 2.1 没有 `request` 不是问题

`mount({ root, document, window })` **不传 `request`**。

但脚本跑在**同源页面**里，`window.fetch` 本身就带 cookie。
dsh-mobile 之所以额外传 `request`，是因为它要做 CSRF 头注入
（`gateway.ts:129` 改写 `window.fetch` 自动塞 `x-dsh-mobile-csrf`）——
那是它自己那套认证的需要。千寻走 DSH 原有的 `qx_token`，CSRF 与 Web 端
完全一致，不需要额外包装。

**所以 `custom.js` 里直接 `window.fetch('/api/session/list', …)` 即可。**

---

## 3. 壳层要做的（比原方案少 4 件）

| # | 模块 | 状态 |
|---|---|---|
| 1 | 设备列表（沿用现有 `connections_screen.dart`） | ✅ 已有 |
| 2 | WebView 宿主 + 加载 `http://<ip>:<port>/` | ❌ 新增 |
| 3 | 返回键语义（WebView 历史 → 设备列表 → 退出） | ❌ 新增 |
| 4 | 底部 Surface 切换（对话 / 远程桌面） | ❌ 新增 |
| 5 | 扫码配对（用 `qx-gate?code=` 兑换） | ⚠️ 已有 `pair_url.dart` |
| 6 | ~~自签 CA 固定~~ | ❌ **不需要** |
| 7 | ~~mobile-access 四层认证~~ | ❌ **不需要** |
| 8 | ~~native-pair / renew 配对流程~~ | ❌ **不需要** |
| 9 | ~~三层 cookie 注入~~ | ❌ **不需要** |

**省掉的 4 件恰好是最容易出错、最难测的那 4 件。**

---

## 4. 但有一条硬约束：WebView 必须带 cookie

⚠️ **`/qx-mobile/*` 与 `/api/*` 都要已配对**（`mobile_authorized`）。
所以 WebView 首次加载前**必须先注入 `qx_token` cookie**。

现有 `GatewayClient` 已有配对能力（`pair_url.dart` 两种格式都解析：
`?code=<32hex>` 一次性 / `?token=<64hex>` 长期），
所以这条**不需要新工作**，但 WebView 宿主要接上。

⚠️ **不能用 `?token=` 放 URL 里**：配对链接带一次性凭据，
URL 会进日志与历史。`mobile_authorized` 接受 cookie 或 query，
但**壳层应优先走 cookie**，把 query 只留给「配对前探测」。

---

## 5. L3 定制层要写什么（custom.css + custom.js）

按 `08-l3-contract.md` §5 的 20 条硬约束 + 本方案的四层粒度：

```
L0  改 CSS 变量 / 媒体查询
L1  换 root 布局，保留 DSH 原版内容组件   ← 先做这层
L2  替换某些槽位的内容组件
L3  用 custom.js 重建整块界面
```

**关键：`register` 的 `root` 是追加层，不替换 DSH 界面**
（`bootstrap.js:38-41` 建独立 div 挂 `document.body`）。

所以「重建」= 自己的界面挂 root + CSS 藏 DSH 层。
**但不要一上来就 `display:none` 藏掉 DSH** —— 那是「重写」不是「重排」，
会把 141 个 RPC 的 UI 工作量全揽过来。正确顺序：

1. `custom.css` 先做窄屏适配（断点、48dp 触控、16px 输入、软键盘）
2. 确认哪几块**必须**自己重建（导航、工具卡、审批、diff）
3. `custom.js` 逐块替换

---

## 6. 与用户需求的对应

用户要的（`09-shell-bridge.md` + 本轮要求）：

| 要求 | 本方案怎么满足 |
|---|---|
| 先进入**设备列表** | 壳层根路由 = 设备列表（沿用现有实现） |
| 选设备进入 DSH，**DSH 是独立页面** | 选设备 → `push('/dsh')`，WebView 占满 |
| 主页面内有**返回按钮**回设备列表 | 顶栏 ← + 系统返回键，第 3 步语义 |
| 底部切**远程桌面** | 壳层底部 Surface 切换（原生视图显隐） |
| DSH 内部按 **Web 端布局** | 复用 DSH 原版槽位，只重排不重写 |
| 侧边栏等不兼容的**用 Flutter 特有方式** | `custom.js` 重建为全屏覆盖层 + 原生返回 |
| mobile 能力加到**默认千寻项目** | `mobile_ui.rs` 已在仓库，只需写 `custom.{js,css}` |
| **只要编译完成，无需安装** | 定制文件在 `<dsh_home>/mobile-access/`，**热生效** |

---

## 7. 风险与退路

| 风险 | 概率 | 退路 |
|---|---|---|
| 千寻升级改了 `/qx-mobile` 契约 | 低 | 契约在 `mobile_ui.rs` 里，改动会体现在源码 |
| DSH 页面结构变化导致选择器失效 | **中** | 选择器要**全部带自有前缀**；锚点用 `data-*` 与 `role` 而非 class 名 |
| 2MB 工具输出卡顿 | 中 | `custom.js` 虚拟滚动 + 截断 |
| 明文 HTTP 被系统拦 | 低 | `network_security_config.xml` 放行局域网段（已存在该文件，需检查配置） |

**DSH 页面结构变化这条要特别当心**：`mobile-layout.ts` 用
`[data-disclosure-row]`、`[data-question-key]`、`[class*="_ioSection"]`
这些**语义属性**而非 class 名，说明 DSH 自己也认为 class 名会变。
我们的 `custom.js` 必须同样只用**稳定属性**。

# 壳层认证契约（源码级验清，方案 B 的必要条件）

> 全部来自 `dsh-mobile/src/gateway.ts` 与 `http-security.ts`。
> 本文回答一个问题：**壳怎么变成一个「已认证的浏览器」**。
>
> ⚠️ 本文是方案 B 的**最后一个未验环节**。此前我以为「`request` 已认证」
> 是白送的 —— 实测发现**不是**，认证要壳自己做。差点误判方案 B 不成立。

---

## 0. 先说我犯的错

上一轮我验到 403，写成「通道不通的风险」。**那是我探针的缺陷。**

真实链路是两层认证，我探针只过了第一层的一半：

```
请求 → [Origin 检查] → [CSRF 检查] → [Session Cookie 检查] → 反代到 DSH
         ↑ 我漏了         ↑ 我漏了        ↑ 我也没有
```

我拿网关的 `qx_token` 去打 mobile-access，那是**另一套凭据**。
带 Origin 后从 403 变 401，正说明卡在下一层（Session Cookie）。

---

## 1. 四条凭据/检查（全部来自源码）

`http-security.ts:4-10` 定义了常量：

```ts
export const DEVICE_COOKIE  = 'dsh_ma_device'
export const SESSION_COOKIE = 'dsh_ma_session'
export const CSRF_COOKIE    = 'dsh_ma_csrf'
export const CSRF_HEADER    = 'x-dsh-mobile-csrf'
export const LOCAL_ADMIN_PREFIX = '/api/mobile-access'
export const AUTH_PREFIX        = '/mobile-access'
export const WS_PATHS = new Set([
  '/api/events.mux', '/api/events.host', '/api/remote.mux',
])
```

| 检查 | 位置 | 失败时 |
|---|---|---|
| **Origin** 精确匹配 | `gateway.ts:2066` | `403 forbidden` |
| **Session Cookie** 存在且有效 | `gateway.ts:1010-1014` | `401 authentication_failed` |
| **CSRF** 非 GET/HEAD 时必须带 | `gateway.ts:1016-1019` | 401 |
| **CIDR** 来源地址在允许段 | `network.ts` `addressAllowed` | 403 |

⚠️ **CSRF 头由页面自动注入**（`gateway.ts:129`）：
DSH Mobile 在页面里注入一段 bootstrap，改写 `window.fetch`，
对同源的非 GET/HEAD 请求自动从 cookie 里读 `dsh_ma_csrf` 塞进
`x-dsh-mobile-csrf`。

**所以 `mobile.js` 里的 `request` 不必自己处理 CSRF** ——
前提是请求经过 DSH Mobile 注入的那层。

---

## 2. 配对流程（壳必须实现的部分）

`gateway.ts:1033-1057` `handlePair` + `:1207` 路由：

```
POST /mobile-access/auth/pair
  body: { token: <配对码>, label?: <设备名> }
  → 201 { paired: true, deviceId, csrfToken, sessionExpiresAt }
  → Set-Cookie:
      dsh_ma_session = <短期 session>   HttpOnly, path=/, maxAge=sessionExpiresAt
      dh_ma_csrf    = <csrfToken>        **非** HttpOnly（页面要读）
      dsh_ma_device = <长期 deviceToken> HttpOnly, path=/mobile-access/auth/renew
```

### 2.1 配对码从哪来

**必须从电脑上取**。`gateway.ts:1185` 明确：
配对页只有在 `pairingStatus().open` 为真时才可访问（否则 404）。

`gateway.ts:2190` 有本机管理端点 `POST /api/mobile-access/pairing/open`
（`LOCAL_ADMIN_PREFIX`），以及 `GET /api/mobile-access/devices`。

README.zh-CN.md `:14` 的用户流程：
> 局域网连接在 **移动访问 → 局域网** 中生成密钥或配对链接

**即：用户在 DSH 桌面端点「移动访问」卡片 → 生成配对密钥 → 手机扫码。**

### 2.2 续期

```
POST /mobile-access/auth/renew
  Cookie: dsh_ma_device=<长期 token>
  → 200 { renewed, deviceId, csrfToken, sessionExpiresAt }
  → 重新 setSessionCookies
```

`gateway.ts:1059-1085`。有速率限制（`renewLimiter`，超了 429）。
DSH Mobile 会在 session 过期时自动续（`README:19`：日常 Web 会话是短期的）。

---

## 3. 给原生 App 的专用端点：`/mobile-access/auth/native-pair`

`gateway.ts:1215`：
```
POST /mobile-access/auth/native-pair
```

⚠️ **这一个我还没读源码**，但从路由位置看，它与 `handlePair`（`:1033`）
并列，是**专门给原生客户端的配对入口** —— 因为原生 App 没有浏览器，
不能走 `PAIR_PAGE` 表单（`gateway.ts:132-148` 那张 HTML 页）。

**它大概率是「用 deviceToken 换 session」而不是「用配对码换 session」**，
但**我还没验**，不猜。Shell 3 开工前必须读 `handleNativePair` 的实现。

---

## 4. 对壳的硬要求（照抄 `dsh-mobile` 的 Android 实现）

| # | 要求 | 依据 |
|---|---|---|
| 1 | 只接受 HTTPS origin，Manifest 禁明文 | `README:40` |
| 2 | CA **只在 App 内固定**，不装进系统 | `README:41`、`:49` |
| 3 | 接受**该 CA 为精确主机+有效期**签发的叶证书，其余 `SslError` **全部取消** | `README:41` |
| 4 | Origin 只存协议+主机+端口；**路径/查询/fragment 不持久化** | `README:43` |
| 5 | 同源主页面留在 WebView；**外部 HTTPS 链接交系统浏览器** | `README:43` |
| 6 | 文件输入用系统文档选择器，**不需存储权限** | `README:44` |
| 7 | 下载只允许**当前 exact origin 的前台 GET**；认证控制路径永不下载 | `README:45` |
| 8 | device token 用 **Android Keystore** 加密；Web 存储在 App 沙箱 | `README:46` |
| 9 | **App 备份关闭**，TLS 私钥与签名密钥不入库 | `README:47` |
| 10 | JS 桥用 `androidx.webkit` WebMessage，**绝不用 `addJavascriptInterface`** | `README:53` |
| 11 | 入站消息 ≤ 1 MiB、剪贴板 256 KiB、二进制 8 MiB、回复 12 MiB | `README:53` |
| 12 | 桥**不暴露** Cookie / 设备 token / 配对密钥 / CA 私钥 / 任意 Android API | `README:53` |

---

## 5. ⚠️ Python 探针的一个坑（别再踩）

用 `ssl.create_default_context(cafile=ca.pem)` 会报：

```
CERTIFICATE_VERIFY_FAILED: Missing Authority Key Identifier
```

**这不是证书无效。** OpenSSL 的链验证要求 CA 带 AKI，
而 dsh-mobile 自签的 CA 没带。Android / 浏览器 / Node 都不强制要求 AKI。

**所以：壳不能用「把 CA 装进系统信任库」的做法**（要求 2 就是为了防这个），
必须照 `PinnedTls.kt` 做**精确 Origin 固定**。

---

## 6. 方案 B 的最终确认

L3 通道要成立，需要同时满足：

| 条件 | 谁负责 | 状态 |
|---|---|---|
| 页面在 `https://<ip>:3443` 下 | 壳 | ✅ 端点已活（403/401 而非连不上） |
| TLS 固定通过 | 壳 | ⏳ 未验（真机） |
| 已配对 → 有 `dsh_ma_session` | 壳（配对流程） | ⏳ 未做（`devices.json` 不存在） |
| Origin / CSRF 自动满足 | DSH Mobile 注入 | ✅ 源码确认 |
| `request` 能调 RPC | — | ⏳ **等 session 到位后验** |

**结论：通道设计上没有阻断项。** 剩下的全是壳该做的工程：
TLS 固定 + 配对 + WebView 宿主。

⚠️ **但我必须诚实说明一处未验**：
我至今**没有拿到过一个真实的 `dsh_ma_session`**，因此
「配对后 `request` 能否直接调 RPC」这一条**仍是推断，不是实测**。
它依赖 `handleNativePair` 与 `access.pair()` 的实现细节。
**Shell 3 的第一件事就是把它验掉** —— 在那之前，方案 B 是「设计上成立」
而非「已验证成立」。

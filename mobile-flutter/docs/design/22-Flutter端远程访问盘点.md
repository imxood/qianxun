# 22 — Flutter 端远程访问 DSH 现状盘点

> 盘点日期：2026-11-05｜范围：`mobile-flutter/lib/data/dsh/**`、`lib/data/store/connection_store.dart`、`lib/features/dsh/**`、`lib/features/connections/**`
> 性质：只读盘点（本次未改 Flutter 端任何代码）。结论均带 `文件:行号`。

## 0. 一句话

Flutter 端走**两条彼此独立的远程通道**（网关 `qx-gate` 明文 HTTP + DSH Mobile 插件自带的 `mobile-access` HTTPS 3443），统一用「配对链接 -> 换凭据 -> 存连接 -> 每次请求手工带 cookie」四步；DSH 页面本体由 WebView 承载，Flutter 只做原生产物（列表/审批/外链分流）。

## 1. 两条通道对比

| 面   | 网关通道（方案 A）                                                                                                                                                 | mobile-access 通道（方案 B）                                                               |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ |
| 地址 | `http://<ip>:23090`                                                                                                                                                | `https://<ip>:3443`                                                                        |
| 凭据 | `qx_token`（长期设备 token）                                                                                                                                       | `dsh_ma_session` + `dsh_ma_csrf`（session 需续期）                                         |
| TLS  | 无（明文；密文性靠 EasyTier 隧道）                                                                                                                                 | 自签 CA，**必须按精确 origin 固定**（不能装系统信任库，缺 AKI）                            |
| 代码 | [gateway_client.dart](../lib/data/dsh/gateway_client.dart)、[dsh_client.dart](../lib/data/dsh/dsh_client.dart)、[stream_mux.dart](../lib/data/dsh/stream_mux.dart) | [mobile_access.dart](../lib/data/dsh/mobile_access.dart)（数据模型已就绪，传输层未见实现） |
| 现状 | **已实现并真机走通**                                                                                                                                               | **仅数据模型 + 凭据壳**（见 §5）                                                           |

## 2. 完整链路（网关通道）

1. **配对链接解析** — [pair_url.dart](../lib/data/dsh/pair_url.dart):8-10
   - 新式 `?code=<32hex>`（一次性兑换码）与旧式 `?token=<64hex>`（长期 token）两形态都收；
   - `PairUrl.tryParse` 严格校验 scheme/host/port（`protocol_test.dart:30-42`）。
2. **探测 + 兑换** — [gateway_client.dart](../lib/data/dsh/gateway_client.dart):106-159
   - 旧式：直接带 query 调 `/qx-mobile/info`（网关对该路由开 CORS `*`），**免兑换**；
   - 新式：先 `GET /qx-gate?code=…` 拿 `Set-Cookie: qx_token`，再用 cookie 调 info 复核（两跳，缺一不可）。
   - 结果分类为 5 态：`ProbeOk` / `ProbeUnauthorized` / `ProbeUpstreamDown` / `ProbeUnreachable` / `ProbeBadUrl`（`:46-82`），UI 可据此给不同文案。
3. **凭据落盘** — [connection_store.dart](../lib/data/store/connection_store.dart):60-65
   - `?code=` 一次性，兑换后必须存 `qxToken`，否则每次冷启动都要重扫码；
   - `Connection.effectiveToken` = `qxToken ?? pair?.token`（`:117`）；
   - **值相等性**必须含 `qxToken`：family provider 用 `==` 定位条目，漏掉 token 会导致「重新配对后 WebView 仍用旧 cookie」（`:12-30` 的注释是踩坑记录）。
4. **API 调用** — [dsh_client.dart](../lib/data/dsh/dsh_client.dart):110-169
   - 线格式：`POST /api/<ns>/<method>`，两层信封 `{"type":"client-request","rpcId","method","payload":{"args":{…}}}`；
   - **参数名由宿主 descriptor 声明，猜不得**：`session/list` 用 `_request`，其余单参用 `request`，无参用 `{}`（`:71-98`）；摊平写 payload 会被宿主拒绝；
   - `dart:io` 与 `package:http` **都没有 cookie jar**，每次请求手工带 `Cookie: qx_token=…`（`:25-26, :130`）。
5. **事件流** — [stream_mux.dart](../lib/data/dsh/stream_mux.dart):239-292
   - `IOWebSocketChannel.connect` 到 `ws://<origin>/api/remote.mux`（`QxRoute.streamMux`），**手工带 cookie**（浏览器自动带，dart:io 不会）；
   - `pingInterval: 25s`（网关↔DSH 那腿有 10 分钟空闲超时，静默链会被自己掐掉）；
   - `channel.ready` 成功时**必须** complete `_openController`（否则界面永远卡「正在连接电脑…」——`:269-273` 记了这个坑）。
6. **DSH 页面** — [dsh_page.dart](../lib/features/dsh/presentation/dsh_page.dart):63-177
   - WebView 承载，**加载前先写 cookie**（`WebViewCookieManager().setCookie`），否则「界面在但数据全空」；
   - 加载 URL 必须剥 query（`stripCredential`），token 只进 cookie 不进 URL/历史；
   - 外链交系统浏览器、同源留 WebView（`:138-151`）；
   - 页面 UI 本身来自 `<dsh_home>/mobile-access/custom.{css,js}`（千寻定制层，见千寻侧 `docs/05`）。

## 3. 与千寻侧的对应关系

| Flutter 侧                      | 千寻/DSH 侧                                                                                                          |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `/qx-gate?code=`                | `gateway.rs` `pair_url_with_code` + `PairCodes`（5 分钟一次性码）                                                    |
| `Cookie: qx_token`              | `gateway.rs` `authorized()`（cookie 优先，query 兜底）                                                               |
| `/qx-mobile/info`               | `mobile_ui.rs` `info()`（CORS `*`，供未配对探测）                                                                    |
| `/api/<ns>/<method>`            | `gateway.rs` 兜底 handler -> `dsh_upstream::forward`（透传到 DSH 回环）                                              |
| `/api/remote.mux`               | `gateway.rs` `ws_handler` -> `dsh_upstream::upgrade_ws`（帧级双向桥）                                                |
| `mobile-access/custom.{css,js}` | `mobile_ui.rs` `/qx-mobile/custom.css` `/qx-mobile/custom.js`（2026-10-31 已按用户要求下线，见千寻 `docs/21` §背景） |

## 4. 已识别的坑（代码注释即证据）

1. **旧数据只有 URL 带 token**，`qxToken` 字段为 null —— 只读 `connection.qxToken` 会得到「没有凭据」的假象。判定收敛在 [dsh_credential.dart](../lib/features/dsh/presentation/dsh_credential.dart) 的 `resolveCredential()`（纯函数 + 6 个测试）。
2. **`Connection` 必须有值相等性**（含 `qxToken`），否则 family provider 每次都当新条目，WS 被反复拆建。
3. **RPC 参数名不能猜**（`_request` vs `request`），猜错报的是宿主 descriptor 错误。
4. **WS 握手成功必须 complete `_openController`**，否则永远卡在连接中。
5. **cookie 不落 jar**：HTTP 与 WS 都要手工带；WebView 侧靠 `setCookie` 预置。
6. **移动端定制层是最脆的一环**（靠 DOM 注入，DSH web UI 一改就碎）——2026-10-31 已下线该层，Flutter 端 `dsh_page.dart` 的 `_probeCustomLayer` 与 `DshHostPhase.customLayerMissing` 状态机**仍然存在**，现已失去存在意义（待清理项，见 §5）。

## 5. 未闭环 / 待确认

| 项                            | 状态                                                                                                                                                                    | 证据                                                                                           |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| mobile-access（方案 B）传输层 | **只有数据模型**，未见 HTTP/WS 客户端实现；`MobileAccessCredentials.cookieHeader` 无调用方                                                                              | [mobile_access.dart](../lib/data/dsh/mobile_access.dart):153（该方法注释自述「只给壳自己用」） |
| 连接页「两种连接」入口        | 连接页 UI 未见方案切换控件（grep `adapter/mode/方案` 仅命中一条说明文案）                                                                                               | `connections_screen.dart:85`                                                                   |
| 定制层探测状态机              | 定制层已下线，`_probeCustomLayer` / `customLayerMissing` 成为死代码                                                                                                     | `dsh_page.dart:198-215`、`webview_host_state.dart`                                             |
| 真机验收                      | 网关通道有真机迭代日志（`docs/design/14-真机迭代日志.md`）；mobile-access 通道未验证                                                                                    | —                                                                                              |
| 与本次新增的「本地服务反代」  | **Flutter 端未接入**：新反代面是独立端口 + `qx_proxy_to` cookie 形态，现有 `Connection` 模型只有 `url` 一个 origin 字段，装不下「同一网关下的第二个 origin + 目标切换」 | 千寻 `docs/21` §1                                                                              |

## 6. 若要让 Flutter 端用上本地服务反代

需要改的最小面（未实施，供决策）：

1. `Connection` 增加「反代目标」字段（目标 `host:port`），与既有 `url`（网关 origin）并存——两者不同源。
2. 新增一条 WebView surface：加载 `<反代端口 origin>/`，并在导航前种 `qx_proxy_to` cookie（目标 host:port）。注意 `Connection.effectiveToken` 只能给网关 origin，**反代 origin 也要 `qx_token`**（网关反代面与网关同鉴权）。
3. `isSameOrigin` 判定要把反代 origin 纳入（同源外链分流规则不变）。
4. 目标切换：改 `qx_proxy_to` cookie 后 reload（或直接导航到 `/?_qx_to=<目标>`，服务端会 302 回 `/`）。

## 变更记录

- 2026-11-05 首版：应用户要求盘点 Flutter 端远程访问实现（只读）。

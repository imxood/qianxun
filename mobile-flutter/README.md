# 千寻移动端 · Flutter 原生客户端

用 Flutter 重写的 DSH 远程客户端。**不使用 WebView** —— DSH 的会话列表、
转录、工具执行、审批、提问全部是原生控件，因此可以真正适配手机的宽度、
字号与手势，而不是把桌面页面缩小到 390pt 宽。

参考实现：`E:/develop/dsh-workspace/deepseek-harness-mobile`（Kotlin/Compose 原生客户端）
桌面网关：`E:/develop/dsh-workspace/qianxun/src-tauri`（Rust/axum）

> 参考项目是 **Kotlin/Compose** 而不是 Flutter——它提供的是**协议**，
> 不是模板。本实现把它的 wire 层逐条搬到 Dart，并把 UI 重新按手机的信息
> 密度设计。

规模：26 个 Dart 源文件，约 6.7k 行（含注释）。

---

## 1. 为什么不用 WebView

原来的 RN 客户端（`mobile/`）把 `/qx-gate` 塞进 `react-native-webview`。
这条路能跑通，但有三个结构性问题：

1. **桌面页面在手机上不可用。** DSH 的转录是按宽屏设计的多栏布局，压到
   390pt 后代码块横向溢出、工具输出把整屏刷满、侧栏挤成图标条。
2. **定制层是注入式且脆弱的。** `window.qxMobile.register` 往页面里插 DOM，
   依赖 `[class*="message"]` 这类非契约选择器，DSH 一次升级就可能失效。
3. **拿不到协议。** WebView 里的 JS 自己连 `/api/remote.mux`，壳层拿不到
   会话状态，也就做不了离线摘要、原生通知、后台 job 追踪。

原生实现直接持有 JSON-RPC 与流多路复用，能力上限完全不同。

---

## 2. 协议契约

三层结构，对应桌面端的真实分层：

```
┌─ Flutter 客户端 ────────────────────────────────────┐
│                                                      │
│  ① 千寻网关（Rust/axum，自有路由）                    │
│     GET  /qx-gate?code=…      配对兑换 → cookie       │
│     GET  /qx-mobile/info      身份探测（唯一开 CORS）  │
│     GET  /qx-mobile/version   定制层哈希               │
│                                                      │
│  ② DSH JSON-RPC（透传）                              │
│     POST /api/<ns>/<method>   一元调用                 │
│                                                      │
│  ③ DSH 流多路复用（唯一 WS）                          │
│     WS   /api/remote.mux      具名逻辑流               │
└──────────────────────────────────────────────────────┘
```

### 2.1 配对

桌面端签发的是 **一次性 code**，不是长期 token：

```
http://<ip>:<port>/qx-gate?code=<32 hex>   ← 当前格式，5 分钟有效，兑换即消费
http://<ip>:<port>/qx-gate?token=<64 hex>  ← 旧格式，仍兼容但不再签发
```

兑换：`GET /qx-gate?code=…` → `302` + `Set-Cookie: qx_token=<64hex>; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax`

> ⚠️ **修掉的老 bug**：RN 客户端 `mobile/src/validate.ts:7` 的正则是
> `^http:\/\/[a-z0-9.-]+:\d+\/qx-gate\?token=[0-9a-f]{8,}$`，只认 `?token=`。
> 网关早就改成签发 `?code=` 了，所以**当前签发的链接一律被拒**，且
> `searchParams.get('token')` 取到空串 → 探测永远 401 → 状态点永远灰。
> 本实现在 `lib/data/dsh/pair_url.dart` 里两种都认。

**cookie 必须手工管理。** `dart:io` 的 `HttpClient` 与 `package:http` 都
**没有 cookie jar**——不会持久化、不会自动重发。必须自己解析
`Set-Cookie` 并在每次请求（含 WebSocket 升级）上带 `Cookie: qx_token=…`。
浏览器会替你做，dart 不会。

凭据被存在连接记录里（`Connection.qxToken`），因为 `?code=` 是一次性的：
兑换一次原始链接就废了，不存就得每次冷启动重新扫码。

### 2.2 一元 RPC

```http
POST /api/<namespace>/<method>
{"type":"client-request","rpcId":"<uuid>","method":"<ns/method>",
 "payload":{"args":{…}}}
```

响应：

```json
{"type":"server-response","rpcId":"<同一 id>","result":{"ok":true,"value":{…}}}
{"type":"server-response","rpcId":"<同一 id>","result":{"ok":false,"error":{"code":"…","message":"…","details":{…}}}}
```

⚠️ `ok:false` 是**业务失败**（HTTP 仍是 200），与 4xx/5xx 是两回事。
混为一谈会让「会话不存在」显示成「网络断了」。

### 2.2.1 参数信封：两个从参考项目搬来就已过时的地方

参考项目（Kotlin，锁 harness 0.1.2-alpha.1）把参数**摊平**当 payload。
实测于 qianxun 0.7.0 内置的 harness（用 `scripts/probe_rpc.py` 打的），
真实形状是：

| 方法 | 正确载荷 | 写错会得到 |
|---|---|---|
| `session/list` | `{"args":{"_request":{}}}` | `args fields do not match the descriptor: missing "request"; unexpected "_request"` |
| `session/page` / `prompt` / `cancel` / `search` / `create` / `$events/result` | `{"args":{"request":{…}}}` | 同上 |
| `agentPresets/list` / `session/modelCatalog` | `{"args":{}}` | `args fields do not match the descriptor: unexpected …` |
| **任何**方法，payload 摊平 | `{"payload":{}}` | `Remote payload must contain exactly one plain-object args field` |

要点：

1. `payload` **恒**是 `{"args": {…}}` 这一层信封。
2. 信封里那个对象的键要**逐字匹配远端 descriptor**，而 descriptor 给
   「单个请求对象」起的名字**不统一**——只有 `session/list` 是 `_request`。
3. **不要凭记忆写。** 升级 DSH 后参数名可能变，`probe_rpc.py` 打一发就知道。

### 2.3 附件上传：**唯一一条不走 `{"args":{…}}` 的路由**

实测（`scripts/probe_upload.py`）：

```http
POST /api/session/uploadFileBinary?sessionId=<sid>&name=<文件名>
Content-Type: application/octet-stream

<裸二进制 body>
```

```json
{"ok":true,"value":{"receiptId":"<uuid>",
  "file":{"attachmentId":"sha256:<hex>","name":"probe.png","bytes":70}}}
```

四个必须记住的点：

1. **没有 `{"args":{…}}` 信封，也没有 `client-request` 信封。**
   这是裸二进制 POST，响应也是裸的 `{ok, value}`，不是
   `{"type":"server-response", …}`。用 `DshClient.call()` 打它必然失败
   （`415 Unsupported Media Type`）。
2. **`sessionId` 走 query**，不是 body、不是 header。
3. **`name` 走 query**；不传时服务端默认叫 `file`。
4. 返回的是 **`receiptId`**，不是 `attachmentId`。
   `receiptId` 才是送进消息的那个（一次性消费凭证）；`attachmentId`
   是持久标识，用于之后 `session/attachment` 读回。

实测过程里踩到的三个错误答案，都记在探针脚本注释里：
`multipart/form-data` → 415；`{"args":{"request":{…}}}` → 415；
裸 JSON base64 → 400（`sessionId is required`）。

### 2.4 编码规则（对齐 `WireJson`）

| 标志 | 含义 | Dart 侧要求 |
|---|---|---|
| `explicitNulls = false` | null 字段**被省略** | 绝不发 `"cursor": null` |
| `encodeDefaults = true` | 有默认值的字段**总是**写出 | 别写「等于默认值就省掉」 |
| `ignoreUnknownKeys` | 未知键忽略 | 解码宽松 |

### 2.3 流多路复用

`WS /api/remote.mux`——**唯一**的 WS 入口。文档里提过的
`/api/events.mux` 与 `/api/events.host` 在 harness 0.1.2 已被删除。

客户端 → 宿主：

```json
{"type":"open","streamId":"1","endpoint":"session/follow","payload":{"args":{…}}}
{"type":"cancel","streamId":"1"}
```

宿主 → 客户端：

```json
{"type":"item","streamId":"1","value":{…}}
{"type":"error","streamId":"1","error":{"code":"…","message":"…","details":{…}}}
{"type":"end","streamId":"1"}
```

逻辑流端点：

| 端点 | 用途 |
|---|---|
| `$events` | 连接存活 + 通知 + 主机请求。**无条件打开**——健康度不依赖界面是否在监听 |
| `$events/result` | 一元端点，回应 waterfall |
| `session/follow` | 会话实时流 |
| `session/control` | 队列 / 后台 job / 投影 |
| `workspace/follow` | 工作区注册表 |

### 2.4 `$events` 帧

| `type` | 字段 | 含义 |
|---|---|---|
| `ready` | `clientId`, `host.home` | 开帧，**必须第一个**。`clientId` 绑定本代所有回复 |
| `emit` | `event`, `args[]` | 通知，**断线不重放** |
| `waterfall` | `event`, `eventId`, `agentId`, `request` | 等待本客户端回答 |
| `cancel` | `eventId` | 撤回，**不回答**地撤掉提示 |
| 其它 | — | 原样保留 |

本客户端只认领两种 waterfall：

- `approval/request` → `POST $events/result` with `{"kind":"result","value":{"outcome":"allowed-once"}}`
- `user-questions/request` → `{"kind":"result","value":{"answers":[{id,selected[],custom?}]}}`

三种 outcome 是**三种不同的决定**：`next`（不回答，让后续监听器接手）、
`result`（认领）、`rejected`（让它失败）。

> ⚠️ **关闭提问必须是 `rejected` + code 恰为 `cancelled`。** 代理只认这一个码，
> 其它任何 code 都变成 `bad-response` 并让等待一直挂着。而「每条都答空选」
> 是一个**合法回答**，模型读成「没有偏好」——与关闭语义相反。

> ⚠️ `custom` 挂在**每一条** answer 上，不是挂在批次上。放错位置会被宿主的
> zod 解析**静默剥掉**，答案会变成空。宿主还会**逐位**校验条数与 id。

### 2.5 会话流

`session/follow` 的帧只有三个具名 tag：`snapshot` / `assistant-stream` /
其它（就是一条 record 本身）。这样设计是为了让将来新增的 record 类
**永远不会被误认成**开场帧。

事件封套：

```json
{"type":"assistant/message","seq":42,"time":1730000000000,"data":{…}}
```

关键类型（完整 29 个见 `lib/data/models/session.dart`）：
`turn/start` `turn/end` `user/message` `assistant/message` `tool/call`
`tool/result` `todo/write` `command/run` `session/title` …

> ⚠️ **`SessionSummary` 没有 `title` 字段，也没有 `createdAt`。** 标题只在
> `projections.values["title"]` 里。这是列表行最容易写错的地方。
> `agentPreset` 同理：顶层可能没有，要从 `projections` 兜底。

> ⚠️ **format v4 的 `tool/result` 有两个位置。** v3 把结果放在
> `content[].type=="tool-result"`；harness ≥0.1.7 的 v4 把它提升到了
> **消息顶层**（`role:"tool"` + 顶层 `toolCallId`/`isError`/`content`）。
> 只读一处会丢掉**整个**工具结果。见 `ToolResultData.fromJson`。

### 2.6 传输约束

- **`http://` 与 `ws://`，没有 TLS。** 密文性完全由 EasyTier 隧道提供。
- 端口默认 **23090（release）/ 23091（debug）**，用户可改。`17400` 只是
  `LEGACY_GATEWAY_PORT`，为迁移旧配置保留。**永远从配对链接读端口。**
- **网关↔DSH 那一腿有 10 分钟空闲超时。** 客户端这一腿没有，但中间设备
  上的静默链会自己断。`IOWebSocketChannel(pingInterval: 25s)` 处理这个。
- **网关重建会掐断全部连接。** 设置变更、DSH 启停、配对、吊销令牌都会触发。
  这是**常态不是错误**，所以 `DshConnection` 的重连是默认行为：
  指数退避（1s→2s→…→30s 封顶）+ 30% 抖动。没有抖动的话，网关一恢复
  所有客户端会在同一毫秒一起重连，把刚起来的 DSH 再撞一次。

---

## 3. 目录

```
lib/
├── main.dart
├── app/
│   ├── app.dart                  MaterialApp + 路由 + 字号封顶
│   └── theme/
│       ├── qx_tokens.dart        色彩 / 间距 / 圆角 / 动效刻度
│       ├── qx_type.dart          排版刻度
│       └── qx_theme.dart         ThemeData（暗色优先）
├── data/
│   ├── dsh/
│   │   ├── qx_protocol.dart      路由常量（对齐 gateway.rs）
│   │   ├── pair_url.dart         配对链接解析（code + token 双格式）
│   │   ├── gateway_client.dart   /qx-gate + /qx-mobile/info
│   │   ├── rpc_envelope.dart     JSON-RPC 信封与错误分类
│   │   ├── dsh_client.dart       一元 RPC + args 信封 + waterfall 应答
│   │   ├── stream_mux.dart       WS 逻辑流多路复用
│   │   └── dsh_connection.dart   代际管理 + 退避重连
│   ├── models/
│   │   ├── content.dart          ContentBlock / MessageData / JobView / Question
│   │   └── session.dart          SessionWireEvent / SessionSummary / PendingInteraction
│   └── store/connection_store.dart
├── features/
│   ├── connections/              连接列表（App 入口）
│   ├── sessions/                 会话列表（DSH 工作台）
│   ├── workspace/                会话详情 / 转录 / 输入 / 审批 / 提问
│   └── logs/                     运行日志（筛选 / 分享 / 清空）
└── shared/widgets/primitives.dart

scripts/
├── fdev.py           构建/检查/测试包装器（Python 里配好环境再调 flutter）
├── env.py            环境变量计算与组件自检
├── probe_rpc.py      协议探测器：打网关问「参数到底该长什么样」
└── publish_apk.py    把 APK 发布到本机 HTTP 服务供手机下载
```

---

## 4. 设计取向

**深色优先，工具感优先。** 这是一台放在口袋里的开发机控制台，用户是
盯着自己电脑屏幕干活的人。因此：

- 界面安静、信息锐利、状态色有意义。
- **四色语义环**（运行=青、等待=琥珀、完成=蓝、失败=赤）是整个 App 的
  信息骨架，只用于状态。**每个状态点旁边都有状态文字**——纯色点在色觉
  障碍用户眼里毫无信息。
- 卡片不靠阴影分层（暗色背景上几乎不可见），靠**描边 + 极轻表面提亮**。
- 排版签名在**等宽层**：工具名、命令、退出码、session id、耗时全部走
  JetBrains Mono。UI 文字层保持系统字体——中文没有一款非系统字体能在
  所有机型上拿到正确字重与 hinting，为中文换字体的代价远大于收益。
  但工具类 App 的性格来自数据层。
- 拒绝紫渐变 + 大圆角那套通用 AI 审美：那类界面不承载工具状态，
  只承载装饰。

关键交互决定：

- **思维链默认折叠**。它是模型的内心戏，不是结论。
- **工具卡参数与输出默认折叠，状态永远可见**。你在地铁上只想知道 agent
  还活着没有、在跑什么，而不是读 300 行 grep 结果。工具失败时**错误码
  在展开前就可读**——状态是结论，输出是过程。
- **可点区域必须能被看见**。表面用 `Material` 承载而不是
  `Container(decoration:)`：ListTile 把背景与墨水飞溅画在最近的 Material
  祖先上，中间隔一个带底色的 Container 会把涟漪整个盖住——点下去毫无
  反馈。这条是 UI 测试逼出来的，见 `test/ui_test.dart`。
- **用户消息右对齐带底色，模型消息不套气泡**。agent 输出往往很长，
  气泡会再砍 24px 宽度并制造大量视觉噪声。
- **同一信息不出现两次**。没有标题投影时标题回退到 cwd 末段，副行就
  不再重复它。
- **新内容不抢回底部**。用户正在上翻时不自动滚到底。
- **待答交互固定压在输入框之上**。它是阻塞的——agent 在等答案。
- **连接健康时不显示状态条**。永远占位的「已连接」条会吃掉 40 像素。

---

## 5. 构建

### 环境

本机已就绪（装在 D 盘，没写进系统 PATH）：

| 组件 | 路径 | 说明 |
|---|---|---|
| Flutter SDK | `E:\develop\tools\flutter` | 3.47.6 stable |
| JDK | `D:\programs\jdk-17.0.20.1+1` | Temurin 17.0.20.1（AGP 9.1 要求 17）——**原本就有** |
| Android SDK | `D:\programs\android-sdk` | 本次新装：platform-36 / build-tools 36.0.0 / platform-tools / emulator |
| AVD | `D:\programs\android-avd\qx36.avd` | pixel_6 规格，API 36 google_apis x86_64 |
| pub 缓存 | `E:\develop\.pub-cache` | **必须与项目同盘**，见下方 |

**先加载环境**，再跑 flutter：

```powershell
cd E:\develop\dsh-workspace\qianxun\mobile-flutter
python scripts\fdev.py check
```

`scripts/fdev.py` 在 Python 进程里把 `JAVA_HOME` / `ANDROID_HOME` / `PATH`
配好，再用 `subprocess` 调 flutter —— 子进程完整继承，不跟 shell 的变量
作用域打架。

```bash
python scripts\fdev.py check     # analyze + test
python scripts\fdev.py analyze
python scripts\fdev.py test
python scripts\fdev.py apk       # 分 ABI release
python scripts\fdev.py apk --debug
python scripts\fdev.py clean
python scripts\env.py --check   # 只看环境齐不齐
```

> 项目里**没有** `.ps1` / `.bat`。本机实测在 dot-source 的 `.ps1` 里设
> `$env:PUB_*` 会被**静默丢弃**（设完读回来是空），而同样内容在 Python 的
> `os.environ` 里是稳的。与其每次跟 shell 变量作用域打架，不如在 Python
> 里一次配好。

### 协议探测脚本

`scripts/probe_rpc.py` 会直接打网关，把每种方法的真实载荷形状打出来：

```powershell
python scripts\probe_rpc.py --token <64位hex>
```

**不要凭记忆写 DSH 的参数名。** 参考项目锁在 harness 0.1.2-alpha.1，
载荷是摊平的；实际部署（qianxun 0.7.0 内置的 harness）要求
`{"args": {…}}` 信封，且信封内的键要逐字匹配远端 descriptor——参数名
**并不统一**（`session/list` 是 `_request`，其余是 `request`）。猜不得，
只能打一发看它说什么。

### ⚠️ 跨盘符会把构建搞挂

Kotlin 2.x 的增量编译用 `Path.relativeTo()` 算缓存键。pub 缓存在 C:、
项目在 E: 时它会抛：

```
IllegalArgumentException: this and base files have different roots:
  C:\Users\maxu\AppData\Local\Pub\Cache\hosted\...\shared_preferences_android-2.4.28\...
  E:\develop\dsh-workspace\qianxun\mobile-flutter\android\...
```

表面报错却是 `Could not close incremental caches in build/...`，
**一个字都没提「跨盘」**，极易被误判成代码问题。

**修复在 `android/gradle.properties`**：

```properties
kotlin.incremental=false
kotlin.incremental.useClasspathSnapshot=false
```

全量构建只多花约 4 分钟，换构建稳定是明确的交易。要恢复增量：删掉这两行
并 `flutter clean`。**不要**试图靠搬 `PUB_CACHE` 解决——那只是绕开症状，
损坏的缓存表仍会让构建继续失败。

### 日志与排查

传输层的故障（cookie 没带上、WS 升级被拒、mux 帧不认识、超时）在界面上
的表现是**转圈**或**显示离线**——没有任何信息能说明卡在哪一步。所以
App 内置了完整的日志链路：

- **入口**：连接页右上角的回执图标
- **全局记录**：`lib/core/qx_log.dart`，内存保留最近 2000 条，同时**落盘**
  （App 被杀也留现场）。`FlutterError.onError` 与 `PlatformDispatcher.onError`
  都已接管，未捕获异常会写进日志而不是直接崩掉。
- **埋点**：WS 握手、mux 收发帧、`$events` 开帧、一元 RPC 的每次调用与
  结果、探测的每一步（含 cookie 有无、状态码、耗时）。
- **页面**：级别筛选、关键字过滤、实时追加、清空、**分享**、复制到剪贴板。
- **分享时正文和文件都带**。只给文件的话，分享到「聊天/笔记/邮件正文」
  这类只收文本的目标时对方拿到的是空的（曾经就因此把一句摘要当日志发
  出去）；只给正文则超 Binder 事务上限会崩。文本那份按 400 条截断。

下次再出问题，点右上角回执图标 → 分享，把 txt 发过来即可。日志里 token
只打头尾 4 位，不会泄漏完整凭据。

### 验证状态

```
flutter analyze   → No issues found!
flutter test      → 00:02 +145: All tests passed!
scripts\check_contrast.py → ✓ 全部达标（明暗两套色板，WCAG AA）
flutter build apk → √ Built build\app\outputs\flutter-apk\app-*-release.apk
```

APK 实测（`aapt2 dump badging`）：

| 项 | 值 |
|---|---|
| package | `com.qianxun.qianxun_mobile` |
| versionName | `0.3.0` |
| minSdk / targetSdk / compileSdk | 24 / 36 / 36 |
| application-label | 千寻 |
| 大小 | 见下（分 ABI release，arm64 约 18 MB） |

测试分五类（145 项）：

- `test/protocol_test.dart` —— **协议契约**：配对链接双格式、cookie 解析、
  RPC 信封的成功/业务失败/传输失败三分、`tool/result` 的 v3/v4 双位置读取、
  `SessionSummary` 标题只能来自投影、`turn/end` 的两种嵌套形状、未知事件/块
  不丢、提问回答的 `custom` 语义。
- `test/attachment_test.dart` —— **附件层**：MIME 猜测、二进制上传路由、
  图片内联不上传、同一 `requestId` 重发安全。
- `test/session_list_test.dart` —— **五态判定与排序**：每一态都有真实数据源、
  判定顺序（已断开压过一切 / 实时优先于快照 / 归属不串台）、第二行是 agent
  做的事而非用户输入、`等你` 永远置顶、`userQuestions` 两个嵌套层级。
- `test/cold_start_test.dart` —— **冷启动**：恰好一台才自动选、多台不替用户
  决定、显式选择优先于回退。
- `test/design_system_test.dart` + `test/ui_test.dart` + `test/logs_test.dart`
  —— **布局与视觉**：轨的真实渲染宽度、状态词在 320/360/390dp × 1.0/1.5/2.0x
  下不被裁切、2.0x 字号零溢出、WCAG 对比度闸门。

UI 测试不是「凑合的替代品」：它抓到了三个真机截图看不出来的缺陷，
其中一个（涟漪被吞）会让所有可点区域**完全没有触觉反馈**。

### 模拟器：当前不可用

已装 emulator 37.2.12.0 与 API 36 x86_64 镜像，AVD `qx36` 已建。
但**跑不起来**：

```
emulator -accel-check
→ Android Emulator hypervisor driver is not installed on this machine (exit 6)

C:\Windows\System32\drivers\hvax64.sys   不存在
Win32_ComputerSystem.HypervisorPresent  False
```

Windows 上的 Android 模拟器对 x86_64 镜像**强制要求硬件加速**（WHPX）。
本机 CPU 固件虚拟化是就绪的（i9-14900HX，VMX + SLAT 均开），但 WHPX
**特性未启用**。用 `-accel off` 走 TCG 软件模拟实测：进程能起，adb 能看到
`emulator-5554`，但 **20 分钟仍未完成启动**（`sys.boot_completed` 始终为空，
状态一直 `offline`）。

启用需要管理员权限且**必须重启**：

```powershell
# 管理员 PowerShell
DISM /Online /Enable-Feature /FeatureName:HypervisorPlatform /All /NoRestart
# 重启后：
. .\scripts\env.ps1
emulator -avd qx36
```

在 WHPX 启用之前，真机/模拟器验证都做不了，只能到「APK 构建成功 +
55 项测试通过」这一层。

### Android 明文流量

网关只有 `http://`/`ws://`，Android 9+ 默认禁止明文。
`android/app/src/main/res/xml/network_security_config.xml` 已配好：
`base-config` 禁止明文，只对私网段与回环放开——这样万一 App 里出现了指向
公网的明文请求，Android 会直接拦掉，而不是静默放行。

`AndroidManifest.xml` 已加 `INTERNET` 权限与 `networkSecurityConfig` 引用。

> `<domain>` 不支持 CIDR，所以按段列举。EasyTier 默认分配 `10.x.x.x`；
> 若你的网络用了别的段，在该文件里补一行。

### iOS ATS

需要 `Info.plist` 的 ATS 例外（gateway 是明文）：

```xml
<key>NSAppTransportSecurity</key>
<dict>
  <key>NSAllowsLocalNetworking</key>
  <true/>
</dict>
```

`NSAllowsLocalNetworking` 覆盖 `.local` 与链路本地地址；跨网段（EasyTier
虚拟网卡）可能仍需 `NSAllowsArbitraryLoads`。**iOS 真机尚未验证**。

---

## 6. 已实现 / 未实现

**已实现**

- 配对（`?code=` 与 `?token=` 双格式）、凭据持久化、在线探测
- 连接列表：状态点 + 状态文字 + 主机名、重命名 / 更新链接 / 删除
- 代际管理、指数退避重连、401/403 → 转为「需重新配对」而非无限重试
- 会话列表：状态、cwd、轮次、上下文占用、token、相对时间
- 会话转录：用户/模型消息、思维链折叠、工具卡、待办卡、轮次分隔
- 一元 RPC 全套封装、`$events` 帧解析与 waterfall 应答
- 审批弹层（允许一次 / 拒绝）、提问弹层（单选/多选/自由文本）
- 发消息（`requestId` 语义正确）、中断轮次

**未实现**

- 图片附件上传与展示（`session/uploadFileBinary`）
- 子 agent 树（`subagents/list` 端点已在客户端里，UI 未接）
- 后台 job 列表（`session/control` 帧解析未接）
- 工作区浏览（`workspace/follow`）
- 模型切换（`session/modelCatalog` / `selectModel`）
- 技能、目标（goals）、设置平面
- 系统推送通知
- iOS 真机验收

协议侧的 DTO 已在 `lib/data/models/` 铺好大部分形状，接 UI 是增量工作。

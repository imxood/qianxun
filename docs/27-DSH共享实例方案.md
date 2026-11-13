# 27 · DSH 共享实例方案（复用本机已在运行的 `dsh web`）

> 状态：已实现，本地 Windows 与远程 Linux(omarchy/CEF) 均验证通过。
> 关联代码：`src-tauri/src/harness/dsh_share.rs`（新增）、`src-tauri/src/harness/supervisor.rs`、
> `src-tauri/src/harness/commands.rs`、`src-tauri/src/remote/gateway.rs`、`src-tauri/src/remote/commands.rs`、
> `src-tauri/src/dsh_upstream.rs`、`src/lib/ipc/contract.ts`、`src/features/env/EnvPage.svelte`。

---

## 1. 要解决的问题

同一台机器上跑着两个 DSH 实例时，千寻内嵌的 DSH 页发消息会报：

```
This session is already in use, possibly by another running DSH instance
(such as dsh web or the desktop app). Quit other running DSH instances and try again.
```

**实测根因**（远程 omarchy，`~/.dsh` 共享）：

- 用户自己在终端/foot 里起的 `dsh web`（pid 318480，监听 `127.0.0.1:3080`）
  与千寻托管的 `dsh --profile web --no-open`（pid 429798，监听 `127.0.0.1:45239`）
  **共用同一份 `~/.dsh`**（两者都没设 `DSH_HOME`，都落到 `~/.dsh`）。
- 会话锁是**目录内的排他文件**：`~/.dsh/sessions/<工作区 slug>/session-<uuid>/session.lock`，
  靠 fd 排他持有（`/proc/*/fd` 里可见）。用户那个实例先锁上了同一把锁，
  千寻的实例打开同一会话即被拒。
- 附带副作用：两个实例抢同一个 `~/.dsh/profiles/web/cordis.yml`，谁后启动谁覆盖。

**结论**：这不是锁的实现 bug，是「两个实例共享一份 DSH home」的必然结果。
再加锁、重试、隔离工作区都只是把症状往后推。

---

## 2. 决策：方案 A —— 复用，而不是再起一个

用户拍板（m03648「A」）：**千寻自动发现并复用已在运行的 `dsh web`；没有才自己拉起。**

于是同一台机器上从「两个 DSH 抢一把锁」变成「一个 DSH、一把锁」——冲突从根上消失。

---

## 3. 关键机制：自签浏览器会话 cookie

### 3.1 启动 token 拿不到

DSH web 启动时打印的 `http://127.0.0.1:PORT/?token=…` 里那个 token：

- 只存在进程内存（`PROCESS_LAUNCH_TOKENS`），进程退出即失效；
- 日志里没有——实测 `~/.dsh/logs/startup-*.log`（89 KB）对 `dsh web` 命中 0 次，
  `bili.log`、billion-context 的 `proxy-origin`/`instances/*.json` 也不含。

所以「从外部拿到 token 再连」这条路走不通。

### 3.2 持久密钥可以自签

DSH 在 `~/.dsh/.credentials.yaml` 里存了**持久签名密钥**：

```
records:
  client-connection/browser-session:
    kind: grant
    payload:
      version: 1
      secret: <base64url 32 字节>
```

对照服务端实现（`@deepseek-ai/dsh-client-connection/lib/index.js`）：

| 位置                                | 内容                                                                                                              |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `COOKIE_PREFIX`（:277）             | `dsh-auth-`                                                                                                       |
| `canonicalSecret()`（:317-322）     | secret 必须是 base64url **解码后正好 32 字节**                                                                    |
| `cookieName(authority)`（:335-337） | `dsh-auth-` + base64url(sha256(authority))                                                                        |
| `encodeCookie()`（:351-357）        | `v{version}.{base64url(JSON)}.{base64url(HMAC-SHA256(secret字节, body文本))}`                                     |
| `requestAudience()`（:307-316）     | authority = Host 头（http 取 `url.host`，即 `host:port`，**无 scheme**）                                          |
| `authenticate()`（:559-569）        | `payload.authority == authority` 且 `issuedAt <= now < expiresAt` 且 `expiresAt - issuedAt <= maxAgeMilliseconds` |
| `admit()`（:742-750）               | v1 cookie 直接放行成 `browserPeer`；v2 才是 API operator ⇒ **v1 浏览器 cookie 对 API 同样有效**                   |
| `cookieMaxAgeDays`（:1017, :1030）  | 默认 30 天（上限）                                                                                                |

于是可以自己签一张等价 cookie：

- cookie 名：`dsh-auth-` + base64url(sha256(authority))
- cookie 值：`v1.<base64url(JSON)>.<base64url(HMAC-SHA256(secret解码字节, body文本))>`
- body JSON：`{version:1, authority, issuedAt, expiresAt}`，有效期 30 天

**实测**：自签 cookie 访问远程的 `:45239`（千寻自己那个）与 `:3080`（用户那个）**都回 200**；
不带 cookie 一律 401，正文含 `dsh web authentication required`。

> 坑：用 secret **字符串**签 → 401。必须用 base64url 解码后的 32 字节。

### 3.3 踩过的坑：时间戳单位

自签 cookie 最初一律 401。定位到原因是 **时间戳写成了秒**：

- 服务端拿 `Date.now()`（毫秒）比 `issuedAt/expiresAt`，秒级时间戳被判成「早已过期」；
- 修复：`src-tauri/src/harness/dsh_share.rs` 的 `COOKIE_TTL_MILLISECONDS` +
  `now_millis()`，并钉了两条回归单测（`时间戳是毫秒不是秒`、`有效期是毫秒且不超过服务端上限`）。

---

## 4. 实现

### 4.1 探测：`src-tauri/src/harness/dsh_share.rs`

```
pub struct SharedInstance { pub origin: String, pub cookie: String, pub pid: Option<u32> }
pub async fn detect(exclude_ports: &[u16]) -> Option<SharedInstance>
pub async fn issue_verified_cookie(origin: &str) -> Option<String>
```

流程（**全程只读**：不杀进程、不写文件、不启动进程）：

1. 读密钥：`DSH_HOME` 环境变量优先、再 `~/.dsh`，去重（DSH_HOME 优先是必须的：
   千寻嵌套在 DSH 会话里启动时，DSH_HOME 会指向会话私有 home，签错密钥）。
2. 枚举监听端口：Linux 读 `/proc/net/tcp{,6}`（状态 `0A`）；Windows 跑
   `netstat -ano -p tcp`（`proc_guard::hide_console` 防止闪控制台窗）；
   其它平台如实返回空。
3. 裁剪候选（上限 128）：常用 dsh 端口（3080/17300/3000/8080）**永远保留** →
   临时端口（≥32768）降序 → 其余固定端口从低到高。
4. **并发**探测（`join_all`，每端口 1.5s 超时，墙钟≈1.5s，与端口数无关）：
   状态 401 且正文含 `dsh web authentication required` 才算命中。
5. 逐把密钥自签 → 带 cookie 复验 200 → 命中即返回；多命中取端口号最小者（结果稳定）。
6. 排除自己的回环网关端口（见下）。

### 4.2 关键避坑：别把自己当成可复用实例

千寻的回环网关（默认 23091）会把请求**转发给上游 DSH**；上游没带 cookie 时回的正是
那句 `dsh web authentication required`。不排除它，千寻会接上**自己**——自环。

因此 `detect(exclude_ports: &[u16])` 由调用方传入千寻自己的网关端口
（`src-tauri/src/harness/commands.rs`：`dsh_share::detect(&[settings.remote.port])`）。

### 4.3 挂载共享实例：`src-tauri/src/harness/supervisor.rs`

- `Status::Ready` 新增 `shared: bool`（前端据此区分「自己拉起」与「复用别人的」）。
- `Supervisor::attach_shared(instance, browser_url)`：
  - 已 Ready 直接返回原 origin；
  - 写 `shared_cookie`（凭据不外泄到前端，只随网关转发用）；
  - 发布 `Status::Ready{origin, url, pid, shared: true}`，日志「[DSH] 已连接到正在运行的实例（…）」；
  - 起 `watch_shared()` 看门狗：10s 一探，连续 3 次探不到 → 清 cookie、发 `Status::Failed`；
    **绝不替用户重启，更不 kill 别人的进程**。
- `stop()` 只清 cookie、断开连接——共享进程不是千寻的，不归它管。

### 4.4 接线：网关与上游

- `src-tauri/src/remote/gateway.rs`：`start()` 增 `dsh_cookie: Option<String>`；
  有 cookie 走 `Upstream::with_shared_cookie(...)`（日志「网关使用自签 cookie 连接共享的 DSH 实例」），
  没有才走原来的「拿 dsh_url 换 cookie」。
- `src-tauri/src/remote/commands.rs`：`fingerprint()` 把 cookie 算进指纹（换实例/重签要重建网关）；
  `sync()` 里 `shared` 为真时 **dsh_url 必须置 None**（否则网关会拿自己的回环地址去「兑换」cookie）。
- `src-tauri/src/dsh_upstream.rs`：`Inner.self_signed` 标记「这张 cookie 是自签的，
  失效时只能重签而不是兑换」；`issue_cookie()` 分流。

### 4.5 启动路径：`src-tauri/src/harness/commands.rs`

`start_locked()` 的顺序改为：

1. `dsh_share::detect(&[settings.remote.port]).await`——**两种来源模式都做**
   （内置托管也会跟用户手起的实例抢同一把会话锁）；
2. 命中 → `attach_shared(...)`，并**跳过** `after_successful_start()`
   （known-good 快照与安全 profile 预热都会写用户共享的 `~/.dsh`——那是别人的资产）；
3. 未命中 → 照旧 `launch_plan_for()` + `supervisor.start(plan)` 自己拉一个。

安全模式（`harness_safe_mode_start`）保持自拉最小 profile，不复用。

### 4.6 前端

- `src/lib/ipc/contract.ts`：`HarnessStatus` 的 ready 变体加 `shared: boolean`；
  `formatHarnessStatus` 共享时显示「运行中 · {origin}（复用已在运行的实例）」。
- `src/features/env/EnvPage.svelte`：共享时额外一行提示
  「已复用本机正在运行的 DSH（origin · pid N）：千寻不接管它的启停——「停止」只是断开连接，不会结束那个进程。」

---

## 5. 安全边界（逐条）

| 边界                    | 实现位置                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------ |
| 不 kill 别人的进程      | `attach_shared` 不持有进程句柄；`stop()` 只清 cookie；`watch_shared` 失联只报 Failed |
| 不写用户共享的 `~/.dsh` | 共享分支跳过 `after_successful_start()`（快照 + 安全 profile 预热）                  |
| 探测全程只读            | `dsh_share.rs` 只发 GET，不写文件、不启进程                                          |
| 探测不走系统代理        | `probe_client()` 用 `.no_proxy()` + 不跟随重定向                                     |
| 不把千寻自己接上自己    | `detect(exclude_ports)` 排除自己的网关端口                                           |
| 密钥/cookie 不出本机    | cookie 只存 Rust 侧 `shared_cookie`，只随网关转发                                    |

---

## 6. 验证

### 6.1 本地 Windows（`pnpm dev`，debug 二进制）

- 本机 DSH web：pid 17772，监听 `127.0.0.1:63348`（本会话的 DSH）。
- `harness_status` → `{"phase":"ready","origin":"http://127.0.0.1:63348","url":"http://127.0.0.1:23091","pid":17772,"shared":true}`
  ⇒ 千寻**没有**自拉 DSH。
- 应用日志：「[DSH] 已连接到正在运行的实例（127.0.0.1:63348）」、
  「网关使用自签 cookie 连接共享的 DSH 实例」。
- 网关首页 `http://127.0.0.1:23091/` → **200 + DSH 首页 HTML**（服务端持有的 cookie 被上游接受）。
- 千寻 DSH 页 iframe（`src=http://127.0.0.1:23091/`）里 **DSH Web UI 完整渲染**
  （截图 `.tmp/shots/shared_dsh_page.png`：会话列表、对话内容、状态栏「DSH 运行中 · http://127.0.0.1:63348」）。
- **停止安全性**：`harness_stop` → `{"phase":"stopped"}`；
  进程外复验 `http://127.0.0.1:63348/` 仍回 401（= 进程还活着，只是要 cookie），
  `tasklist` 确认 pid 17772 仍在；再 `harness_start` 又接回同一实例（`shared=true`、同 origin）。

### 6.2 远程 omarchy（CEF 内嵌 Chromium，外部 DSH 模式）

- 环境：192.168.20.3 / Arch / hyprland+XWayland；用户的 `dsh web`（foot 启动，pid 318480，`:3080`）；
  `~/.dsh/.credentials.yaml` 有 32 字节 secret。
- 结果见 §6.3。

### 6.3 远程实测结果（192.168.20.3，omarchy/Arch，CEF 内嵌 Chromium，外部 DSH 模式）

编译：`cargo build --features tauri/custom-protocol` → exit 0（新增 `hmac v0.12.1`）。
重启（只杀我们自己的进程树，用户的 `dsh web` 不动）：清理后 `ps` 里
**只剩用户的 `bin.js web`（pid 318480）**，我们拉起的 `--no-open` 子进程与它的
billion-context / playwright 子孙全部清掉。

| 判据                | 结果                                                                                                                                                    |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ① `harness_start`   | 返回 `"http://127.0.0.1:3080"`（用户那个实例的 origin）                                                                                                 |
| ① `harness_status`  | `{"phase":"ready","origin":"http://127.0.0.1:3080","url":"http://127.0.0.1:23091","pid":318480,"shared":true}`                                          |
| ② 是否自拉 DSH      | **没有**：`ps` 里只有用户的 `bin.js web`，没有任何 `--no-open` 子进程                                                                                   |
| ③ 用户 DSH 是否被杀 | 否，pid 318480 存活（5 小时 49 分那条老进程原样还在）                                                                                                   |
| ④ 千寻网关首页      | `http://127.0.0.1:23091/` → **HTTP 200，41856 B**，正文是 DSH 首页 HTML                                                                                 |
| ⑤ DSH 页 iframe     | `src=http://127.0.0.1:23091/`、1052×674，渲染出完整 DSH Web UI（截图 `.tmp/shots/remote_shared_dsh.png`，状态栏「DSH 运行中 · http://127.0.0.1:3080」） |
| 应用日志            | 「[DSH] 已连接到正在运行的实例（127.0.0.1:3080）」+「网关使用自签 cookie 连接共享的 DSH 实例」                                                          |

**会话锁冲突的结构性解决**：远端现在**只有一个** DSH 实例——用户的那个。
千寻不再开第二个 DSH，也就不存在第二个去抢 `session.lock` 的进程；
原来的 “This session is already in use” 失去产生条件。

> 远端的一个细节：千寻启动后 DSH 处于 `stopped`，需要显式 `harness_start`
> （本地 dev 实例会自启）。复用逻辑发生在 `harness_start` 路径上，与来源模式无关。

---

## 7. 限制与未闭环

1. **DSH_HOME 必须一致**：自签 cookie 只在「在跑的实例与本机读到的密钥同源」时有效；
   用户换过 `DSH_HOME` → 签出的 cookie 被拒 → `detect()` 返回 None → 退回自拉实例（安全失败，不会连错）。
2. **依赖 DSH 的响应文案**：判据串 `dsh web authentication required` 变了就探不到（保守失败）。
3. **端口上限 128**：监听端口极多、且 dsh web 落在临时端口段深处时可能探不到（可调 `MAX_CANDIDATES`）。
4. **版本显示**：共享模式下环境页显示的仍是千寻自带的 DSH 版本号，不是共享实例的版本。
5. **「在浏览器打开」**：共享模式下给的是千寻自己的网关地址（浏览器直连需要 cookie，故走网关）。
6. **Linux 反查 pid**：靠 `/proc/net/tcp` 的 inode 扫 `/proc/*/fd`；root 下可能读到他人 fd（只取匹配 inode 的第一条，结果稳定）。

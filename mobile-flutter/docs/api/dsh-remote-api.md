# 千寻移动端 · DSH Remote API 接口文档

> 真源：`E:\develop\dsh-workspace\deepseek-harness`
> 采集：2026-10-06，直读 `packages/typert/**` + `packages/api/**` + `packages/*/src/index.ts`
> 状态：**141 个 `@Remote` 方法 / 29 个服务 / 29 个命名空间**，
> 其中 133 个在标准客户端装配里，8 个（`speech` / `claudeCodeMods`）为实验性。

---

## 0. 注册与派发机制

**没有手写方法表。** 方法由 TypeScript 装饰器声明，构建期分析器生成 wire 描述符。

| 层 | 位置 | 作用 |
|---|---|---|
| 装饰器 | `packages/typert/protocol/src/index.ts:198` `@Remote`、`:250` `@RemoteScope` | 在原型上打 v1 marker |
| 命名空间绑定 | `typert/protocol/src/index.ts:152,166` | `namespace = options.namespace ?? serviceKey` |
| 构建期分析器 | `typert/generator/src/analyzer.ts:1085-1234` | 生成 `InvocationDescriptor`，**决定 wire 键名** |
| 宿主派发 | `api/gateway/src/index.ts:233,357,386,699,756` | 端点 = `` `${namespace}/${method}` `` |

**wire 键名规则**（`analyzer.ts`）：
- JSON 参数 → `wire: 参数名`（`:1140`）
- lookup 参数 → `wire: \`${lookup.key}Id\``（`:1129`）
- 末位 `signal: AbortSignal` → **从 wire 移除**（`:1102-1113`）
- 类型含 `undefined` 的参数可省略（可省略 ≠ 必填）

**生产环境登记的 lookup 重命名**（全仓库仅这 3 处）：
`agent` → `agentId`（`core/agent/src/index.ts:258`）、
`session` → `sessionId`（`core/session/src/index.ts:951`）、
`workspaceFileScope` → `workspaceFileScopeId`（`api/workspace-files/src/index.ts:203`）。

⚠️ **`@RemoteScope` 在生产代码中零使用**（只在生成器测试夹具里）。
所以对非 Cordis 客户端来说，`agentId` 永远是 `args` 里的**必填**字段。

---

## 1. 线格式（必须逐字匹配）

### 1.1 一元调用
```http
POST /api/<namespace>/<method>
content-type: application/json
```
```jsonc
{
  "type": "client-request",
  "rpcId": "<uuid>",
  "method": "<namespace>/<method>",
  "payload": { "args": { <wireField>: <value> } }
}
```

- `payload` 的**唯一**合法键是 `args`（`gateway/src/index.ts:1138-1145`，别的键直接拒）。
- `args` 里的键是**参数名**。例如签名是 `async prompt(request, …)` 的方法，
  线上的键就是 `request` —— **不是**把 request 的字段摊平。
- 多余键 → `gateway/arguments-invalid`；缺必填 → 同样被拒（严格模式）。
  可选参数应**省略该键**，不要发 `null`。
- 端点必须**恰好两段**，每段匹配 `/^[A-Za-z0-9_$.-]+$/`。

> ⚠️ `packages/extensions/tool-cordis/src/api-catalog.ts` 是**手写目录字符串**，
> 与装饰器一致但**不是契约**。派发路径用的是装饰器 + 生成的描述符。

### 1.2 一元响应
```jsonc
{ "type":"server-response", "rpcId":"<同>",
  "result": { "ok":true,  "value": <json> } }
{ "type":"server-response", "rpcId":"<同>",
  "result": { "ok":false, "error": { "code", "message", "details":{} } } }
```
- `void` 结果**整个省略 `value`**（`gateway/src/index.ts:689`）。
- 含 `Uint8Array` 的结果：字段变 `null`，字节作为附件随
  `content-type: multipart/form-data` 的多个 part 发送（`client/connection/src/client/rpc.ts:83-139`）。
- **非 2xx HTTP = 传输失败**，不是业务结果。

### 1.3 流（WebSocket，路径恰好 `/api/remote.mux`）
- 客户端→宿主：`{type:'open', streamId, endpoint, payload:{args:{…}}}` /
  `{type:'item', streamId, value?}` / `{type:'end'|'cancel', streamId}`
- 宿主→客户端：`{type:'item', streamId, value?}` /
  `{type:'error', streamId, error:{code,message,details}}` / `{type:'end', streamId}`
- 服务端每 2000ms ping；上行收件箱上限 262144 字节/流。
- 一元方法发到 mux → `gateway/signature-invalid`，反之亦然。

### 1.4 鉴权（会挡住天真客户端）
- `GET /?token=<launchToken>` → `303` + `Set-Cookie: dsh-auth-<…>=v1.<payload>.<hmac>`
- 每个 `/api` 与 `/api/remote.mux` 请求都必须带该 cookie，且 `Host` 必须是回环
  或在 `trustedHosts` 里配置过的 authority（`api-request-trust.ts:75-118`）。
- 有 `Origin` 时必须等于 Host authority；`Sec-Fetch-Site: cross-site` 被拒。
- **没有 API key / bearer token 通道。** launch token 只存在于 `dsh web` 打印的 URL 里。

### 1.5 版本协商：**没有**
线路上无 `protocolVersion`、无 hello、无能力交换、无服务端方法清单。
可用性只能**调用后**从 `gateway/invocation-unavailable` /
`gateway/method-unavailable` / `gateway/service-unavailable` 判断。

---

## 2. 方法清单（按命名空间）

`*` = 流方法。`agentId` 即 `SessionId` 字符串。参数按 wire 顺序。

### 2.1 `session` —— 移动端核心
`api/session-controller/src/index.ts`

| 方法 | 行 | `args` | 返回 |
|---|---|---|---|
| `session/list` | 258 | `_request` | `{items:SessionSummary[]}` |
| `session/search` | 269 | `request` | `{items,hasMore}` |
| `session/create` | 279 | `request{workspaceId?,cwd?,sessionId?,agentPreset?}` | `{sessionId,agentPreset?}` |
| `session/selectModel` | 289 | `request{provider,model,reasoningEffort?,sessionId}` | `{selected}` |
| `session/initializeDefaultModel` | 298 | — | `void` |
| `session/modelCatalog` | 316 | — | `ModelCatalog` |
| `session/canOpenWorkspacePath` | 325 | — | `boolean` |
| `session/openWorkspacePath` | 346 | `request{action?,application?,path}` | `{opened:true}` ⚠️**仅桌面** |
| `session/workspacePathApplications` | 376 | `request{path}` | `[]` ⚠️**仅桌面** |
| `session/rename` | 409 | `request{sessionId,title}` | `{title,seq}` |
| `session/fork` | 421 | `request{sessionId,atSeq?}` | `{sessionId}` |
| `session/prompt` | 432 | `request{requestId,sessionId,mode:'queue'\|'steer',content,clientTimeZone?}` | `{accepted:true}` |
| `session/attachment` | 443 | `request{sessionId,attachmentId}` | `{attachment,data /*base64*/}` |
| `session/updateQueue` | 453 | `request{sessionId,itemId,action:{kind:'edit'\|'remove'\|'steer'}}` | `{accepted:true}` |
| `session/cancel` | 463 | `request{sessionId}` | `{accepted:true}` |
| `session/page` | 474 | `request{address,throughSeq,beforeSeq?,maxMessages?,turnWindow?}` | `{records,hasMore}` |
| `session/follow` * | 486 | `request{address,maxMessages?,turnWindow?,assistantStream?:true}` | `snapshot` / `event` / `assistant-stream` 帧 |
| `session/projections` | 497 | `request{sessionId}` | 基线 或 `null` |
| `session/control` * | 526 | — | `baseline`/`projection` 帧（宿主全局） |

`session/prompt` 的 `content` 三种 part：
`{type:'text',text}` / `{type:'image',mediaType,data,name?}` / `{type:'file',receiptId}`。

`address` 两种：`{kind:'session',sessionId}` 或
`{kind:'subagent',parentSessionId,childSessionId,mode}`。

### 2.2 其余命名空间（移动端相关性分级）

| 命名空间 | 方法 | 移动端相关性 |
|---|---|---|
| `userQuestions` | `answer{agentId,callId,answer}`、`attachWait*` | ⭐⭐⭐ **除 waterfall 外还有一元应答通道** |
| `commands` | `list{agentId}`、`execute{agentId,line,submittedAttachments}` | ⭐⭐⭐ 斜杠命令 |
| `fileReferences` | `list{agentId,query}` | ⭐⭐ `@`-mention 候选 |
| `fileUploads` | `upload{agentId,request{data /*base64*/,name?}}` | ⭐⭐⭐ **base64 上传，与我们的裸二进制路由并存** |
| `sessionReferenceResolver` | `candidates{agentId,query}` | ⭐⭐ |
| `permissionPresets` | `catalog` | ⭐⭐⭐ 权限档位目录 |
| `agentPresets` | `list` / `read` / `select{agentId,agentPreset}` | ⭐⭐ |
| `goals` | `get/edit/pause/resume/complete/clear/create{agentId,ref,request}` | ⭐⭐ |
| `subagents` | `prompt{parentSessionId,childSessionId,…}`、`interruptByParent` | ⭐⭐ |
| `job` | `list*{sessionId}`、`follow*{sessionId?,jobId,from?}`、`kill` | ⭐⭐ |
| `workspaceFiles` | `read` / `readBytes` / `stat` / `list` / `changes*` | ⭐⭐ |
| `schedule` | `list/catalog/history/delete/update` | ⭐ |
| `llm` | `listProviders` / `listConfigurableProviders` / `discoverModels` | ⭐ |
| `account` | 12 个，含 `watchExpiry*` / `watch*` | ⭐ |
| `settings` / `credentials` | 读用 `redactSecrets:true`；**任何方法都不返回密钥值** | ⭐ |
| `messageFeedback` / `sessionFeedback` | 赞踩 | ⭐ |
| `workspace` | 增删改查 + `archiveSession` / `pinSession` / `follow*` | ⭐⭐ |
| `officeToPdf` | `render` | ⭐ |
| `dynamicCordisRunner` | 12 个动态插件方法 | ☆ |
| `pluginManager` | 12 个（**会改宿主磁盘配置、跑 pnpm install**） | ☆ |
| `productAnalytics` / `pluginInventory` / `pluginRegistryProbe` | — | ☆ |
| `terminal` | 10 个 ⚠️ 见 §3 | ☆☆ 建议**不做** |
| `directoryPicker` | `pick` / `list` / `createDirectory` | ☆ |
| `skills` | `list{sessionId}` | ⭐ |

实验性（需对应客户端包，宿主不门禁）：
`speech/*`（6）、`claudeCodeMods/*`（2）。

---

## 3. ⚠️ 三个安全发现（移动端必读）

### 3.1 `terminal/*` = 宿主上的远程代码执行
`api/terminal-controller/src/index.ts:151` 注释明说：终端**以操作系统用户权限
启动真实宿主进程，刻意绕过 agent 沙箱与审批流程**。

也就是说，拿到 `terminal/create` 的人可以在用户电脑上跑任意命令，
**且这条路径不经过 `approval/request`**——也就是说它**不会**出现在
「等你回答」里，用户看不到自己刚刚放行了什么。

**建议**：移动端不实现终端。参考客户端实现了（`TerminalPanel.kt`），
但那是一个我们不该照搬的决定。

### 3.2 `workspaceFiles/read` 允许读工作区**以外**的路径
`api/workspace-files/src/index.ts:1-20` 显式允许工作区根之外的路径
（`list` / `changes` 才有工作区限制）。手机客户端继承了这个暴露面，
**不能**把它当成沙箱化的预览 API。

### 3.3 `pluginManager/*` 改磁盘配置并装包
会写 profile、跑 `pnpm install`、重载插件。移动端不应提供。

### 3.4 其它边界
- 冷加载（未使用）的 Service 没问题；**未挂载的插件**调它会失败
  （`gateway/service-unavailable`）。
- `commands/find` 与 `session/workspaceDesktop` **不是 Remote**（无装饰器），
  尽管看起来可调。
- 141 个方法里**只有 133 个**在标准客户端装配中。

---

## 4. 错误码

统一 `{code, message, details}`。

**Gateway**（`api/gateway/src/types.ts:116-135`）：
`bad-request` / `cancelled` / `internal` / `ambiguous-endpoint` /
`arguments-invalid` / `binding-invalid` / `context-failed` / `context-not-found` /
`context-unavailable` / `definition-unavailable` / `input-invalid` /
`invocation-unavailable` / `lookup-failed` / `lookup-not-found` /
`lookup-unavailable` / `method-unavailable` / `protocol` / `provider-mismatch` /
`result-invalid` / `service-unavailable` / `signature-invalid` / `uplink-overflow`

**领域**（节选）：
`session/{not-found,conflict,agent-busy,title-invalid,model-unavailable,attachment-invalid,queue-item-not-found,projections-unavailable,provider-credentials-unavailable,provider-models-unavailable,steer-unavailable,writer-held}`、
`workspace/not-found`、`workspace-file/{not-found,outside-workspace,too-large,not-text,not-regular-file,not-directory,watch-unsupported}`、
`terminal/{unavailable,limit-reached,control-unavailable}`、`job/not-found`、
`subagent/{not-found,unauthorized,attachment-invalid,catalog-diagnostic,invalid-time-zone}`、
`agent-preset/{not-found,locked,invalid,conflict}`、`settings/{rejected,conflict}`、
`credential/rejected`、`directory-picker/{unavailable,unreadable,exists,create-failed}`、
`llm/model-discovery-rejected`、`speech/{invalid-audio,transcription-failed}`、
`document-render/failed`

⚠️ 逐码 `details` 形状**未完全穷举**（未逐个读 `declare module … RemoteErrorDetailsMap`）。

---

## 5. 未验证

- 逐码 `RemoteErrorDetailsMap` 的完整条目。
- 未逐个打开的请求/响应类型字段：`AccountView`、`ScheduleUpdateRequest`、
  `SubagentPromptRequest.delivery`、`JobView` 内部形状、`DynamicCordis*` 全家。
- **本部署注册的是严格描述符还是 SRC 回退模式** —— 两者端点与 wire 名相同，
  但严格模式额外强制必填/类型校验。取决于宿主是否加载了各包的生成贡献，
  **需要看一个活的 `dsh web` 进程**。
- 未修改任何代码。

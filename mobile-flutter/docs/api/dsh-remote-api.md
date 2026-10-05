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

#### `session/prompt` 的 `requestId` 是**真的去重**（源码级验证）

`packages/api/session-controller/src/commands.ts:330`

```ts
const agent = await this.resolveAgent(request.sessionId)
if (hasPromptRequest(agent, request.requestId)) return { accepted: true }
```

`hasPromptRequest`（`:602-614`）查**两处**，都命中才算重复：

1. `agent.inbox.nextTurn` / `agent.inbox.nextStep` —— 未消费的队列
2. `agent.session.snapshotEvents()` 里 `type === 'user/message'` 且
   `data.source.rpcId === requestId` —— 持久化日志

且 `requestId` 被原样写进消息源（`:333` `source.rpcId = request.requestId`），
所以这是**同一个值**在比对，不是派生值。

**测试覆盖**（`tests/commands-upload-file.host.spec.ts`）：

| 行 | 场景 | 断言 |
|---|---|---|
| `:345` | 重试的 rpcId 已在 inbox | `followup` 只调一次 |
| `:354` | 重试的 rpcId 已在持久化日志 | `followup` **没被调** |

**对客户端的含义**：超时后用同一 `requestId` 自动重试**是安全的**，
不会产生两条轮次。⚠️ 但去重路径会走 `resolveAgent`（`agent.ts:170`
「Resolve **or resume**」），会话不在内存时会**触发一次 resume**。
移动端应设**退避 ≥ 2s、最多 2 次**。

### 2.1.5 `assistant-stream` 的帧形状（源码级确认）

`SessionAssistantStreamFrame`（`session-controller/src/types.ts:513-543`）
是三态联合，**没有 `delta` 字段**：

| `type` | 字段 |
|---|---|
| `start` | `attemptId, revision, startedAfterSeq, turn, step` |
| `chunk` | `attemptId, revision, index, time, chunk: JsonValue` ← **不透明** |
| `end` | `attemptId, revision, index, outcome{committed{eventType,seq} \| abandoned}` |

`chunk` 装的是**压缩 record 流**（`llm/src/assistant-stream.ts:20-44`）：

| record `type` | 载荷 | 展开为 |
|---|---|---|
| `text-chunks` | `{time0, index, dt[], texts[]}` | `text-delta` × N |
| `reasoning-chunks` | `{time0, index, dt[], texts[]}` | `reasoning-delta` × N |
| `tool-call-chunks` | `{time0, index, dt[], id, name?, args[]}` | `tool-call-delta` × N |
| `chunk` | `{time, chunk}` | `block-start`/`block-end`/`usage`/`finish` 原样 |

⚠️ **连续 delta 被压成「一整个 run」** —— 一个 record 内有 N 个片段。
不能写成「遍历帧取 `frame.chunk.text`」：那会把 N 段当 1 段。
权威展开器是 `expandAssistantStream()`（`llm/src/assistant-stream.ts:202-232`），
**Dart 端需要等价实现**。

**三条服务端强制的客户端约束：**

1. `revision` 严格 +1。跳号 → `RemoteStreamCarrierError`
   `session assistant stream skipped revision N`（`client/transport.ts:208-211`）
2. `index` 严格 +1。不连续 → 该 attempt 作废 + `rebaseline`
   （`client/sessions/assistant-stream.ts:149`）
3. `end.outcome.seq` 是权威。**不要用 chunk 数推断轮次结束** ——
   `end.index` 是「本帧代表的 chunk 帧数」（`types.ts:534`），不是消息长度。

流式行是**瞬态**的，最终真相在 `assistant/message` 事件上：
成功消息的瞬态行留到 owning Step 结束才撤（`client/sessions/assistant-stream.ts:190`），
`interrupted === true` 的立即撤。**渲染时不能把流式内容当最终态提交。**

### 2.1.6 待答信号的覆盖边界（⚠️ 有确凿缺口）

`UserQuestionProjectionView`（`interaction/user-questions/src/types.ts:100-105`）：

```ts
{ active: readonly PendingUserQuestion[]   // 仍可作答，按提问顺序
  settled: readonly SettledUserQuestion[] } // 已被答复结算
```

每项 `PendingUserQuestion` = `{callId, questions[], state}`，
`state: 'open' | 'continued'`（`:72`）—— `open` 是工具调用仍可能自己返回答案，
`continued` 是答案只能作为新一轮消息到达。

**✅ 覆盖**：`ask_user_question` 在 `session/list` 的 `projections.values`
里有列表级信号，且带 `callId`。参考客户端就是这么用的
（`client/ui-user-questions/src/client/index.ts:315-331`），
**不依赖实时 waterfall**。

⚠️ 还要交叉读 `projections.inbox` 的 `next-step` / `next-turn`：
里面 `source.kind === 'user-question-reply'` 且 `callId` 命中的，
表示**答复已提交但还没被 agent 消费**，此时应把可编辑卡片撤掉
（`index.ts:320-328`）。

**❌ 缺口**（`types.ts:95-98` 原文）：

> Both halves of one Session's timed `ask_user_question` state, as every
> Client reads them. **A call made while the blocking legacy tool was in
> effect appears in neither half.**

**无 `callId` 的阻塞式交互，在 `active` 与 `settled` 里都不出现。**
参考客户端对此的处理是直接删卡片
（`ui-user-questions/src/client/index.ts:275-276`）：
`if (callId === undefined) card.remove()`。

**对移动端的含义**：「等你」区必须三源合流 ——
① 投影 `userQuestions.active`（主源，可直接答）
② 实时 waterfall（覆盖无 `callId` 的阻塞交互）
③ 本地持久化的待答快照（覆盖 App 重启后 waterfall 收不到事件的情况）
**且必须区分「可答」与「仅快照」** —— 后者只能置灰，
因为 `callId` 可能已消失，点了发不出去。

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

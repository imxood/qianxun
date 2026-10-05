# 千寻移动端 · 调研纪要（第一轮）

> 本文记录**读源码与截图得到的事实**，不是设计决策。
> 设计决策在 `05-product-design.md`，本文只负责「先别设计，先搞清楚」。
>
> 采集时间：2026-10-06
> 采集方式：直读源码 + 直看截图。**每条都带出处。**

---

## 0. 参考项目坐标

| 路径 | 是什么 | 在本次调研里的角色 |
|---|---|---|
| `E:\develop\dsh-workspace\deepseek-harness` | TypeScript monorepo，~4700 `.ts` | **协议与 API 的权威来源** |
| `E:\develop\dsh-workspace\deepseek-harness-mobile` | Kotlin/Compose，`com.labteto.dshmobile` | 同协议的移动端参考实现 + 5 张真实截图 |
| `E:\develop\dsh-workspace\dsh-mobile` | 另一个移动项目（有 `design-system/`） | 待确认是否同产品不同栈 |
| `E:\develop\dsh-workspace\qianxun\mobile` | 旧 React Native 客户端 | 功能覆盖清单（防漏项） |
| `E:\develop\dsh-workspace\qianxun\mobile-flutter` | 我们在写的 | — |

`deepseek-harness` 里与 API 相关的包：
`api`（`remotes` / `session-controller` / `workspace-controller` / `job-controller` /
`terminal-controller` / `settings-controller` / `account-controller` / `gateway` /
`workspace-files`）、`sdk/protocol`、`typert/protocol`。

---

## 1. 协议架构（三层）

```
Typert 协议  ── 生成 ──▶  Remote 贡献声明
     │                        │
     │                   packages/api/remotes   ← 唯一的选择与挂载点
     │                        │
     └──▶ API Gateway  ──▶  端点 / 载体 / 取消 / 重连
                              │
              Cordis 事件 ───┤───▶ 两类投递
                              │
                  emit（单向，不重放）
                  waterfall（请求-响应，Agent 作用域，可重放）
```

关键源码：
- `packages/api/remotes/src/index.ts` — Host 侧事件源与 waterfall 转发
- `packages/api/remotes/src/remote-events.ts` — **转发白名单（唯一真源）**
- `packages/api/remotes/README.md` — 架构说明与已知限制

---

## 1b. 线格式（`packages/typert/protocol/src/types.ts`）

| 事实 | 出处 |
|---|---|
| `RemoteResult<T> = {ok:true, value:T} \| {ok:false, error:RemoteFailure}` | `types.ts:76-78` |
| 端点名为 `namespace/method` | `types.ts:220-224` |
| 作用域端点为 `ContextKey:Namespace/method` | `types.ts:242-243` |
| 失败码是可扩展联合 `RemoteErrorDetailsMap`，本包先声明载体级：`gateway/bad-request`、`cancelled` | `types.ts:49-56` |
| 流 = `RemoteStream<Out, In>`，客户端拿到 `RemoteStreamHandle<Out,In>`：下行 `AsyncIterable` + 上行 `uplink(item)` + `end()` + `dispose()` | `types.ts:93-124` |
| 每个参数在 wire `args` 里有**必需键名** | `types.ts:295-308` |

**与我们 Batch 1 实测一致**：`{ok,value}` 信封、`/api/<ns>/<method>` 路径。
尚未验证的是**内层参数键名**（我们探针发现 `session/list` 是 `_request`、
其余是 `request`）——这一点必须在 API 文档里逐方法列明，不能一概而论。

---

## 2. 转发事件白名单（24 条，其中 waterfall 只有 2 条）

出自 `packages/api/remotes/src/remote-events.ts:20-48`：

| 事件 | 模式 |
|---|---|
| `approval/request` | **waterfall** |
| `user-questions/request` | **waterfall** |
| `api-session/added` | emit |
| `api-session/removed` | emit |
| `api-session/status` | emit |
| `api-session/activity` | emit |
| `api-session/error` | emit |
| `agent-preset/selected` | emit |
| `commands/change` | emit |
| `credentials/record-updated` | emit |
| `credentials/reference-updated` | emit |
| `deepseek-account/session-expired` | emit |
| `deepseek-account/model-sign-in-required` | emit |
| `goal/activation-changed` | emit |
| `cordis/request-run` | emit |
| `cordis/request-run-resolved` | emit |
| `cordis/dynamic-package` | emit |
| `cordis/dynamic-retract` | emit |
| `cordis/inspect-query` | emit |
| `cordis/inspect-query-resolved` | emit |
| `llm/adapters-updated` | emit |
| `permission-presets/catalog-changed` | emit |
| `plugin-manager/changed` | emit |
| `plugin-manager/install-log` | emit |
| `plugin-manager/install-state` | emit |
| `settings/document-updated` | emit |
| `schedule/changed` | emit |

### 2.1 emit 与 waterfall 的区别（对移动端影响很大）

`remotes/src/index.ts:52-76`：

- `emit` = 单向通知。**不重放**。
- `waterfall` = 请求-响应，**必须**在 `request.agent` 携带被路由的 Agent
  身份，宿主会**拒绝**身份缺失或不匹配的转发（`index.ts:65-67` 直接抛
  `TypeError`）。

`remotes/README.md:77` 的限制原文：

> Only scoped waterfalls that are still pending are replayed after reconnection;
> ordinary one-way notifications remain isolated best-effort deliveries and are
> not replayed.

**推论**：手机断线重连后，**审批与提问会被重放**（agent 还在等），
但 `api-session/*` 的增删改**不会**。所以会话列表在重连后必须**重新拉一次**，
不能靠事件补。这直接验证了我们当前「`session/list` 全量拉取 + 事件增量」的
设计方向是对的。

---

## 3. 会话面（`session-controller` 客户端契约）

`packages/api/session-controller/src/client/contract/sessions.ts` — `ISessions`：

| 成员 | 说明 |
|---|---|
| `list` | `ObservableSnapshot<SessionListState>` |
| `create({workspaceId, cwd, sessionId?})` | 创建或采纳一个会话 |
| `refresh()` / `refreshProjections(id)` | 刷新列表 / 刷新单个投影 |
| `search(query, signal)` | 搜**消息内容**索引，返 `{items, hasMore}` |
| `fork({sessionId, atSeq?, increaseTitle?, onCreated?})` | **会话分叉**，按 seq 边界 |
| `subagentAddress(id)` | 解析子 agent 地址 |
| `retain` / `using` / `retainInfo` | 引用计数生命周期 |
| `scope` / `scopeOf` / `sessionOf` / `binding` | 作用域与绑定 |

`contract/session.ts` — `ISession`（**我们最需要对齐的一份**）：

| 成员 | 参数 / 返回 | 我们有吗 |
|---|---|---|
| `prompt(content, mode: 'queue'\|'steer', signal?, requestId?)` | `Promise<RemoteResult<{accepted:true}>>` | ✅ Batch 1 实现了 |
| `beginSubmission(input)` | 本地回显 + 铸出 `requestId` | ❌ 没有 |
| `readAttachment(attachmentId)` | 取回图片字节 | ❌ 没有 |
| `updateQueue(itemId, action)` | 编辑/删除/steer **待发队列** | ❌ 没有 |
| `cancel()` | 中断当前轮次 | ✅ 有 |
| `rename(title)` | 重命名 | ❌ 没有 |
| `loadOlder()` | 往前翻（≥50 条，上限 500） | ❌ 没有 |
| `loadThrough(seq)` | **翻到指定 seq**（轮次跳转） | ❌ 没有 |
| `command(line)` | 斜杠命令 | ❌ 没有 |
| `projections.faceOf(key)` | 按 key 读投影 | ✅ 有（但整包读） |

---

## 4. 参考客户端设计系统要点

`deepseek-harness-mobile/app/src/main/java/com/labteto/dshmobile/ui/theme/`

- `Color.kt`：中性阶 17 档 `Bluish00..1000`；品牌蓝 `Deepseek50..900`；
  语义绿/红/琥珀各 3 档；`ContextMeter` 三色（System 灰 / Tools 紫 /
  Messages 蓝）。暗色 `borderL1/L2/L3 = rgba(255,255,255,.06/.12/.16)`
  —— **三档发丝描边，无阴影**。
- `Type.kt`：`16/24 · 14/22 · 13/20 · 12/18 · 11/14`，**全部无
  letterSpacing**，无 mono 数据层。
- 组件：`StateDot` / `ContextMeter` / `DisclosureRow` / `ToolCards` /
  `QuestionComposer` / `DsBottomSheet` / `EmptyHero` / `RelativeTime`。

`StateDot.kt:31` 的运行中表现是 **3×3 像素矩阵追逐**（9 帧 / 9 秒），
不是一个呼吸的圆点 —— 它编码的是**形状在动**，不是「亮暗在变」。

`ContextMeter.kt:36-40` 有一段三份文档都没有的设计判据：
上下文压力应当放在**输入栏旁**，因为「上下文压力是你写下一条消息时就要
处理的东西，不是一轮跑砸了之后再去翻的」。

---

## 5. 参考客户端截图里被证实的问题

截图位于 `deepseek-harness-mobile/docs/images/`（5 张）与 `screen.png`。

| 问题 | 出处 |
|---|---|
| 会话详情一屏 19 条形态相同的工具行，零结论 | `chat.png` |
| `15 turns · 283 steps · TTFT 4.3s · 80 tok/s` 常驻底栏 | `chat.png` |
| Chat / Trajectory 双顶层 tab | `chat.png` / `trajectory.png` |
| 连接页约 30% 屏高给 logo + 重说 App 名的副标题 | `home.png` |
| 中英混排同列 | `chat.png` 顶栏 |
| `Stopped` 药丸夹在两条工具行之间，无语境 | `chat.png` |
| 12 条子 agent 形态全同 | `subagent.png` |
| 一轮仅 1 步时 Trajectory 约 65% 屏高空白 | `trajectory.png` |

值得抄的：单行工具披露行、`Title :: value` 装置、文件类工具的专属字形、
分段式上下文计量表、输入栏权限芯片、运行中指示点。

---

## 6. 尚未采集（本轮未完成）

- `workspace-controller` / `job-controller` / `terminal-controller` /
  `settings-controller` / `account-controller` 的方法面
- `packages/typert/protocol` 的 schema 与代码生成规则
- 线格式细节：请求信封 / 响应信封 / 事件帧的确切 JSON 形状
- `dsh-mobile` 是否同产品
- 旧 RN 客户端的功能覆盖清单

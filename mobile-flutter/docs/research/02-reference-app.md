# 参考客户端测绘结论

> 来源：agent 直读 `deepseek-harness-mobile` 源码 + 5 张真实截图，
> 交叉 `dsh-mobile` 与旧 RN 客户端。
> 本文只记**结论**，设计决策留给 05。

---

## 0. 三个仓库的真实关系（此前搞错了）

| 仓库 | 是什么 |
|---|---|
| `deepseek-harness-mobile` | Kotlin/Compose，**直连 harness JSON-RPC**，四模块，11 套语言资源 |
| `dsh-mobile` | **同一产品的另一套栈** —— DSH 侧的服务端插件（npm `dsh-mobile`）+ Web/PWA + Android 壳。自带 `src/mobile-layout.ts` 与 `design-system/`（**尚未读**） |
| `qianxun/mobile`（旧 RN） | **不是功能超集**，而是一个 WebView 壳：`surfaces.tsx:20` 就是 `<WebView source={{uri: conn.url}}>` |

**推论**：Flutter 客户端应继承 Kotlin 客户端的**直连模型**，否则会把
`dsh-mobile` 的插件依赖一并继承。

### 旧 RN 相对 Kotlin **多出来**的两样（真实功能回退）
1. **「更新配对链接」**（`HomeScreen.tsx:224`）—— 对已有连接换新 URL。
   Kotlin 只能「忘记 + 重新配对」（`ConnectScreen.kt:178`）。
2. **打开完整 web 客户端的逃生通道**。RN 本质上能显示整个 DSH web UI，
   Kotlin 原生重写没有任何这类入口 —— **一旦漏掉某个 harness 能力，
   手机上就彻底没有了**。

---

## 1. ⚠️ 截图早于代码：EN/CN 混排的根因不是「忘了做本地化」

`ChatLabels.kt:97-117` 已为 4 个内置 preset 建了本地化表
（`values/strings.xml:363` = "Standard mode"，`values-zh-rCN/strings.xml:362`
= "标准模式"）。截图里的中文来自**修复前**的构建。

真正的问题是：宿主从 `preset.yml` 直读 `name`，而那些文件是中文写的，
所以 `agentPreset.list` **无论客户端什么语言**都回「标准模式」
（`ChatLabels.kt:99-104` 明说了）。而这张表**只覆盖 4 个 id，
且只在 `trust == SYSTEM` 时生效**。

→ 需要的不是四行映射表，而是**「宿主字符串与 UI 语言不一致」的通用标记策略**。

---

## 2. 转录的结构性缺陷：分组能力在错的 tab 里

- 转录层把**每个 `ToolCallNode` 独立渲染成一张 `ToolCard`**
  （`ChatNodeItem.kt:408-471`），**没有任何「按 turn / 按阶段聚合」的逻辑**。
- 而 turn 分组**存在**（`ChatProjections.kt:135` 的 `groupByTurn`），
  但只被 `TrajectoryTab.kt:67` 用 —— **那个 tab 平时是空的**。

**结论**：唯一的分组能力被放在了一个大部分时候空着的 tab 里，
而默认 tab 完全没有。

→ **默认转录里，同一 turn 连续的 Bash/Read/Write 应折叠为一行**
`Ran 19 steps · 3 files changed · 12s`，点开才见明细。

---

## 3. 值得照搬的 9 条

| # | 做法 | 出处 |
|---|---|---|
| 1 | 单行工具披露 `Verb :: dots :: summary`，24dp 一行承载一次调用 | `DisclosureRow.kt:63` |
| 2 | 动词来自工具的**变体**而非原始工具名（`web_fetch` → `Read`），图标列与动词列因此始终对齐 | `ToolRowModel.kt:23-31, 82-97` |
| 3 | 路径**相对化到 session cwd**，同时处理 `\` 与 `/` | `ToolRowModel.kt:140-145` |
| 4 | `2×2 点阵`做 title/summary 分隔符（4dp 内不占宽度） | `DisclosureRow.kt:145-157` |
| 5 | ContextMeter 放**输入栏旁**（见 §6 冲突） | `ContextMeter.kt:34-40` |
| 6 | 权限 chip 常驻（高频检查、低频修改 → 可见但不占主操作位） | `Composer.kt:250-255` |
| 7 | 顶栏两行分工：**行一=连接的控制**、**行二=会话的控制**；无内容时整行消失 | `ChatTopBar.kt:53-63, 112-113` |
| 8 | 反向列表 + 常量 streaming key + 自动翻页**硬上限 1 页** | `ChatTranscript.kt:56, 73, 193, 218` |
| 9 | 状态点三态优先级 `running > pendingInteraction > idle`；处理「点停止后最后一帧仍显示在写」的竞态 | `ChatListDrawer.kt:709`、`ChatNodeItem.kt:300-302` |

第 8 条的 OOM 教训值得单独记：一页按**事件**计数，而大多数事件
（chunk delta、工具流量、轮次边界）什么都不渲染，所以**日志密集型会话
永远填不满屏幕**，填充循环就把整个历史拉进来，每轮 4000 事件并重新
fold 全部已持有内容直到堆爆。→ Flutter 侧要用同样的推理预先防住。

---

## 4. 状态处理的规律

**好文案的 11 个空/错态，全部是「用户自己能解决」的失败态**：
扫不到（附三个可检查项）、内容搜索关闭（明说「这是能力说明不是失败」）、
设置只读（说清限制**和**去哪里改）、子 agent 只读（说明为什么输入框不在）、
steering 但 agent 空闲（选项可见但惰性 + 一行 hint）。

**差文案的 8 个，全部是「开发者/作者/审查者层面的事实缺失」**：
`No subagents` / `No background jobs` / `No pending turns` / `No active goal` /
`No recorded requests yet` / `No plugins` …一律 `No X` 模板。

→ **规律**：一个会话没有后台作业是**常态，不是状态**。
这 8 个里**至少 5 个不该是空态而该是不渲染**。

**两个空白的缺口**：
- **重连状态不可见**。`AppRoot.kt:58` 让 `RECONNECTING && hasConnected` 继续显示
  主屏（正确），但屏内**没有任何东西说明正在重连**。`ConnectionManager.kt:56`
  有 `attempts` 计数，**从未渲染**。重连第 1 次与第 47 次在界面上完全一样。
- **agent-detached 与 idle 视觉上不可区分**（都只显示 `StateDot(Idle)`）。

---

## 5. 此前未记录的新缺陷

| # | 缺陷 | 出处 |
|---|---|---|
| a | **工具耗时被渲染成一行孤儿文本**（独立 `Text` 排在 `ToolCard` 之前，不是行尾槽位） | `ChatNodeItem.kt:437-442` |
| b | **Details 面板空态文案过期**，叫用户「点工具行看输入输出」，而那功能已搬到别的 tab | `DetailsPanel.kt:218-223` vs `:99-100` |
| c | **整卡可点 + 卡内嵌 Delete 按钮** —— 破坏性操作嵌套在正向操作的点击区内 | `ConnectScreen.kt:407, 466-471` |
| d | **4 处裸 Material3 组件**泄漏到生产路径（断线横幅被做成按钮样式、两个 `Workspace` 入口、选模式用手写 `"✓ "` 拼串） | `ChatScreen.kt:392,394`、`SheetSubagents.kt:149,153` |
| e | `EmptyHero` 硬编码英文 `"Preview"`，不走 `stringResource` | `EmptyHero.kt:62-70` |
| f | `EmptyHero` 的 `chips` 能力**已写好但两个生产调用点都不传** | `ChatTranscript.kt:180`、`ChatListDrawer.kt:408` |
| g | 搜索能力关闭的提示出现在**打字之后**（应在决定搜索那一刻） | `ChatListDrawer.kt:248` |
| h | `1 steps` 单复数错误（`%1$d steps` 直接当 action 用） | `strings.xml:331` |
| i | **归档会话卡片放在设置页第一个分组**，而它同时在抽屉里有折叠区 —— 同一份数据两个入口 | `SettingsScreen.kt:131` |
| j | **审批只有 Allow once / Reject**，无 always-allow。对远程客户端风险高，但只有 once 意味着每个 `ls` 都要点一次 | `InteractionPanels.kt:80-92` |
| k | `host.lastHome` 是**客户端本地缓存**，探测失败时仍展示，用户会以为是当前状态 | `ConnectScreen.kt:405` |

---

## 6. 与 05 §3.4 正面冲突的判据（仍未裁决）

`ContextMeter.kt:34-40`：

> 上下文压力是你**写下一条消息时就要处理**的东西，不是一轮失败之后再去找的东西。

`chat.png` 证实它**同时**出现在输入栏右侧与详情抽屉。
05 §3.4 判给抽屉。**两条理由都成立**，等拍板。

**新证据**：`projections.values.contextBreakdown` 实测存在
（`{messageTokens, systemTokens, toolsTokens}`）——**数据源已确证**，
只差「放哪」的决定。

---

## 7. 未覆盖

- `dsh-mobile/src/mobile-layout.ts` 与 `design-system/` **未读**（同产品的
  触屏布局契约，可能含遗漏屏幕）
- `ToolCards.kt`(562) / `Markdown.kt`(554) / `QuestionComposer.kt`(560)
  / `theme/*` 未逐行读
- `banner.jpg` / `screen.png` 未看
- 未确认 git 历史，**无法确定哪些缺陷已修、哪些仍在**

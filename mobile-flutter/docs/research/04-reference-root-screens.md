# 参考项目的根屏到底怎么做的（源码级事实）

> 调研来源：4 个仓库的源码与 CSS 常量逐行核对，每条带 `<文件>:<行号>`。
> 未跑过任何实机；`deepseek-harness-mobile` 的五张截图是**英文 locale**。
> 本文的唯一目的是**报告事实**，不含建议。

---

## 0. 一句话结论

> **四个项目里，没有一个把「会话列表」做成根屏主体。**
> 三个 DSH 系项目的根屏都是**单个会话的对话**（列表在侧栏/抽屉里），
> 旧 RN 壳的根屏是**连接列表**。
> 而「要用户处理的事」在**三套实现里一律是行内属性**，不是独立看护带。

---

## 1. 四个项目的根屏结构

| 项目 | 根屏是什么 | 列表在哪 | 有看护带？ |
|---|---|---|---|
| `deepseek-harness`（Web/桌面，**同产品**） | 会话工作区 | 左栏 `sidebar` | ❌ 无 |
| `deepseek-harness-mobile`（Compose） | 单个会话的对话 | 左滑抽屉 | ❌ 无 |
| `dsh-mobile`（WebView 壳） | 对话主体 | 56px→340px 抽屉 | ❌ 无（纯继承） |
| `qianxun/mobile`（旧 RN） | **连接列表** | 根屏就是列表 | ❌ 无 |

### 1.1 `deepseek-harness`（最权威的同产品参考）

**没有「首页」这一屏。** 只有一个 `root` 槽位，三栏框架：
左 `sidebar` / 中 `main` / 右 `rightbar`
（`packages/client/ui-layout/src/client/index.ts:178-190`）。
中栏默认渲染会话（`AppFrame.tsx:40-43`）。

左栏从上到下（`ui-sidebar/src/client/SidebarRoot.tsx:216-312`）：

1. 品牌行（宽态兼作「新建会话」快捷入口）+ 折叠开关 `:216-256`
2. 「新建会话」按钮行 `:259-277`
3. 全局面板导航行 `sidebar.panellist` `:279-293`
4. **工作区/会话浏览区** `sidebar.workspaces` `:297-302`
5. 底部 footer + 设置 `:305-312`

浏览区内部（`ui-workspace/src/client/rows/WorkspaceBrowser.tsx:1207-1419`）：

- 区段头 36px：标题 + 内联搜索 + 视图选项 + 加工作区 `:1207-1322`
- 列表体：分组树 / 平铺 / 搜索结果三选一 `:1343-1419`

**待处理是行内属性**（`ui-workspace/src/client/rows/Rows.tsx:347-397`）：

- `pendingInteraction` ∈ {`approval`, `plan-review`, `question`}
  （`ui-workspace/src/client/tree.ts:35`、`:379-388`）
- 呈现：**行首状态点 + 右侧紧凑文字标签**
- 文案是三个字：`待审批` / `计划待审` / `待回答`
  （`ui-workspace/src/locales.ts:109-111`）—— 因为 web 端行高只有 **32px**
  （`Rows.module.css:110-113`），三个字是塞得下的极限
- 另有一个「运行中但未查看」的绿色完成点 `completionUnread`
  （`tree.ts:412`、`Rows.tsx:395`）
- 唯一的框架级 badge 槽 `sidebar.toggle.badge` 实际被**桌面更新指示**占用，
  不是 agent 待办（`ui-sidebar/src/client/contract/slots.ts:19`）

**大列表策略：无虚拟滚动、无分页。** 全部行一次性渲染进
`overflow-y: auto`（`WorkspaceBrowser.module.css:358-374`）。
控制手段是**每工作区折叠**：默认只显示 **5 条 idle 会话**，
blank/running/有运行中子代理的行**不占配额**
（`WorkspaceBrowser.tsx:55-70`），其余折叠成「展开 N 条」，每次 +5
（`:581-602`）。

**搜索是双路**：本地对 title/工作区名做子串匹配，再与 Host
`session.search` 合并（`tree.ts:542-585`），防抖 250ms
（`WorkspaceBrowser.tsx:51`、`:1105-1123`），协议结果**上限 20 条**
（`packages/api/session-controller/src/types.ts:197`），查询串上限
500 UTF-16 code units（`WorkspaceBrowser.tsx:53`）。

**置顶规则**：pin 集合**领在每个分节内部最前**，不是独立分节 ——
`sectionMembers()` 返回 `[placeholders, pinned..., rest]`（`tree.ts:273-287`）。

**分组三模式**：`workspace` / `workspace-tree` / `flat`
（`WorkspaceBrowser.tsx:121-124`）；排序两模式 `manual` / `updated`
（`:127-128`）；归档三档筛选 `hide/show/only`（`tree.ts:230`）。

**subagent 完全不进列表**：`session.origin === 'subagent'` 直接
`return false`（`tree.ts:244`），只在父行上给一个「N 个子代理运行中」
的状态（`Rows.tsx:351-361`）。

**blank 会话只显示被选中的那一条，且永远排第一**（`tree.ts:245`、`:217-223`）。

### 1.2 `deepseek-harness-mobile`（Compose）

**`home.png` 实际是连接页（ConnectScreen），不是会话列表。**
已连接/重连中且曾连过 → `MainScreen`（`ui/AppRoot.kt:58-71`）。

`MainScreen` 是 `ModalNavigationDrawer`，drawerContent 是 `ChatListDrawer`
（`ui/screens/main/MainScreen.kt:52-59`）。**根屏 = 单个会话的对话**。

抽屉从上到下（`ChatListDrawer.kt:194-437`）：

1. 标题行「Chats」+ 搜索图标 + 排序 chip + 设置图标 `:194-224`
2. **全宽「New session」按钮** `:226-232`
3. 折叠式搜索框（点图标才出现，源码注释明确说**为了不给手机高度抽屉浪费一行**）`:236-257`
4. `LazyColumn` 列表体 `:261`：
   - 有查询时：「Search」分组头 + 结果行 `:262-288`
   - **「Pinned」独立分组头 + 置顶会话行**，且置顶会话
     **同时仍留在自己工作区分组里**（`:291-308` 注释：*each stays in its own group too*）
   - 每个工作区：可折叠头 → 该区会话 → 其 subagent 按
     `origin == 'subagent'` **就地缩进嵌套** `:309-358`、`:146-151`
   - 「Sessions」分组头 + 未归属任何工作区的会话 `:360-387`
   - 「Archived」DisclosureRow，默认收起 `:389-404`
   - 全空时 `EmptyHero` `:406-413`
5. 底部「新建工作区」行 `:416-437`

**待处理仍是行内属性**：`pendingInteraction != null` → 状态点转
Warning + 右侧 `DsPill(warn=true)` 显示 **「Needs you」**
（`ChatListDrawer.kt:709-715`、`:744-747`；
`res/values/strings.xml:120` = "Needs you"）。
跨会话提醒只走**系统通知**，独立 channel `needs_action`（HIGH 优先级），
事件 `ReviewRequested` / `QuestionRequested`
（`notify/DshNotifications.kt:35`、`:89`；
`notify/NotificationObserver.kt:170-184`）；前台且该会话已打开时抑制
（`NotificationObserver.kt:143-146`）。

**行上的状态编码**（`ChatListDrawer.kt:667-758`）：

- **最左 2dp × 24dp 强调条**：仅当前会话为 accent 色 `:684-690`
- 16dp 缩进（有 subagent 时按 depth×16dp）+ 独立可点 chevron `:667`、`:695-707`
- **8dp 状态点**：`running → Running`（蓝色，带 3×3 像素追逐动画）、
  `pendingInteraction → Warning`（琥珀）、否则 Idle `:709-715`；
  `ui/components/StateDot.kt:29-43`
- 副行：工作区 basename + `·` + 相对时间，caption 11px `:725-742`
- 右侧 `DsPill`：待处理「Needs you」、subagent 数量 `:744-758`
- 当前会话：整行背景 `sidebarNavActive` `:669`

**对话屏内另有常驻 dock 带**：TodoDock / GoalBar / QueueDock，
都渲染为**折叠的单行摘要**、点击展开。源码注释写明理由：
*avoid data arriving pushing transcript and composer around*
（`ui/components/Docks.kt:54-62`；`ChatScreen.kt:450-461`）。

**搜索**：本地匹配 title/工作区名**不防抖**（源码注释：防抖反而让人觉得
搜索没反应，`ChatListDrawer.kt:127-137`）；协议 `session.search`
250ms 防抖 `:122-125`。内容搜索不可用时显示一行说明而不是报错
（`:246-255`；`SessionStore.kt:2079-2086` 明确 `session-query-sqlite`
默认 `openAt: never`）。本地结果上限 30 条（`SessionSearch.kt:28`、`:92`）。

**blank 会话全部隐藏**（harness 会复用它们）`ChatListDrawer.kt:88-91`、`:141`。
**只有含当前会话的工作区默认展开** `:316-320`。
排序**持久化**在 HostsStore 而非 remember `:79-83`、`:114-117`。

### 1.3 `dsh-mobile`（WebView 壳，**此前完全没读过**）

**它不重写会话列表 UI** —— `mobile-layout.ts` 注册的 root 子槽是
`sidebar` / `conversation` / `details` / `shell.overlay`（`:337-345`），
其中 `sidebar` 拿到的就是 DSH **原版** `SidebarRoot`。

`MobileAppFrame` 首屏（`src/mobile-layout.ts:292-320`）：

1. `<main class="dshm-main">` 常驻内容区，渲染 `conversation` 槽 `:293`
   ← **这是屏幕主体，不是列表**
2. 关闭浮层的 scrim 按钮 `:294-303`
3. `<aside class="dshm-drawer">` 抽屉，aria-label「工作区与会话导航」`:304-312`
4. `<aside class="dshm-details">` 详情面板，宽 `min(94vw,460px)`，
   右滑入 `:146-148`、`:313-318`
5. `<div class="dshm-overlay">` 全局浮层 `:319`

**断点（这可能就是「遗漏屏」的来源）**：

- 窄屏：抽屉固定定位，`width:56px`，展开 `min(88vw,340px)` `:140-141`
- **唯一断点 `WIDE_LAYOUT_MIN_WIDTH_PX = 900`** `:45`：≥900px 时抽屉变
  `position:static` 的 grid 第 1 列，展开 340px / 收起 56px，
  并取消遮罩 `:178-184`、`:299`
- 宽视口初始 `sidebarOpen: true`，但控制器是**模块级单例**，
  重连/切回前台不重置用户显式折叠 `:71`、`:325-329`
- 其他断点只做局部微调：`max-width:420px` 上下文字段转单列；
  `max-width:600px` 提问/计划评审卡片改成 48px 触控目标与双列按钮 `:164-177`

**设计规范**（`design-system/dsh-mobile/MASTER.md:9-45`）：
白底 `#FFFFFF` / surface `#F8FAFC` / 主文 `#172554` / accent `#2563EB` /
danger `#DC2626`；卡片圆角 16px、按钮 12px、触控目标 ≥44px（主操作 ≥48px）；
过渡只允许 150–200ms；输入框 16px；测试宽度 375px 与 390px 竖屏 + 横屏 +
字体缩放 + 键盘弹起。

### 1.4 `qianxun/mobile`（旧 RN 壳 —— 对照组）

`HomeScreen`（`src/screens/HomeScreen.tsx:79-201`）根屏 = 连接列表：

1. 导航行：大标题「连接」+ 36×36 圆形「+」`:81-92`
2. 列表或空态 `未添加连接` / `点右上角 + 添加电脑`，`paddingTop: 96` `:99-108`
3. 通知文字 `:187-200`
4. 长按底部 sheet：重命名 / 更新链接 / 删除 / 取消 `:203-347`

**没有搜索、没有分组、没有置顶、没有待处理区。** 排序写死：
`lastUsedAt` 倒序，再 `addedAt` 倒序 `:42-46`。
**大列表用 `ScrollView` 一次性全渲染** `:94` —— 与 393 条直接冲突。
状态：行首 8dp 圆点 on=绿/off=红/未探测=三级灰 `:123-135`；
副行 `host · 中文状态文字` `:76-77`；行高 `minHeight:64` `:403-426`。
状态探测：进前台逐个 `probeInfo`，每台间隔 120ms，20 秒轮询 `:48-74`。

进连接后是 `WorkspaceScreen`，底部 Surface 切换 tab
（`WorkspaceScreen.tsx:25-89`），Surface 注册表**当前只有「工作台」**
（一个 WebView）和**「远程桌面」（占位未实现）**（`src/surfaces.tsx:53-56`）。

---

## 2. 三个项目的共同做法

1. **根屏都不是会话列表页。** 会话列表永远在侧栏/抽屉。
2. **「要用户处理」一律行内属性，不做独立看护带。** 三套 DSH 系实现
   都是「行首 warning 状态点 + 右侧短文字」，跨会话提醒**推到系统通知**。
3. **会话按工作区分组**，未归属的单独成一节（web 端叫「未分组」
   `tree.ts:18`；移动端叫「Sessions」`strings.xml:103`）。
4. **默认折叠 + 手动展开，而不是虚拟滚动。** web 端每工作区 5 条 idle
   配额（`WorkspaceBrowser.tsx:55`）；真正用懒加载的只有 Compose 的
   `LazyColumn`（`ChatListDrawer.kt:261`）。
5. **状态用「行首小圆点 + 右侧短文字」双编码**，语义集一致：
   idle / 进行中 / 等用户 / 完成 / 错误。
6. **搜索是本地元数据 + 可选协议内容搜索的双路**，且都对协议缺失做了降级。

---

## 3. 它们互相矛盾的地方

1. **置顶是「独立分节」还是「分节内领前」** —— **直接冲突**。
   Compose 有独立「Pinned」分节且**置顶会话同时留在原分组**（`:291-308`）；
   web 端没有 Pinned 分节，pin 只让行领在**所在分节**最前（`tree.ts:273-287`）。
2. **归档是「筛选器」还是「底部折叠区」** —— 冲突。
   web 端三档筛选（`tree.ts:353-355`）；移动端底部 DisclosureRow
   （`ChatListDrawer.kt:389-404`），另有一个独立 `ArchivedSessions` 面板带自己的搜索框。
3. **subagent 的位置** —— 冲突。
   web 端**完全排除出行**（`tree.ts:244`），只在父行给「N 个子代理运行中」；
   移动端**就地缩进嵌套成树**（`ChatListDrawer.kt:146-151`）。
4. **搜索上限不同**：web **20** 条（`types.ts:197`）/ 移动 **30** 条（`SessionSearch.kt:28`）。
5. **排序控件形态不同**：web 是三段式 ViewOptionsMenu（分组+排序+归档合一，
   `WorkspaceBrowser.tsx:106-168`）；移动是标题行右侧一个只管排序的 chip
   （`ChatListDrawer.kt:476-509`），**分组与筛选根本不可选**。
6. **待处理文字长度不同**：web 端「待审批/计划待审/待回答」三字
   （适配 32px 行高）；移动端「Needs you」完整 pill。
7. **折叠态处理不同**：web 端收成 56px rail 后**列表体完全不渲染**
   （`WorkspaceBrowser.tsx:1325-1339`）；`dsh-mobile` 收成 56px 时
   **内容仍挂载**，只被 CSS 压扁（`mobile-layout.ts:140-145`）。
8. **完成未读提醒**：web 端有 `completionUnread` 绿点（`tree.ts:412`）；
   移动端状态点只有 Running/Warning/Idle 三态，**无完成态**。
9. **大列表策略**：qianxun 与其余三者相反（`ScrollView` 全量无分页）。

---

## 4. 未知 / 风险

- **未跑过实机。** 五张 Compose 截图都是**英文 locale**，
  中文下文案长度不同，行高/截断结论未验证。
- `home.png` 与当前源码**不完全对应**：截图里有橙色警告条
  （对应 `ConnectScreen.kt:140-153`），但没有源码 `:122-136` 的
  LAN/Relay 分段控件 —— 可能截的是更早版本。
- `dsh-mobile` 在真实 ≥900px 视口下是否一致，只读了 CSS 与测试断言
  （`tests/mobile-layout.test.ts:313-327`），**未做视觉验证**。
- `deepseek-harness` 的设计文档未读，根屏结论全部来自源码与 CSS 常量。

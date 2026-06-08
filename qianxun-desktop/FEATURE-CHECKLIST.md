# 千寻桌面端 · Mock 阶段功能清单 (v1)

> 状态: 提议 v1 · 2026-06-07 · Mavis 起草
>
> **目的**: 在 daemon 真实化之前, 用 mock 数据完整实现 Svelte 5 + Tailwind CSS 应用, 验证所有基础交互, 让 UI/UX 设计先稳定下来. 等此清单 100% 通过 + 6 场景跟 HTML 预览一致, 再进入 daemon 真实化阶段.
>
> **设计基线**:
> - `docs/chat-first-redesign.md` v1 (5 实体 + 3 列布局)
> - `docs/daemon-design.md` v1.0 (25 endpoint + 12 SSE 事件)
> - `docs/30_子项目规划/_shared-contract.md` v2 (跨 Track 契约)
> - `qianxun-desktop/preview/index.html` (6 场景可视化模板)
>
> **本阶段 Out-of-scope** (留到 daemon 真实化阶段):
> - 真实 HTTP / IPC 调用 daemon
> - 真实 SQLite 持久化 (用 localStorage 临时)
> - 真实 LLM 流式 (用 `setTimeout` 模拟)
> - 真实 Plan 后台调度 (用 setTimeout 模拟状态变化)
> - 真实 keyring / API Key
> - 真实 systemd / VPS 集成

---

## §0 文件结构 (实施前先定)

```
qianxun-desktop/
├── src/
│   ├── lib/
│   │   ├── types/
│   │   │   ├── entity.ts          # Project / Session / Plan / SubSession / Message / Experience / Minute
│   │   │   ├── sse.ts             # SSE 12 事件类型
│   │   │   └── ui.ts              # Theme / Col 宽度 / 当前活跃
│   │   ├── mock/
│   │   │   ├── projects.ts        # 3 个项目
│   │   │   ├── sessions.ts        # 6 个 session (跨 3 项目)
│   │   │   ├── plans.ts           # 2 个 plan (1 running + 1 done)
│   │   │   ├── sub_sessions.ts    # 4 个 sub_session
│   │   │   ├── messages.ts        # ~20 条消息
│   │   │   ├── experience.ts      # 3 条项目经验
│   │   │   ├── scheduled.ts       # 1 个定时任务
│   │   │   └── seed.ts            # 一键 seed 所有 mock
│   │   ├── stores/
│   │   │   ├── project.svelte.ts  # Svelte 5 rune store
│   │   │   ├── session.svelte.ts
│   │   │   ├── plan.svelte.ts
│   │   │   ├── sub_session.svelte.ts
│   │   │   ├── chat.svelte.ts     # 消息流 + 流式
│   │   │   ├── ui.svelte.ts       # theme / col 宽度 / 当前活跃
│   │   │   ├── toast.svelte.ts    # 弹窗状态
│   │   │   └── persist.svelte.ts  # localStorage 读写
│   │   ├── components/
│   │   │   ├── layout/
│   │   │   │   ├── ThreeColumnLayout.svelte
│   │   │   │   ├── Divider.svelte          # 拖动分隔线
│   │   │   │   ├── TopBar.svelte
│   │   │   │   └── ThemeToggle.svelte
│   │   │   ├── col1/                       # Col 1 侧栏
│   │   │   │   ├── Sidebar.svelte
│   │   │   │   ├── NewTaskButton.svelte
│   │   │   │   ├── SearchBox.svelte
│   │   │   │   ├── ScheduledTaskSection.svelte
│   │   │   │   ├── TaskHistorySection.svelte
│   │   │   │   ├── ProjectSection.svelte
│   │   │   │   ├── ProjectItem.svelte
│   │   │   │   ├── TaskItem.svelte
│   │   │   │   ├── AgentTeamSection.svelte
│   │   │   │   └── SidebarFooter.svelte
│   │   │   ├── col2/                       # Col 2 Chat
│   │   │   │   ├── ChatView.svelte
│   │   │   │   ├── ChatHeader.svelte
│   │   │   │   ├── MessageList.svelte
│   │   │   │   ├── MessageBubble.svelte
│   │   │   │   ├── PlanBlock.svelte
│   │   │   │   ├── PlanTaskRow.svelte
│   │   │   │   ├── InputArea.svelte
│   │   │   │   ├── ContextChips.svelte
│   │   │   │   ├── TabBar.svelte
│   │   │   │   ├── EmptyChat.svelte
│   │   │   │   └── StreamingCursor.svelte
│   │   │   ├── col3/                       # Col 3 Inspector
│   │   │   │   ├── Inspector.svelte
│   │   │   │   ├── PlanSummary.svelte
│   │   │   │   ├── TaskList.svelte
│   │   │   │   ├── ChangedFiles.svelte
│   │   │   │   ├── NewSessionHints.svelte
│   │   │   │   └── SubSessionContext.svelte
│   │   │   ├── shared/
│   │   │   │   ├── Avatar.svelte
│   │   │   │   ├── Badge.svelte
│   │   │   │   ├── Icon.svelte             # lucide-svelte 包装
│   │   │   │   ├── Modal.svelte
│   │   │   │   ├── StatusDot.svelte
│   │   │   │   └── Toast.svelte
│   │   │   └── modals/
│   │   │       ├── ExperienceSuggestModal.svelte
│   │   │       └── ApprovalModal.svelte
│   │   └── utils/
│   │       ├── format.ts            # 时间 / 数字格式化
│   │       ├── id.ts                 # 临时 id 生成
│   │       └── stream.ts             # mock 流式输出
│   ├── routes/
│   │   ├── +layout.svelte           # 整体壳 (TopBar + 3 列)
│   │   └── +page.svelte             # 主页面 (默认第一个 session)
│   ├── app.html
│   └── app.d.ts
├── static/
├── package.json
├── tailwind.config.js
├── svelte.config.js
└── FEATURE-CHECKLIST.md  ← 本文件
```

---

## §1 数据层 (mock) — 6 项

- [ ] **1.1** `lib/types/entity.ts` — 7 个核心 TypeScript 类型 (`Project` / `Session` / `Plan` / `PlanTaskSpec` / `SubSession` / `Message` / `ProjectExperience` / `SessionMinute`), 跟 `daemon-design.md` v1.0 §2.2 一字不差
- [ ] **1.2** `lib/types/sse.ts` — 12 个 SSE 事件类型 (`MessageStartEvent` / `TextEvent` / `ThinkingEvent` / `ToolCallEvent` / `ToolResultEvent` / `PlanUpdateEvent` / `SubSessionEvent` / `ExperienceSuggestEvent` / `StatusEvent` / `ErrorEvent` / `TurnFinishedEvent` / `MessageStopEvent`), type union + discriminator
- [ ] **1.3** `lib/types/ui.ts` — UI 状态类型 (`Theme = 'dark' | 'light'` / `ColumnWidths` / `ActiveView`)
- [ ] **1.4** `lib/mock/seed.ts` — 一键 seed, 初始化 localStorage; 包含 3 项目 (千寻桌面端 / 千寻 daemon / qianxun-test) + 6 session + 2 plan + 4 sub_session + ~20 message + 3 经验 + 1 定时任务
- [ ] **1.5** `lib/stores/persist.svelte.ts` — `persistStore(key, defaultValue)` 工具, 读写 localStorage, 序列化 plan/session 状态
- [ ] **1.6** `lib/stores/seed.svelte.ts` — `seedAll()` / `resetAll()` actions, 顶部 "重置 mock" 按钮触发

---

## §2 Svelte 5 Rune Stores — 7 项

> 用 Svelte 5 runes (`$state` / `$derived` / `$effect`), 不用旧 store API. 跨组件共享用 `export function createXxxStore()` 模式.

- [ ] **2.1** `lib/stores/project.svelte.ts` — `projects` (array) + `activeProjectId` + `expandProject(id)` / `collapseProject(id)` / `isExpanded(id)` (Set 状态)
- [ ] **2.2** `lib/stores/session.svelte.ts` — `sessions` (按 project_id 索引) + `activeSessionId` + `create({ project_id?, folder? })` / `get(id)` / `messages(id)` / `switchTo(id)`
- [ ] **2.3** `lib/stores/plan.svelte.ts` — `plans` (按 session_id 索引) + `tasks(plan_id)` + `sub_sessions(plan_id)` + `start(session_id, contract)` / `cancel(plan_id)` / `tickPlanStatus()` (后台 setTimeout 模拟, e.g. 1 task done / 30s 后变 2 done)
- [ ] **2.4** `lib/stores/sub_session.svelte.ts` — `sub_sessions` (按 plan_id / parent_session_id 索引) + `openSubSession(id)` (触发 Col 2 tab 切换) + `terminate(id, status)`
- [ ] **2.5** `lib/stores/chat.svelte.ts` — 消息流 + 流式输出 (调用 `streamMock()` 模拟) + `appendMessage(session_id, msg)` + `autoScroll()`
- [ ] **2.6** `lib/stores/ui.svelte.ts` — `theme` (dark/light) + `col1Width` / `col3Width` (持久化) + `expandedProjectIds` (Set) + `expandedHistory` / `expandedAgentTeam`
- [ ] **2.7** `lib/stores/toast.svelte.ts` — `toasts` 数组 + `push(toast)` / `dismiss(id)` (用于 experience_suggest / approval 弹窗)

---

## §3 布局骨架 — 5 项

- [ ] **3.1** `components/layout/ThreeColumnLayout.svelte` — 3 列 flex 布局, Col 1 / Col 2 / Col 3 宽度从 `uiStore` 读
- [ ] **3.2** `components/layout/Divider.svelte` — 4px 拖动条, hover 显示 amber 颜色, mousedown + mousemove 实时改 `uiStore.col1Width` / `col3Width` (范围 0-560px), touchstart 兼容
- [ ] **3.3** `components/layout/TopBar.svelte` — 顶部导航 (项目名 / 当前 session 标题 / 主题切换按钮), sticky
- [ ] **3.4** `components/layout/ThemeToggle.svelte` — dark ↔ light 切换, 改 `document.documentElement.classList`, 写 localStorage
- [ ] **3.5** `tailwind.config.js` — brand 色 (amber 50-900) + plan 状态色 (running sky / done emerald / failed rose / pending zinc) + 字体 (Inter / JetBrains Mono)

---

## §4 Col 1 (项目与会话侧栏) — 11 项

> 跟 `preview/index.html` Scene 1 Col 1 一致, 6 段: 新建任务 + 搜索 / 定时任务 / 任务历史 / 项目 / Agent 团队 / 底部 (Provider/设置/daemon)

- [ ] **4.1** `components/col1/Sidebar.svelte` — 容器, 滚动区分段, 底部 footer 固定
- [ ] **4.2** `components/col1/NewTaskButton.svelte` — 顶部 "+ 新建任务" 小链接, 点击触发 `createEmptySession()` + 切到 Col 2 空白 Chat
- [ ] **4.3** `components/col1/SearchBox.svelte` — 搜索框, 实时过滤 session / project 名称 (前端 mock 搜索, 不调 daemon)
- [ ] **4.4** `components/col1/ScheduledTaskSection.svelte` — "定时任务" 段, 1 个 item "记忆维护" + 蓝点状态
- [ ] **4.5** `components/col1/TaskHistorySection.svelte` — "任务历史" 段, 平铺 6 个 session, active 高亮, Plan 状态点; 含 "更多 (8)" 折叠
- [ ] **4.6** `components/col1/ProjectSection.svelte` — "项目" 段, 标题栏带 + 新建项目 (空操作), 4 个项目列表
- [ ] **4.7** `components/col1/ProjectItem.svelte` — 单个项目, chevron 旋转 + folder icon, 点击展开/折叠, 展开后显示 task 列表
- [ ] **4.8** `components/col1/TaskItem.svelte` — 单个 session item (在项目下), message icon, 状态点, 点击切到该 session
- [ ] **4.9** `components/col1/AgentTeamSection.svelte` — "Agent 团队" 段, 空状态 "无团队, 点 + 新建"
- [ ] **4.10** `components/col1/SidebarFooter.svelte` — 底部 3 项: ⚡ Provider · DeepSeek / ⚙ 设置 / ● daemon 已连接 (跟原版一致, 不用马许/Ultra Plan)
- [ ] **4.11** 项目展开/折叠状态在 `uiStore.expandedProjectIds` (Set) 持久化

---

## §5 Col 2 (Chat 主工作区) — 12 项

- [ ] **5.1** `components/col2/ChatView.svelte` — 容器, 根据 `activeSessionId` 切换显示 (EmptyChat / ChatHeader + MessageList + InputArea / TabBar)
- [ ] **5.2** `components/col2/ChatHeader.svelte` — 顶部栏, 显示 session 标题 / 模型 / 状态 (active / 已完成 X min ago / 新会话 · 还没开始)
- [ ] **5.3** `components/col2/MessageList.svelte` — 消息流, 自动滚动到底部 (用 `$effect` 监听 `messages.length` 变化)
- [ ] **5.4** `components/col2/MessageBubble.svelte` — 单条消息气泡, 支持 user / assistant / system 角色, 头像 (User 用灰色 / Assistant 用 brand 渐变)
- [ ] **5.5** `components/col2/PlanBlock.svelte` — **核心组件**, 完整展开 Plan 块: 头部 (名字 + 状态 + 取消) + 任务列表 + 底部 (改了 N 文件 / 进度 / verifier 待就绪)
- [ ] **5.6** `components/col2/PlanTaskRow.svelte` — 单个 task 行, 状态图标 (done check / running loader / pending) + 名字 + role 标签 + 耗时 + verifier PASS/FAIL + "打开子会话" 链接
- [ ] **5.7** `components/col2/InputArea.svelte` — 底部输入框, ContextChips + textarea + 工具栏 (+ / 始终授权 / 模型 / 发送)
- [ ] **5.8** `components/col2/ContextChips.svelte` — 上下文 chip 行, folder (未选/项目) + model (MiniMax-M3), **不**含 branch chip
- [ ] **5.9** `components/col2/TabBar.svelte` — 子会话 tab bar (Col 2 顶部), 主会话 tab + 子会话 tabs, 每个 tab 有 close 按钮
- [ ] **5.10** `components/col2/EmptyChat.svelte` — 新会话空白态, 中间 "开始一个新任务" 提示 + "第一个消息发出后自动归类到项目或 Chat"
- [ ] **5.11** `components/col2/StreamingCursor.svelte` — 流式输出光标 (跟 preview 一致, 1px brand 颜色, 闪烁)
- [ ] **5.12** `utils/stream.ts` — `streamMock(sessionId, prompt)` 模拟 LLM 流式输出, 用 setTimeout 分块 append 文本 + 工具调用 + Plan 调用 (跟 chat-first §5.2 一致)

---

## §6 Col 3 (Inspector 检查器) — 6 项

- [ ] **6.1** `components/col3/Inspector.svelte` — 容器, 顶部 "Inspector" 标题 + 工具按钮, 滚动内容根据 `activeSessionId` 切换
- [ ] **6.2** `components/col3/PlanSummary.svelte` — Plan 概要段, 启动时间 / 超时 / verifier 配置 / 依赖图 (`t0 → t1 → t2`), **不再**列 task (避免重复)
- [ ] **6.3** `components/col3/TaskList.svelte` — **唯一**详细任务列表, 完整 task 行 (耗时 + role + verifier PASS/FAIL + 打开子会话 inline)
- [ ] **6.4** `components/col3/ChangedFiles.svelte` — 变更文件清单, +/-/~ 标记 + 文件名
- [ ] **6.5** `components/col3/NewSessionHints.svelte` — 新会话模式提示 (如何归类 / 能做什么 / 快捷键 ⌘N ⌘K ⌘/)
- [ ] **6.6** `components/col3/SubSessionContext.svelte` — 子会话模式: 元信息 + 父 Plan 引用 + "跳回主会话" 按钮

---

## §7 跨场景交互 — 12 项

> 跟 `preview/index.html` 6 场景的可见行为一一对应.

- [ ] **7.1** 新建任务入口: 顶部 + 新建任务 OR ⌘N 快捷键 → `createEmptySession()` + 切到 Col 2
- [ ] **7.2** 新会话渲染: Col 2 显示 EmptyChat + 聚焦输入框 (用 `bind:this` + `tick()` next tick `focus()`)
- [ ] **7.3** 输入消息 → 第一个消息发出 → session.title 自动从首条消息截取 (前 30 字)
- [ ] **7.4** 自动归类演示: 输入框 chip 选了 "千寻桌面端" 时, session 创建后落到该项目下; 不选 = 落 "未分类" / "Chat"
- [ ] **7.5** 流式响应 mock: 用户发消息 → `streamMock()` 模拟 assistant 文本流 (每 50ms append 一段) + 模拟工具调用 + 模拟 Plan 调用
- [ ] **7.6** Plan 自动调度: Plan 发起后, 用 setTimeout 模拟 task 状态变化 (1 task done / 5s, 2 task done / 15s, 3 task done / 30s), Plan 块实时更新
- [ ] **7.7** 打开子会话: Col 3 / Plan 块的 "打开子会话" 点击 → Col 2 顶部出现 tab, Col 2 内容切到 sub_session 消息流
- [ ] **7.8** 子会话只读: terminated 的 sub_session, 任何"发消息"操作 disable, tooltip "子会话已终止, 请回主会话追问"
- [ ] **7.9** 主题切换: 顶部按钮 dark ↔ light, 全局 `document.documentElement.classList` + 持久化, Col 1/2/3 全跟随
- [ ] **7.10** 状态点动画: running Plan 状态点用 `animate-pulse-soft` (Tailwind 自定义 keyframe), spinner 用 lucide `loader` + `animate-spin`
- [ ] **7.11** 拖动分隔线: mousedown + mousemove 实时改宽度, mouseup 持久化到 uiStore; 拖到 0 收起对应列
- [ ] **7.12** 错误展示: mock 模式可手动注入错误 (开发者工具按钮), 触发 SSE `error` 事件, Col 2 显示错误提示气泡

---

## §8 弹窗 (Modal) — 2 项

- [ ] **8.1** `components/modals/ExperienceSuggestModal.svelte` — Plan 完成后触发, 显示 "建议沉淀 N 条经验" + items 列表 (checkbox) + [沉淀] / [修改] / [跳过] 按钮
- [ ] **8.2** `components/modals/ApprovalModal.svelte` — 工具调用 R2+ 风险时触发, 显示命令 / 路径 / URL + [批准] / [拒绝] 按钮, 记住选项 checkbox

---

## §9 路由 (mock 阶段) — 1 项

- [ ] **9.1** `routes/+page.svelte` — 单页面应用 (SPA 模式), 不需要 router. 通过 `uiStore.activeView` 切场景:
  - `'main'` → 默认进入第一个 active session 的 Chat
  - `'new'` → EmptyChat (新会话)
  - `'empty'` → 空状态 (无 session)
  - `'sub'` → 子会话视图 (TabBar 激活)

---

## §10 验收测试 — 5 项 (跟 preview 6 场景对照)

> 每项用 Playwright 跑 (Web 端) 或手动 (Tauri 端) 验证.

- [ ] **10.1** 打开应用 → 默认 Scene 1 主场景: Col 1 侧栏完整 (新建任务 / 搜索 / 4 段 / Provider), Col 2 有 running Plan 块 + 流式输出, Col 3 有 Plan 概要 + Tasks + Changed files
- [ ] **10.2** 切 Scene 2 (Plan 完成态): Col 1 当前任务高亮 (有 check 圆点), Col 2 Plan 块是 done 状态 (绿色边框), 底部有交付摘要 + 附件, 弹 "沉淀经验" 气泡
- [ ] **10.3** 切 Scene 3 (新任务中心): Col 1 侧栏窄, Col 2 居中 "千寻, 让干活更简单" + 大输入框 + 3 个 chip (folder/model) + 5 个模板按钮
- [ ] **10.4** 切 Scene 4 (子会话): Col 2 顶部 tab bar (主会话 + 子会话), Col 2 内容是子 Agent 消息流, Col 3 是子会话 context
- [ ] **10.5** 切 Scene 5 (空状态): Col 1 居中 "还没有任务", Col 2 是欢迎卡片, Col 3 是 "千寻能做什么" 功能介绍

---

## §11 Out-of-scope (本阶段不做, 留给 daemon 真实化)

明确**不**在 mock 阶段实现的, 避免 scope 蔓延:

- [ ] **N/A** 真实 HTTP client (SSE 消费 / 25 endpoint 调用)
- [ ] **N/A** 真实 IPC (Tauri invoke / Rust → TS bridge)
- [ ] **N/A** 真实 LLM 流式 (用 DeepSeek API 调 chat)
- [ ] **N/A** 真实 keyring 集成 (API Key 存 macOS Keychain / Linux libsecret)
- [ ] **N/A** 真实 SQLite 持久化 (用 localStorage 临时)
- [ ] **N/A** 真实 Plan 后台调度 (用 daemon Rust 跑 PlanRegistry)
- [ ] **N/A** 真实 SubSession 独立进程 (用 daemon SubSessionHost)
- [ ] **N/A** 真实 verifier 独立 re-derive (用 verifier 角色 sub_session 调 LLM)
- [ ] **N/A** 真实 ProjectExperience 写入 qianxun-memory (用 localStorage mock)
- [ ] **N/A** 真实 SessionMinute 增量 + LLM 摘要生成 (用 mock 文字)
- [ ] **N/A** systemd / Windows Service / macOS launchd 集成
- [ ] **N/A** VPS WS 转发 (VPS 端真实化阶段)
- [ ] **N/A** 移动端响应式 (Stage 8 Flutter 单独做)
- [ ] **N/A** 国际化 (zh-CN / en 切换, 留 v2)

---

## §12 验证节奏 (本阶段)

每完成一个 Phase, 跑一次验证 (按 §10):

| Phase | 完成日期 | 验证场景 | 验证人 |
|---|---|---|---|
| §1 数据层 | __ | `pnpm dev` 不报错, mock seed 跑通 | 自己 |
| §2 Stores | __ | §10.1 主场景渲染 (用 mock) | 自己 |
| §3 布局 | __ | §10.1 主场景布局跟 preview 一致 | maxu |
| §4 Col 1 | __ | 4 段 (新建任务 / 搜索 / 4 段 / 底部) 完整 | maxu |
| §5 Col 2 | __ | §10.1 流式输出 + Plan 实时更新 | maxu |
| §6 Col 3 | __ | §10.1 Col 3 跟 Col 2 Plan 联动 | maxu |
| §7 交互 | __ | §10.1-§10.5 全部 | maxu |
| §8 弹窗 | __ | 经验沉淀 / 工具审批 弹窗可关闭 | maxu |

---

## §13 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| mock 数据不真实, 流式效果假 | 低 | 跟 preview HTML 对照, 加真实感 (状态点动画 / spinner / 渐变背景) |
| Col 1/2/3 比例不合理 (e.g. Col 1 太宽) | 中 | 默认值跟 preview 一致, 用户可拖, 持久化 |
| Svelte 5 runes 学习曲线 | 中 | 限制 runes 用法: 不用旧 store, 不用 $: 标签, 全用 $state/$derived/$effect |
| 6 场景全跑完, 跟 preview 不一致 | 高 | §10 逐场景对照, 任何差异修到 0 |
| mock 阶段跟 daemon 阶段接口不兼容 | 中 | 5 实体 + 12 SSE 事件都从 daemon-design v1.0 镜像, 接口一致 |

---

## §14 后续阶段 (mock 跑通后)

| 阶段 | 触发 | 范围 |
|---|---|---|
| **Phase 11** Mock 完成 | 6 场景验收 | 桌面端 v0.1 发布 |
| **Phase 12** Daemon 真实化 | Mock 稳定后 | daemon Rust 端 25 endpoint + 12 SSE 事件落地, SQLite 持久化 |
| **Phase 13** IPC 桥接 | Daemon 端 OK | Tauri IPC 包装 daemon HTTP, 前端 fetch → IPC → HTTP |
| **Phase 14** 真实数据替换 Mock | 全部跑通 | mock 替换为真实 fetch, 移除 `lib/mock/` |
| **Phase 15** Web Console | 桌面端稳定 | 01b-daemon-web-console.md 实施, 复用 §1-§10 组件 |
| **Phase 16** Flutter 移动端 | 桌面端稳定 | Stage 8 独立项目, 共享 daemon API |

---

**审完请确认**:
- §1-§11 范围合理吗? 有遗漏或多余的功能吗?
- §0 文件结构 OK 吗?
- §10 验收测试覆盖 6 场景吗?
- §11 Out-of-scope 范围对吗? (mock 阶段不该碰的)
- §12 验证节奏合理吗?

通过后, 我开始按 §1 → §11 顺序实施.

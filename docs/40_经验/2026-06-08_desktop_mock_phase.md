# Desktop Mock 阶段 项目日记

**时间**: 2026-06-07 ~ 2026-06-08
**目标**: chat-first 范式从设计 → 文档 → Svelte 5 + Tailwind v4 mock 跑通
**作者**: Mavis (按 maxu 要求, 项目日记风格, 不写成正式 ADR)

---

## 背景

千寻桌面端从 VSCode-IDE/Kanban 范式改成 chat-first 范式, 准备跟即将做的 daemon (Rust) 拼成 "TUI/ACP 改 thin client + daemon 唯一 runtime" 的形态.

先后落地的东西:
1. `docs/chat-first-redesign.md` v1 (35KB) — 用户面/UI/5 实体/6 场景
2. `qianxun-desktop/preview/index.html` (95KB) — Tailwind + Lucide 静态预览
3. `docs/30_决策/ADR-0002_daemon_design_chat_first.md` (11KB) — 决策记录
4. `docs/daemon-design.md` v1.0 (72KB, 19 章) — 后端/Rust/SQLite/SSE 详细设计
5. `docs/30_子项目规划/_shared-contract.md` v2 (25KB) — 跨 Track 契约
6. 4 个旧文档归档 (`docs/90_历史/`)
7. **Mock 阶段实施**: 51 个新文件 (22 TS + 29 Svelte), 6 store, 23 component, 路由
8. **9 个真实 bug 修复** (Svelte 5 + Tailwind v4 各种坑)

---

## 时间线

### 2026-06-07 晚 — 设计落定

千寻 kanban 范式的整体 (看板/分支/已归档/分支集成) 决定删除. 改成:
- **Plan 列表 = Sub-sessions** (1 task = 1 sub_session, 不分两个)
- **SSE 事件用 W3C 标准** (`event:` + `data:` 两行), 不用单 JSON `type` 字段
- **Col 2 Plan 块完整展开**, Col 3 不列 Tasks/Sub-sessions (避免重复)
- **新任务 = 新 Chat** (不弹窗, 不居中页, 跟 Claude Code 一致)
- **简化侧栏**: 新建任务是小链接, 不大品牌按钮

**用户偏好确认**:
- 命名: 简洁亲切 (2字以内中文), 拒绝 "统筹/主理/寻策" 这种
- Mavis 不要自动 git commit, 全部手动
- 规划中不包含进程启动测试
- 测试自动化优先, 进程走 in-test 启动
- 二次确认弹窗不需要
- 不要主动修复别人修改
- 命令一律 python 脚本
- 经验沉淀必须详细单独文档

### 2026-06-07 晚 — Mock 实施

一口气把 mock 全写完了. 51 个文件, 几百个组件, 几百行 store, 然后报错.

### 2026-06-08 凌晨 — 修 bug

跑 `pnpm dev` 报 8 个错, 我一个一个修, 每个错都是 Svelte 5 / Tailwind v4 的细节, 单独看都是一个"啊原来", 加起来是一份完整的"避坑指南".

---

## 9 个 bug + 修复 (按发生顺序)

### Bug 1: `class:` 指令不能用在 component 上

```
src/lib/components/shared/Toast.svelte:20:4
This type of directive is not valid on components
```

**症状**: Toast 里给 `<Icon>` 加 `class:text-sky-500={kind === 'info'}`, 编译挂.

**根因**: Svelte 5 的 `class:` 指令只接受**单一 class 名**, 且**只能用在 HTML 元素**, 不能用在 component. `<Icon>` 是我包装的 component, 不是 `<span>`.

**修复**: 改用 computed class 字符串. 加 helper:
```ts
function iconColorClass(kind: string) {
    if (kind === 'success') return 'text-emerald-500';
    if (kind === 'warn') return 'text-amber-500';
    if (kind === 'error') return 'text-rose-500';
    return 'text-sky-500';
}
```

应用: `<Icon class="... {iconColorClass(t.kind)}" />`.

**同类修了 3 个文件**: Toast, ProjectItem (folder icon + message-square icon), TaskHistorySection (folder icon).

**教训**: 写组件时如果只关心"加 class 给根元素", 不需要 `class:` 指令, 直接 `class={...}` 字符串就够. `class:` 适合单 class 条件切换, 不适合复杂组合.

### Bug 2: `class:` 指令属性名不能含 `/` 或 `[]` (Tailwind 修饰符)

```
src/lib/components/col1/ProjectItem.svelte:58:29 Expected token >
class:hover:bg-zinc-200/50={activeId !== s.id}
                                ^
```

**症状**: `class:hover:bg-zinc-200/50` 报错. caret 指向 `}`.

**根因**: Svelte parser 看到 `class:hover:bg-zinc-200/50` 时, `class:` 后面跟 `hover:bg-zinc-200/50={...}`, 这里的 `/50` 让 parser 误以为 `class:hover:bg-zinc-200` 是属性名, `=50` 是值, 然后看到 `{` 又困惑.

Tailwind 的 opacity modifier `/50` 跟 arbitrary value `[200px]` 都会让 parser 挂. **多个 `:` 是 OK 的** (像 `class:dark:hover:bg-zinc-800`).

**修复**: 同样改 computed 字符串. 把整段 class attribute 合并.

**教训**: 遇到 Tailwind 复杂 utility (modifier + opacity, 或 arbitrary value), 别用 `class:` 指令, 直接 computed 字符串. 简单 single class 才用 `class:`.

### Bug 3: `class:` 在 component 上 (跨 Bug 1/2 误诊)

我一开始看到 `class:text-sky-500={...}` 报"not valid on components" 就归到 Bug 1. 但实际报错位置变了, 在 `<button>` HTML 元素上, 用 `class:hover:bg-zinc-200/50` 也挂 — 跟 component 无关, 是 Bug 2.

**教训**: 错误信息要读完整, 报错位置是关键线索.

### Bug 4: Svelte 5 `const $state(...)` 不能重赋值

```
src/lib/stores/ui.svelte.ts:36:3 Cannot assign to constant
```

**症状**: `const col1Width = $state(260)` 后 `setCol1Width` 里 `col1Width = ...` 报常量赋值.

**根因**: Svelte 5 runes 严格区分 `let` vs `const`:
- `let foo = $state(...)` 允许重赋值
- `const foo = $state(...)` 只允许调方法 (push/splice/add/delete), 不允许 `foo = ...`

但 `const set = new Set()` 调 `set.add()` 是 OK 的, 因为没重赋值 `set`.

**修复**: 3 处 `const` → `let`: `col1Width`, `col3Width`, `activeView`. 其他 (expandedProjectIds, toasts) 保持 `const` 因为只调方法.

**教训**: Svelte 5 runes 的 `const` 跟原生 JS 语义一样, 别想"反正 state 是 reactive 的"就乱用 const.

### Bug 5: `{@const}` 必须是控制流块的直接子

```
src/lib/components/col2/ChatView.svelte:66:5
`{@const}` must be the immediate child of `{#snippet}`, `{#if}`, `{:else if}`,
`{:else}`, `{#each}`, `{:then}`, `{:catch}`, `<svelte:fragment>`, `<svelte:boundary>` or `<Component>`
```

**症状**: `{@const plan = planStore.get(msg.plan_ref)}` 写在 `<div class="ml-10">` 里面挂掉.

**根因**: `{@const}` 必须在 `{#if}` / `{:else if}` / `{:else}` / `{#each}` / `{#snippet}` / `<svelte:fragment>` / `<svelte:boundary>` / `<Component>` 这些**控制流块**的直接子节点, 不能包在普通 HTML 元素 (`<div>`/`<section>`) 里.

**修复**: 提到外层:
```svelte
{:else if msg.plan_ref}
    {@const plan = planStore.get(msg.plan_ref)}  ← 提到这里
    <div class="ml-10 max-w-3xl">
        {#if plan}
            <PlanBlock {plan} />
        {/if}
    </div>
```

**教训**: 之前以为 Svelte 4 的 `{@const}` 在哪都行, 实际 Svelte 5 严格限制位置. 写模板时把 `{@const}` 紧贴控制流块, 别埋在 HTML 里.

### Bug 6: 主题切换无效 (真根因)

```
用户: 主题切换无效, 请检查
```

**症状**: 点 ThemeToggle 没反应. 我之前以为是 mode-watcher 没挂.

**根因 (这次找到真因了)**: Tailwind v4 默认 `darkMode: 'media'`, 不响应 `<html class="dark">` 切换! 我手动 `document.documentElement.classList.toggle('dark', ...)` 一直在 toggle, 但 Tailwind 完全不感知.

Tailwind v4 改成 class 模式需要:
```css
@custom-variant dark (&:where(.dark, .dark *));
```

这一行加到 `layout.css` 就好. 是 shadcn-svelte 标准配置, 我初始化时漏了.

**修复过程 (弯路)**: 我先以为是 mode-watcher 没挂, 改了 `ui.svelte.ts` 走 `setMode`/`toggleMode` + 在 `+layout.svelte` 加 `<ModeWatcher />`. 结果整个 app 点击不响应 (Bug 7). 回滚后才追到 Tailwind 这行.

**教训**:
1. **Tailwind v4 跟 v3 不一样, darkMode 默认 media 不是 class**. 配 shadcn-svelte 必须加 `@custom-variant`.
2. **依赖的依赖**: 项目模板 (`pnpm dlx shadcn-svelte init`) 应该自动加这行, 看一下是不是 init 步骤漏了.

### Bug 7: `$effect.root` + `mode.subscribe` 在 module top-level 把整个 app 炸了

```
用户: 上面的修改之后, 很多点击都没有反应了
```

**症状**: 改成 mode-watcher 路径后, 项目里所有 onclick 都失效.

**根因**: 我在 `ui.svelte.ts` module top-level 写:
```ts
let theme = $state<Theme>('dark');
$effect.root(() => {
    return mode.subscribe((m) => {
        theme = m === 'light' ? 'light' : 'dark';
    });
});
```

`$effect.root` 在 module load 时被调, 但 .svelte.ts module 顶层没有 Svelte runtime context. `$effect.root` 是设计给 component init 用的. Module load 调它, 内部 effect 跟 Svelte 5 reactivity 状态机冲突, 整个 app 陷入死锁 / 状态错乱.

**修复**: 完全回滚, 不依赖 mode-watcher, 回到 manual DOM toggle. 配合 Bug 6 的 `@custom-variant` 修复, 主题切换 work.

**教训**:
1. **`$effect` / `$effect.root` 必须在 component context 里调** (.svelte 文件的 script, 或 onMount 之类). module top-level 不能用 runes 副作用.
2. **依赖 mode-watcher 之前先看它怎么挂**: `<ModeWatcher />` 必须在 root layout, 这是"启动器". 没挂它, `mode` store 是死状态.
3. **不要为了"用标准库"过度工程化**: manual `classList.toggle` 在我们的场景就够, mode-watcher 是给"持久化 + system preference"用的, 我们 mock 阶段不需要.

### Bug 8: 视觉按钮但 onclick 没绑 (Toast 状态色 / Inspector 折叠 / PlanBlock 打开子会话)

3 个文件都有"看起来像按钮"但实际是 `<button>` 包了内容, 没绑 `onclick`:
- `PlanBlock.svelte:115` — "打开子会话" 按钮悬空
- `Inspector.svelte:20` — "收起" 按钮 (panel-right-close 图标) 悬空
- Toast 的 4 个 border/text 颜色 — 其实是 Bug 1 修了, 跟这个无关

**修复**: 加 onclick + 加 state (`col3Collapsed`) + 加 toggle method + 折叠后 TopBar 加 "展开" 按钮.

**教训**: mock 阶段做完后, 应该有个"按钮验证清单": 列出所有 `<button>` 元素, 逐个确认 onclick 是否绑了. 我做了 51 个文件一次性写完, 没做这步, 漏了 3 个.

### Bug 9: 任务历史点击重排 (用户反馈)

```
用户: `任务历史` 中的 项, 点击后, 就排序到了 第一个, 这体验有点差,
      正确的行为是 按照创建的先后顺序 固定位置
```

**症状**: 我按 `last_active_at` 排序, 点击 → 改 last_active_at → 重新 sort → 跳到第一个. 像 IDE 的"最近使用", 但用户认为是 bug.

**根因**: 我把"任务历史"当 MRU 实现了, 用户期望是固定时间线.

**修复**: 改成 `created_at` 倒序, 位置固定.

**教训**: 命名歧义坑. "任务历史"在中文里是 "history" (固定), 不是 "recent". 实现前应该问用户, 不要按英文 "recent" 自由发挥.

---

## 设计决策 (mock 阶段定下来的)

### 子会话打开 UX (B 方案)

候选 4 个:
- A. 跳新路由 `/sub-session/{id}` (URL 可分享, 跟 IDE 一致) — 缺点: 左栏状态丢
- B. 同页面切换, Col 2 复用 ChatView 渲染 sub_session messages — 跟 session 切换一致 ✅
- C. 弹窗 / 抽屉 / 浮层 — 跟用户"不弹窗不居中页"硬规则冲突
- D. PlanBlock 内联展开 — 跟 TabBar 设计冲突, 没法跨 plan 切

选 B. 实施:
- `uiStore.activeView.kind = 'sub_session'` 已经支持
- ChatView 加 `{:else if view.kind === 'sub_session'}` 分支
- 抽 `ChatStream.svelte` 复用渲染 + 输入框
- PlanBlock 按钮绑 `subSessionStore.open(id)`
- TabBar 已经按 sub_session 切 (orphan component, 没用上, 跟当前 UX 不冲突, 留着备未来用)

### SubSession 交互模型 (方案 2: 统一追问)

候选 4 个:
- 1. 严格只读 — Active 可交互, 其他禁用输入框
- 2. **统一追问** — Active 正常, 其他走 "followup" 模式, 消息标记 `kind: 'followup'`
- 3. 状态机对齐 — Failed/Aborted 显"重试", Done 显"追问"
- 4. 追问 = 新 session — 切到新会话, 引用原 sub_session

选 2. 理由:
- 跟 Claude Code / ChatGPT "历史会话永远可追问" 一致
- SubSession 已经有 `messages: Message[]`, 复用 1 个数组比新建 N 个"追问 session" 简单
- 跟 session 完全对称 (session 也不区分主线/追问)
- 实施成本: Message 加 `kind?` 字段 + 输入框 placeholder 切换 + followup 角标

实施:
1. `Message.kind?: 'task' | 'followup'`, 默认 `task`
2. `subSessionStore` 加 `isActive(s)` (判 `status === 'Active'`) + `canSend(s)` (非 `ReadOnly`)
3. `chatStore.sendToSubSession(sub_id, text)` — 非 Active 时标 followup
4. `ChatStream.svelte` 抽组件, 支持 `mode: 'task' | 'followup'` 切换 placeholder + 角标
5. Inspector 状态行下方加 hint ("可追问, 不会重新执行")
6. TopBar sub_session 视图显示 role + status + "追问模式" 角标

行为矩阵:

| sub_session.status | 输入框 | 消息 kind | 角标 |
|---|---|---|---|
| `Active` | 正常, "输入消息开始..." | `task` | 无 |
| `Done` | 正常, "追问 (不执行)..." | `followup` | 灰角标 |
| `Failed` / `Aborted` | 同 Done | `followup` | 灰角标 |
| `ReadOnly` | **禁用** | ❌ | "只读" 角标 |

### UI 微调 (用户反馈)

- "返回主会话" 从 Inspector "父 Plan" 块挪到 **TopBar 右侧** (一眼能找到)
- 主题切换从 TopBar 挪到 **Col 1 左下角 (SidebarFooter 的 daemon 状态行右侧)**
- 任务历史按 `created_at` 固定位置, 不重排
- Col 3 Inspector 加折叠按钮 (panel-right-close, 折叠后 TopBar 加展开按钮)

---

## mock 阶段产物清单

### 文件

```
src/lib/types/{entity,sse,ui}.ts            # 类型
src/lib/mock/{projects,sessions,plans,sub_sessions,messages,experience,scheduled,seed}.ts  # mock 数据
src/lib/stores/{ui,project,session,plan,sub_session,chat,seed,persist}.svelte.ts  # 状态
src/lib/components/layout/{ThreeColumnLayout,Divider,TopBar,ThemeToggle}.svelte
src/lib/components/col1/*                   # 9 个 Sidebar 子组件
src/lib/components/col2/{ChatView,ChatStream,PlanBlock,ContextChips,TabBar}.svelte
src/lib/components/col3/*                   # 6 个 Inspector 子组件
src/lib/components/modals/{ExperienceSuggestModal,ApprovalModal}.svelte
src/lib/components/shared/{Icon,Badge,StatusDot,Avatar,Modal,Toast}.svelte
src/routes/+layout.svelte, +page.svelte     # 路由
docs/{chat-first-redesign,daemon-design}.md
docs/30_决策/ADR-0002_daemon_design_chat_first.md
docs/30_子项目规划/_shared-contract.md
qianxun-desktop/preview/index.html
qianxun-desktop/FEATURE-CHECKLIST.md
```

### 验收清单 (FEATURE-CHECKLIST 60 项)

- 6 场景 (新任务/任务历史/项目展开/Plan 块/子会话/主题切换)
- 9 类数据流 (session / plan / sub_session / message / project / experience / scheduled / theme / col-width)
- 7 状态 (UI / project / session / plan / sub_session / chat / seed)
- 5 交互流 (新任务 / 切会话 / 切子会话 / Plan 自动调度 / 主题切换 / Col 折叠)

跑 `pnpm dev` 验收.

---

## 跟 daemon-design v1.0 的 gap

mock 阶段是 chat-first UI 的"骨架", 跟 daemon 真接的差距:

### mock 用 setTimeout 模拟的
- Plan 调度: 启动 5s 后第 1 tick, 之后每 15s 推进 1 task, 终态设 Done + 写 result
- 流式响应: `streamMock()` 用 setTimeout 拼字符串
- 5s 后弹"已连 daemon" toast, 8s 后弹"沉淀经验" modal (3 条 checkbox)
- 任务历史 created_at 写死, session.last_active_at 写死
- SubSession 的 output (files_added, lines_added) 写死

### 真接 daemon 需要做的
- `streamMock` → 真 SSE 客户端, 解析 daemon 发的 12 事件
- Plan 调度 → daemon 的 `dispatcher.run(plan_id)`, 真 LLM call
- SubSession output → 真 `edit_file` / `read_file` 工具执行结果
- 5s/8s 那些 mock 触发器 → daemon 的 session_event / plan_update
- 输入框 send → 调 daemon `POST /v1/sessions/{id}/messages`, 收 SSE 流

### IPC 设计 (Phase 4a)

TUI/ACP 改 thin client, 都通过 HTTP/SSE 接 daemon. desktop 端:
- `lib/api/client.ts` — fetch 包装, base URL 跟 daemon config 同步
- `lib/api/sse.ts` — EventSource 包装, 解析 W3C SSE 格式
- `lib/api/mapping.ts` — daemon event → 内部 store action 的映射
- `lib/stores/chat.svelte.ts` 的 `send()` 改成 fetch daemon 而不是 streamMock
- `lib/stores/plan.svelte.ts` 的 `scheduleAutoComplete` 删掉, 改用 daemon plan_update 事件
- `lib/stores/sub_session.svelte.ts` 的 messages 改成从 daemon fetch, 新消息走 SSE append

---

## 跟用户风格的对齐 (记录, 后续别忘)

- **命名**: 2字以内中文, 英文术语, 中文概念. 拒绝 "统筹/主理" 类.
- **Mavis 不要自动 git commit** — 全部手动
- **Mavis 规划不包含进程启动测试** — 启动 daemon / 跑 team plan / 真连网络, 都用户跑
- **测试自动化优先** — 进程走 in-test 启动 (带 timeout)
- **二次确认弹窗不需要** — 删除/安装代价小
- **不要主动修复别人修改** — 主动修会污染 sibling agent 的 in-flight commit
- **不擅自改用户配置 / daemon / admin.cred**
- **命令一律 python 脚本或 py 命令运行** — 跨平台 shell 一致
- **验证模式先问用户** — 不要自己猜
- **经验沉淀必须详细单独文档** — 不能只 append memory

---

## 接下来的路

### 1. 跑通 mock 阶段 (今天)
- 6 场景 + FEATURE-CHECKLIST 60 项验收
- 清旧文件 (layout/Sidebar, layout/ChatView, layout/SessionList, chat/, team/, ui/ 旧目录)

### 2. Phase 4a — Daemon 升级为唯一 Agent runtime

2 步走, 不一次大改:
- **4a-1 (明天)**: 建 IPC client + 1 个 session 跑通真 LLM (其余仍 mock)
  - `lib/api/{client,sse,mapping}.ts` 新建
  - `chatStore.send` 改成调 daemon (留个 env flag 切回 mock)
  - 真跑一个 chat 看 daemon 流式响应
- **4a-2 (后天)**: Plan / SubSession / 持久化全切真 daemon
  - `planStore.scheduleAutoComplete` 删
  - `subSessionStore.messagesOf` 改成 daemon fetch + SSE append
  - session/project/experience/scheduled 全切

### 3. Phase 4b — VPS Server
WebSocket Hub + 完整认证 + 完整 RAG. 跟 desktop 走同一套 daemon API, 不用重写.

---

写完这份日记, 大概 2 小时. maxu 让我用"项目日记"风格, 不写成正式 ADR, 我尽量按这个来. 下次写经验的时候, 关键是把"为什么这么决策" + "具体什么坑 + 怎么修" + "教训" 写清楚, 比单纯记录做了什么更有用.

---

## 后续: 2026-06-08 凌晨 — Phase 4a-1 完成

### 范围收窄 (重要)

**之前计划**: 4a-1 = "建 IPC client + 1 session 跑通真 LLM (其余 mock)", 4a-2 = "Plan / SubSession / 持久化全切".

**实际改完 4a-1**:
- ✅ IPC client 骨架 (`src/lib/api/{client,chat,types}.ts`)
- ✅ Mock server (`src/lib/api/mock-server.ts`) — in-process, 测用
- ✅ 11 个端到端测试 (`src/lib/api/__tests__/client.test.ts`) — 全 pass
- ✅ vitest setup 修好 ($app/environment alias, env 改用 import.meta.env)
- ❌ **chatStore 改用真 client — 没做**

**为啥不一次做完**: 跟用户讨论时发现 `qianxun/src/daemon/ui/` 是个完整内嵌 UI (我之前完全没看到这个, 51 个 mock 文件是重复造轮子). 跟用户对齐方向后, 选了 A 方案 (退役 daemon/ui, desktop 当唯一前端). 但要彻底弄清楚:
- 路径 v0.2 (`/v1/chat/session/*`) vs v1.0 (`/v1/sessions/*`) — daemon 真用 v0.2, 设计文档是 v1.0
- SseEvent schema — desktop 高层 (12 事件 `event` 字段), daemon 低层 (Anthropic 风格 12 事件 `type` 字段)
- SSE 协议 — desktop parser 读 JSON, 真 daemon (axum) 发 W3C `event:` 行

这些 4a-2 才能彻底定. 4a-1 只到 IPC client + mock 端到端, 算"骨架 ready", 真切要等 4a-2.

### 跑通指南
`docs/40_经验/2026-06-08_phase_4a-1_runbook.md` — 跑测试, 启 desktop 接真 daemon, 4a-2 待办清单.

### 关键发现: daemon/ui 已经成熟, 之前完全没看到
- 14 个 api module (auth, chat, sessions, llm, mcp, memory, skills, settings, tools, system ...)
- 完整组件库 (auth, chat, common, layout, ui)
- 完整测试 (api.test.ts, integration.test.ts, stage-9c-components.test.ts)
- 走 v0.2 路径, 用 v0.2 SseEvent schema
- 跟桌面端 mock 阶段是**独立实现**但**功能重叠**

**教训**: 实施前先做"代码考古", 不要假设项目结构. 我 51 个 mock 文件做完才发现 daemon/ui 早就成熟, 应该:
- 实施前先 grep 整个 workspace 类似功能
- 跟用户对齐"项目当前已有什么"再动手
- 不要"按文档想象的项目结构"动手

### 4a-2 待办 (下次干)
1. chatStore.send 改调 fetchPromptStream, 留 env flag 切真/假
2. SseEvent schema 跟真 daemon 对齐 (写 mapping 层)
3. 路径 v0.2 → v1.0 迁移 (改 desktop + 改 daemon router)
4. parser 升级: 读 W3C `event:` 行
5. planStore.scheduleAutoComplete 删, 接 daemon plan_update 事件
6. subSessionStore / session / project / experience / scheduled 全切
7. 退役 daemon/ui (`rm -rf qianxun/src/daemon/ui`, 改 build 配置)
8. 更新 _shared-contract.md 跟真实现一致

### 几个学到的坑 (本次)
- **SvelteKit 模块在 vitest 解析不了**: 用 alias 指向 mock 文件, 比 vi.mock in setup 稳
- **`import.meta.env` 模块加载时被 snapshot**: vi.stubEnv 后续改不动, 用函数每次读
- **fetch 在 jsdom 需要完整 URL**: 相对路径不 work, 必须拼 base URL
- **SSE 协议 4 种实现** (W3C, JSON `event`, JSON `type`, raw text) 各有差异, 实施前确认 daemon 真发什么
- **mock server 应该严格判 method + path + sessionId**: 默认全走 SSE 会让 4xx 测试失效

---

## 后续: 2026-06-08 07:55 — 架构重新定型 (4 轮讨论)

### 4 轮讨论的关键决策

| 轮次 | 主题 | 决策 |
|---|---|---|
| 1 | 发现 daemon/ui 已存在, 4a-1 路径调整 | 改 A 方向 (退役 daemon/ui, desktop 当唯一前端) |
| 2 | 合并 daemon 到 desktop | 1 个 binary 跑 webview + engine, 共享 state, IPC 链消除 |
| 3 | ACP over WebSocket RFD | 跟 RFD 对齐, 3 transport (stdio + WS + HTTP) 共存 |
| 4 | **最简化原则** (maxu 提) | **2 mode 互斥, 砍 WS 多余层, ACP 留规划** |

### 最终架构 (ADR-0003)

`qianxun-desktop` binary 启 2 种模式, **互斥**:

- **桌面模式 (默认)**: Tauri webview + invoke + engine + **WebSocket server** (同进程, 给未来 client)
- **ACP stdio 模式 (`--acp`)**: 不启 webview, 跑 stdio JSON-RPC, 跟 Zed 通信 (跟 OpenCode 同模式)

`qianxun` binary 保留 tui / acp / daemon / server 多模式, 给 VPS / 远程 / 轻量 CLI 用.

### 砍掉之前 4a-2 8 项 (跟 ADR-0003 决策冲突)

- ~~SseEvent schema 对齐 (daemon v0.2 → desktop 高层 mapping)~~ — desktop 跟 engine 直接交互, 不用走 wire
- ~~路径 v0.2 → v1.0 迁移~~ — desktop 跟 qianxun-core 直接对接, 路径在 Rust 内部
- ~~parser 升级读 W3C `event:` 行~~ — Tauri invoke 不走 SSE wire
- ~~planStore.scheduleAutoComplete 删~~ — Tauri invoke 调 daemon plan_update (4a-2 简化)
- ~~subSession / session / project / experience / scheduled 全切真 daemon~~ — 4a-2 改走 Tauri invoke
- ~~退役 daemon/ui (`rm -rf qianxun/src/daemon/ui`)~~ — 保留为 ADR-0003 决策点
- ~~更新 _shared-contract.md 跟真实现一致~~ — 4a-2 改文档

### 经验教训 (4 轮讨论总结)

1. **实施前做"代码考古"**: 51 个 mock 文件做完才发现 daemon/ui 早就成熟. 教训: 任何大改动前先 grep 类似功能, 跟用户对齐"项目当前已有什么"
2. **"最优化代码"原则**: maxu 反复强调"不要为 webview 启 webview, 不要为 WS 启 WS". 我之前加 WS transport 是"为未来准备" 心理. 教训: **YAGNI** (You Aren't Gonna Need It), 先实现当前需求
3. **ACP RFD 看成熟度看 revision history**: 2025-03 草稿 → 2026-04 重大架构改 → 2026-05 split stream → 2026-06 v1/v2 可靠性. 接近 spec 冻结但还在动. 教训: RFD 不是 final spec, 实施前看最新 revision date
4. **transport 数量跟业务复杂度正相关**: 4 transport (stdio/WS/HTTP/Tauri invoke) 听起来灵活, 实际给业务 / 测试 / 文档都加负担. 2 mode 互斥 (Tauri invoke + stdio) 实际够用. 教训: **简单 > 灵活**

### 当前 todo 状态

- ✅ 4a-1: IPC client + mock server + 11 测试 + 跑通指南
- ✅ ADR-0003: 写完, 反映 2-mode 互斥 + ACP 留规划
- 🔜 4a-2: 排期未定, 等 maxu 确认. 当前暂列 9 项 (跟 ADR-0003 一致)
- 🔜 ACP stdio 模式 spike: 留规划, 未来某天干


# 千寻 Tauri 桌面端 (Stage 1 脚手架)

> 千寻 (Qianxun) 三大前端形态之一: Tauri 桌面 (Track C).
> 设计详见 `docs/30_子项目规划/03-tauri-desktop.md`.

## 当前状态: **Stage 1 — 前端脚手架 (不接 Tauri 2.0)**

本 Stage 只完成 SvelteKit + Svelte 5 + Tailwind v4 + shadcn-svelte 的脚手架,
可在浏览器中独立运行, 验证三栏布局与设计系统. Stage 2 才接入 Tauri 2.0
与本地 Daemon (`http://127.0.0.1:23900`).

## 技术栈 (与 03-tauri-desktop.md §2.3 决策一致)

| 维度 | 选择 | 实际版本 |
|---|---|---|
| 前端框架 | Svelte 5 (runes) | 5.56.0 |
| 构建工具 | SvelteKit + Vite | 2.61.1 / 8.0.16 |
| 样式 | Tailwind CSS | 4.3.0 (CSS-first) |
| 组件库 | shadcn-svelte | 1.3.0 (button + card) |
| 图标 | @lucide/svelte | 1.17.0 |
| 类型 | TypeScript strict | 6.0.3 |
| 包管理 | pnpm | 10.11.0 |

## 命令

```sh
pnpm install      # 安装依赖
pnpm dev          # 启动 dev server (默认 http://127.0.0.1:5173/)
pnpm build        # 生产构建
pnpm check        # svelte-check 类型检查
```

## 项目结构

```
qianxun-desktop/
├── components.json              # shadcn-svelte 配置 (Stage 1: button + card)
├── package.json
├── svelte.config.js             # kit.alias 配置 ($components / $utils / $ui / $hooks)
├── tsconfig.json
├── vite.config.ts               # @tailwindcss/vite + @sveltejs/kit/vite
├── src/
│   ├── app.html
│   ├── app.d.ts
│   ├── lib/
│   │   ├── utils.ts             # cn() + WithElementRef<T>
│   │   ├── types/
│   │   │   └── ipc.ts           # HealthStatus / Project / Session / Team / ...
│   │   ├── stores/
│   │   │   └── connection.svelte.ts  # 4 态连接状态机
│   │   └── components/
│   │       ├── ui/
│   │       │   ├── button/      # shadcn-svelte Button
│   │       │   └── card/        # shadcn-svelte Card + sub-components
│   │       └── layout/
│   │           ├── ThreeColumnLayout.svelte
│   │           ├── Sidebar.svelte
│   │           ├── SessionList.svelte
│   │           └── ChatView.svelte
│   └── routes/
│       ├── layout.css           # Tailwind v4 + shadcn 主题变量
│       ├── +layout.svelte
│       └── +page.svelte         # 三栏 + mock 数据
└── static/
```

## Stage 2 计划 (TODO)

- [ ] 接入 Tauri 2.0 (`@tauri-apps/api`, Rust 端 `src-tauri/`)
- [ ] 真实 IPC 桥接: `invoke('daemon_health')` / `invoke('daemon_list_sessions')` / ...
- [ ] SSE 客户端: `src/lib/sse/client.ts` (POST /v1/chat/session/:id/prompt)
- [ ] MessageBubble / ToolCallCard / ThinkingBlock / CodeBlock
- [ ] settings.svelte.ts + persisted<T> + keyring
- [ ] teams.svelte.ts + Team 管理 UI
- [ ] svelte-i18n (zh-CN + en)
- [ ] ThemeSwitcher (light / dark / system, mode-watcher 已就绪)
- [ ] ConnectionBanner (§10.1 降级 UI)
- [ ] 离线消息队列 (§10.3)
- [ ] SQLite 缓存 (`tauri-plugin-sql`)
- [ ] VPS Server 接入 (WebSocket Hub)

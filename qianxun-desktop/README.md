# 千寻 Tauri 桌面端

> 千寻 (Qianxun) 三大前端形态之一: Tauri 桌面 (Track C).
> 设计详见 `docs/30_子项目规划/04b-tauri-runtime-integration.md`.
> 状态: `docs/10_事实源/desktop-state.md`

## 当前状态: **4a-1 收尾** (2026-06-08)

Tauri 2.x + Svelte 5 桌面端,通过 Tauri IPC invoke 调 `qianxun-runtime` API (in-process, 零网络)。
具体状态见 [desktop-state.md](../docs/10_事实源/desktop-state.md),包括 10 个 Tauri command、11 个 Svelte store、端到端链路、P0/P1 缺口。

## 技术栈

| 维度 | 选择 |
|---|---|
| 前端框架 | Svelte 5 (runes) + SvelteKit |
| 构建工具 | Vite |
| 桌面 runtime | Tauri 2.x (固定 patch version) |
| IPC | Tauri invoke (in-process, 类型安全) |
| 后端调用 | qianxun-runtime RuntimeApi (path dep) |
| 包管理 | pnpm |

## 命令

```sh
pnpm install                                    # 安装依赖
DEEPSEEK_API_KEY=sk-xxx pnpm tauri dev          # 启桌面端 (需 API key)
pnpm tauri build                                # 生产构建 (7 平台)
cd src-tauri && cargo test                      # Rust 后端测试
```

## 端到端链路

```
Svelte 5 ChatView button
  → chat.svelte.ts:send() invoke "send_message"
    → Tauri command (commands/runtime/send.rs)
      → RuntimeApi::send_message → send_message_impl
        → qianxun-core AgentLoop
          → LLM (DeepSeek / minimax)
        ← mpsc::Receiver<SseEvent>
      ← spawn_event_emitter → app.emit("session_event")
    ← onSessionEvent → chat-stream.ts 12-event 状态机
  ← Svelte 反应式重渲染
```

详见 [desktop-state.md](../docs/10_事实源/desktop-state.md) "端到端链路" 段。

## 关联文档

- **状态**: [`docs/10_事实源/desktop-state.md`](../docs/10_事实源/desktop-state.md)
- **集成规划**: [`docs/30_子项目规划/04b-tauri-runtime-integration.md`](../docs/30_子项目规划/04b-tauri-runtime-integration.md)
- **契约**: [`docs/30_子项目规划/_shared-contract.md`](../docs/30_子项目规划/_shared-contract.md)
- **当前决策**: [`docs/30_决策/ADR-0003_desktop_2mode.md`](../docs/30_决策/ADR-0003_desktop_2mode.md)
- **实施经验**: [`docs/40_经验/2026-06-08_04b_subtask_{2,3,4}_*.md`](../docs/40_经验/)

## 当前 P0 缺口

1. 用户手动 E2E 验收 (6 步清单,见 `desktop-state.md` "已知缺口")
2. `sub_session.sendToSubSession` 后端实现
3. `list_plans` Tauri command 注册
4. `project.svelte.ts:loadAll` 后端实现

详见 `desktop-state.md` "已知缺口" 段。

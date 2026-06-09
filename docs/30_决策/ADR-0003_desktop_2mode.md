# ADR-0003: 合并 Desktop + ACP 同进程 (2-Mode 互斥)

- **Status**: Accepted
- **Date**: 2026-06-08
- **Last Revised**: 2026-06-09
- **Authors**: Mavis (按 maxu 决策写)
- **Supersedes**: ADR-0002 (daemon design chat-first) 整篇 (4a 路线), 2026-06-09 ADR-0002 文件已删除

---

## Context

千寻 desktop 当前架构 (mock 阶段) 是:

```
qianxun-desktop (Tauri webview)
       ↓ Svelte UI
       ↓ (mock 数据, setTimeout 模拟流式)
   qianxun-core engine (本机不跑, 只在 mock 模拟)
```

旧设计: `qianxun` binary (多模式: tui / acp / daemon / server) 中 daemon 是 HTTP+SSE 跑在独立进程, 桌面端通过 HTTP 跟 daemon 通信. 部署需要 2 个进程, IPC 链长.

**演进过程中发现**:

1. `qianxun/src/daemon/ui/` 已经有完整 SvelteKit 内嵌 UI (跟 desktop mock 阶段重叠, 旧决策漏判)
2. `qianxun/src/acp/` 已经有完整 ACP 协议实现 (10 module: forwarding_tools, handler, lib, mod, output, prompt, server, session, transport, types)
3. ACP 协议本身 (RFD streamable-http-websocket-transport, 2026-06-05 最新版) 同时支持 stdio + WebSocket + HTTP, 但 Zed 当前**只生产支持 stdio**
4. desktop 用户核心场景 = **单 PC 个人 AI 助理 + Zed 集成**

需要决定: desktop 跟 daemon / engine / ACP server 的关系.

## Decision

**`qianxun-desktop` binary 同时承担 2 种启动模式, 互斥**:

### 模式 A: 桌面模式 (默认, 无 args)

- 启 Tauri webview (Svelte UI)
- Svelte webview 调 qianxun-core engine 通过 **Tauri invoke** (in-process, 类型安全)
- 同进程启 **WebSocket server** (`ws://127.0.0.1:23901/acp` 或类似), 暴露给未来 client (CLI / 浏览器扩展 / 移动端)
- WebSocket server 也可作为 webview 跟 engine 通信的备选通道 (但默认走 Tauri invoke)

### 模式 B: ACP stdio 模式 (`--acp` flag)

- **不启 Tauri webview** (no GUI)
- **不启 WebSocket server** (stdio 走 stdin/stdout, 不需要 WS)
- 跑 qianxun-core engine + ACP stdio JSON-RPC server, stdin/stdout 跟 Zed 通信
- 跟 OpenCode `opencode acp` 同模式
- 用途: Zed 启本地子进程

**互斥**: 一个 binary 一次只跑一种模式, 通过 `argv` 分发.

**共享**: 两种模式共享 qianxun-core engine + AcpHandler trait (业务逻辑 0 重复), 共享 SQLite 数据目录.

### 保留 `qianxun` binary 多模式

- `qianxun tui` (CLI REPL, 走 stdin/stdout)
- `qianxun acp` (ACP stdio, 跟 desktop 模式 B 业务等价, 入口不同 — 留 `qianxun` binary 因为更轻量, 不依赖 Tauri)
- `qianxun daemon` (HTTP+SSE, 给 VPS / 远程 client 用)
- `qianxun server` (VPS server, JWT 认证)

桌面端跑 daemon / server **不**走 desktop binary, 走 `qianxun` binary. 两个 binary 各管各的部署场景.

## Consequences

### 收益

1. **部署简单**: 单 PC 用户 1 个 binary, 不需要启 daemon
2. **IPC 链消除**: 桌面模式走 Tauri invoke (in-process), 0 网络 0 序列化
3. **ACP 集成天然**: Zed 启 `qianxun-desktop --acp` 子进程, 跟 OpenCode 同模式
4. **数据共享**: 桌面模式 + ACP 模式共享 SQLite 同一份数据
5. **未来扩展**: WebSocket server 暴露给任意 client (CLI / 浏览器 / 移动端), 不限于 webview

### 损失 / 权衡

1. **WebSocket server 复杂度**: 桌面模式多一个 server (axum + tokio-tungstenite) 在同进程, 略增加 Tauri 启动负担 (可接受, 跟 daemon 模式比轻多了)
2. **Tauri command 跟 AcpHandler 双注册**: 同一业务逻辑可能要在 Tauri command + AcpHandler 各注册一次, 用 trait 抽象避免重复
3. **当前 Zed 不支持 WebSocket ACP**: WebSocket server 当前没 Zed client 用, 是"为未来准备"
4. **不能跑服务器**: Tauri 不能在 headless Linux 跑, VPS 必须用 `qianxun daemon` / `qianxun server`

## Architecture

```
┌─────────────────────────────────────────────────────┐
│ qianxun-desktop (1 个 binary, Tauri 2.x)            │
│                                                     │
│  Mode A: 桌面 (默认)                                 │
│  ┌─────────────────────────────────────────────┐   │
│  │ Tauri webview (Svelte UI)                    │   │
│  │       ↕ Tauri invoke (in-process)            │   │
│  │ qianxun-core engine + AcpHandler             │   │
│  │       ↕                                      │   │
│  │ WebSocket server (axum + tungstenite)        │   │
│  │  ws://127.0.0.1:23901/acp                    │   │
│  │  (暴露给未来 client)                          │   │
│  └─────────────────────────────────────────────┘   │
│                                                     │
│  Mode B: ACP stdio (--acp)                          │
│  ┌─────────────────────────────────────────────┐   │
│  │ stdio JSON-RPC server                        │   │
│  │  stdin/stdout 跟 Zed                         │   │
│  │       ↕                                      │   │
│  │ qianxun-core engine + AcpHandler (同代码)   │   │
│  └─────────────────────────────────────────────┘   │
│                                                     │
│  ┌─────────────────────────────────────────────┐   │
│  │ SQLite (data/qianxun.db)                     │   │
│  │ 两种模式共享                                   │   │
│  └─────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

## Implementation

### Phase 0 (当前 — mock 阶段)

- mock 阶段代码 (src/lib/api/*, src/lib/stores/*.svelte.ts) 保持, 跑 setTimeout 模拟
- 4a-1 写的 HTTP client 跟 mock server 是契约验证, 保留

### Phase 4a-1 (✅ 完成)

- IPC client 骨架 (HTTP client + mock server)
- 11 个端到端单元测试
- vitest setup 修好

### Phase 4a-2 (后续, 暂不排期)

具体待办待跟 maxu 确认, 暂列:

1. **Tauri main 模式分发**: `qianxun-desktop` 顶层 args 解析, `--acp` 走 stdio, 否则走 webview
2. **Tauri command 注册**: 把 AcpHandler 抽象成 trait, 同一 trait 既能注册成 Tauri command 也能注册成 ACP JSON-RPC handler
3. **WebSocket server 集成**: 桌面模式 setup 阶段 spawn axum server, 同进程
4. **chatStore / planStore / 持久化** 切 Tauri invoke
5. **退役 daemon/ui**: `rm -rf qianxun/src/daemon/ui`, 改 build 配置
6. **`--acp` 模式 spike**: 验证 desktop binary 启 stdio 不启 webview 完整 work (本期暂不做, 留规划)
7. **保留 `qianxun` binary** 多模式 (tui / acp / daemon / server)
8. **更新 `_shared-contract.md`**: 反映 2-mode 互斥架构
9. **ACP 集成 (未来 4a-3+)**: 桌面模式 B 完整实现 + Zed 集成测试

### Phase 4b (VPS / 远程)

`qianxun daemon` + `qianxun server` 完整实现 (跟 desktop 1.0 阶段并行), 走 HTTP+SSE, 给 VPS 跟未来 web 端用.

## 决策点 / 风险

| 风险 | 缓解 |
|---|---|
| Tauri 跟 WebSocket 同进程启动冲突 | 抽象启动顺序: Tauri `setup()` 钩子里 spawn WS server task, 跟 webview 解耦 |
| Tauri 升级影响整个 stack | 业务逻辑在 qianxun-core, Tauri 只负责 webview 跟 invoke, 升级只换 transport |
| stdio 在 Windows GUI app 默认没 console | `lib.rs` 顶部 `#![cfg_attr(not(debug_assertions), windows_subsystem = "console")]` (release 也保留 console) |
| WebSocket server 端口冲突 | 默认 `127.0.0.1:23901` + env `PUBLIC_QIANXUN_WS_PORT` 可覆盖 |
| SQLite 跨模式并发 | SQLite WAL 模式 + 文件锁, 桌面 + ACP 模式同时启不冲突 (互斥锁保证) |

## References

- ADR-0002: daemon design chat-first (2026-06-09 整篇删除,本 ADR 完整取代)
- `_shared-contract.md` v3 (2026-06-09 重写): 跨 Track 契约,RuntimeApi 6 方法 + SseEvent 12 变体
- `docs/10_事实源/runtime-state.md`: qianxun-runtime 子系统状态
- `docs/10_事实源/desktop-state.md`: qianxun-desktop 子系统状态
- `docs/40_经验/2026-06-08_phase_4a-1_runbook.md`: 4a-1 跑通指南
- `docs/30_子项目规划/04b-tauri-runtime-integration.md`: tauri + runtime 集成规划
- `docs/30_子项目规划/04c-qianxun-runtime-extraction.md`: qianxun-runtime 抽取设计
- ACP RFD: `https://agentclientprotocol.com/rfds/streamable-http-websocket-transport` (2026-06-05 最新)
- `qianxun/src/acp/`: 现有 ACP 协议实现 (10 module, 复用)

## Status / Open Questions

- **Open**: 4a-2 排期未定, 等 maxu 确认
- **Open**: WebSocket server 在桌面模式是默认启还是 opt-in (env flag 控?)
- **Open**: 桌面模式跟 ACP 模式能否同时启 (e.g. 一个进程内多 thread) — 当前决策是互斥, 多 thread 模式留未来
- **Open**: Zed 何时支持 WebSocket ACP? 决定后 WebSocket server 在 desktop 才有用

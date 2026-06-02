# 千寻运行说明 (Running Guide)

> 最后更新: 2026-06-02 | 状态: Stage 1-4 全部就绪, 可实际启动运行

本文档说明如何在本地开发环境编译、运行、测试千寻 (Qianxun) 的三个子项目:
- **Daemon** — 本地 HTTP 服务, 唯一 Agent runtime, 跑在 `127.0.0.1:23900`
- **VPS Server** — 远端控制面 (WebSocket Hub + 用户/Team/设备管理), 跑在 `127.0.0.1:23901`
- **Tauri Desktop** — 桌面/移动/Web 三端统一前端, 走 SvelteKit + Tauri 2.0

---

## 0. 前置条件

| 工具 | 最低版本 | 用途 |
|---|---|---|
| **Rust** | 1.85+ (rust-toolchain.toml 锁 stable) | Daemon + VPS Server + Tauri Rust 端编译 |
| **Node.js** | 18+ (推荐 22) | Tauri 前端 SvelteKit dev/build |
| **pnpm** | 9+ | Tauri 前端包管理 (lockfile 是 `pnpm-lock.yaml`) |
| **DEEPSEEK_API_KEY 或 ANTHROPIC_AUTH_TOKEN** | - | 默认 provider 的 API key (任一 env var 即可) |

设置 API key:
```powershell
$env:DEEPSEEK_API_KEY = "sk-xxx"        # 走 deepseek
# 或
$env:ANTHROPIC_AUTH_TOKEN = "eyJ..."     # 走 minimax (推荐)
```

切换 provider 在 `~/.qianxun/config.json` 里改 `"active_provider": "MiniMax"` 或 `"deepseek"`, 不需要改代码.

---

## 1. 编译 (一次性)

```powershell
cd E:\git\maxu\qianxun

# 编译所有 workspace crate (核心 + 记忆 + CLI/daemon/server 入口)
cargo build --workspace --release

# Tauri 桌面端单独编译 (第一次跑会下载 ~1.5GB 的 tauri 2.x 依赖链, 5-10 分钟)
cd qianxun-desktop
pnpm install
cd src-tauri
cargo check   # 单独 check, 不打包 (打包要图标等资源)
cd ../..
```

`target/release/qx.exe` 是单一二进制, 同时支持 `--daemon` / `--server` / 默认 CLI REPL / `--acp-mode` / `--standalone` 等模式.

---

## 2. 启动 Daemon (本地 Agent runtime)

```powershell
# 2a. 配置文件 (一次性, --generate-config 写到 ~/.qianxun/config.json)
.\target\release\qx.exe --generate-config

# 2b. 启动 daemon
.\target\release\qx.exe --daemon --port 23900
```

**输出**:
```
2026-06-02 11:39:40 INFO qx: 以 Daemon 模式启动（端口 23900）
2026-06-02 11:39:40 INFO qx::daemon: Daemon starting on 127.0.0.1:23900
2026-06-02 11:39:40 INFO qianxun_core::provider: [provider] creating provider: id=minimax model=MiniMax-M3 base_url=https://api.minimaxi.com/anthropic
2026-06-02 11:39:40 INFO qx::daemon: [daemon] session store initialized at C:\Users\maxu\.qianxun\daemon.db
```

数据落盘位置: `~/.qianxun/daemon.db` (SQLite, 3 张 `daemon_` 前缀表: `daemon_sessions` / `daemon_conversation_snapshots` / `daemon_event_log`).

---

## 3. 验证 Daemon (5 个核心 endpoint)

```powershell
# 3a. 健康检查
curl http://127.0.0.1:23900/v1/system/health
# → {"status":"ok"}

# 3b. 系统状态 (版本 + 阶段标识)
curl http://127.0.0.1:23900/v1/system/status
# → {"stage":"stage-2-sse-streaming","status":"running","version":"0.1.0"}

# 3c. 工具列表 (Stage 1 写死 8 个 builtin 工具)
curl http://127.0.0.1:23900/v1/tools
# → {"tools":[{"name":"read_text_file","description":"读取文件内容"}, ...]}

# 3d. 技能列表 (Stage 1 写空, 实际加载留 Stage 5)
curl http://127.0.0.1:23900/v1/skills
# → {"skills":[]}

# 3e. 创建会话 (POST)
curl -X POST http://127.0.0.1:23900/v1/chat/session
# → {"session_id":"sess_20260602_034516_151057"}

# 3f. 列出会话
curl http://127.0.0.1:23900/v1/chat/sessions

# 3g. 发送 prompt 走 SSE 流 (需要真 API key, 这里用占位)
curl -N -X POST http://127.0.0.1:23900/v1/chat/session/sess_xxx/prompt \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"hello"}]}'
# → data: {"type":"message_start",...}
#   data: {"type":"text_delta","text":"..."}
#   data: {"type":"message_stop"}
```

---

## 4. 启动 VPS Server (远端控制面, Stage 1+2+3)

```powershell
.\target\release\qx.exe --server --port 23901
```

**输出**:
```
2026-06-02 11:40:00 INFO qx: 以 VPS Server 模式启动（端口 23901）
2026-06-02 11:40:00 INFO qx::server: VPS Server starting on 0.0.0.0:23901
```

VPS Server 提供的 endpoint:
```powershell
# 健康
curl http://127.0.0.1:23901/api/health

# 用户登录 (Stage 1 已实现, JWT)
curl -X POST http://127.0.0.1:23901/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"admin"}'
# → {"token":"eyJ...","user":{...}}

# 列出所有 teams (Stage 3 新增)
curl http://127.0.0.1:23901/api/teams \
  -H "Authorization: Bearer <token>"

# 创建 team
curl -X POST http://127.0.0.1:23901/api/teams \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <token>" \
  -d '{"name":"My Team"}'

# WebSocket 端点 (Stage 2 实现的 WsHub)
wscat -c ws://127.0.0.1:23901/api/ws
# 立即 close (Stage 1 雏形), Stage 4 接完整 auth
```

数据落盘位置: `~/.qianxun/vps.db` (SQLite, 6 张表: `users` / `teams` / `team_members` / `team_projects` / `project_assignments` / `devices`).

---

## 5. 启动 Tauri Desktop (前端 + Rust 端)

**前置**: Tauri 2.0 第一次跑会下载 `windows-rs` / `webview2-com` / `tauri-plugin-*` 等 ~1.5GB 依赖,5-10 分钟.

```powershell
cd E:\git\maxu\qianxun\qianxun-desktop

# 5a. 前端 dev server (SvelteKit)
pnpm dev
# → http://127.0.0.1:5173/ (浏览器三栏 layout + mock 数据)

# 5b. Tauri native window (src-tauri/ Rust 端, 启动后是 native 桌面 app)
pnpm tauri dev
# → native window 启动, 顶栏显示 "Daemon: offline" (因为没连 Daemon)
# → 设置页配 Daemon URL http://127.0.0.1:23900 后, 状态变 connected

# 5c. 生产打包 (跨平台 installer, Stage 5 范围, 当前可尝试)
pnpm tauri build
# → src-tauri/target/release/bundle/{msi,nsis,dmg,deb,appimage}/
```

Tauri 项目结构:
- `src/` — SvelteKit 前端 (Svelte 5 + Tailwind v4 + shadcn-svelte)
- `src/lib/stores/` — `connection.svelte.ts` (4 态) / `session.svelte.ts` (12 事件) / `vps.svelte.ts` (3 态)
- `src/lib/sse/client.ts` — POST + ReadableStream 解析 SSE 12 事件
- `src/lib/components/chat/` — MessageBubble + InputBox + ConnectionBanner
- `src/lib/i18n/` — 起步 zh-CN + en, 10 个 key
- `src-tauri/` — Rust 端, 2 个 invoke command: `health_check` / `daemon_health_fetch`

---

## 6. 三端协同 (Stage 4 衔接)

```
┌─────────────────────────────────────────────┐
│ Tauri Desktop (前端 + Rust 端)              │
│   ↓ invoke('daemon_health_fetch', url)        │
│   ↓ listen('daemon://state-changed')          │
└──────────┬───────────────────────────────────┘
           │ HTTP / SSE
           ↓
┌─────────────────────────────────────────────┐
│ qx --daemon (本机端口 23900)                 │
│   ↑ provider.stream_completion(MiniMax-M3)   │
│   ↓ prompt_handler 接 provider 流             │
│   ↓ SseEvent 12 事件推回客户端               │
└──────────┬───────────────────────────────────┘
           │ SQLite
           ↓
~/.qianxun/daemon.db (3 张 daemon_ 表)

(可选 Stage 4 衔接)
┌─────────────────────────────────────────────┐
│ qx --server (远端 VPS 端口 23901)            │
│   ↑ WebSocket 双向                            │
│   ↓ Team / Project 路由 + RBAC              │
└──────────┬───────────────────────────────────┘
           │ SQLite
           ↓
~/.qianxun/vps.db (6 张表)
```

---

## 7. 常见问题

### Q1: 端口 23900 被占用
```
Error: Address already in use (os error 10048)
```
**解决**: 换端口 `.\target\release\qx.exe --daemon --port 23900`,或 `Stop-Process -Name qx -Force`.

### Q2: 启动 daemon 报 "cannot determine ~/.qianxun home dir"
**解决**: 设置 `USERPROFILE` 环境变量 (Windows): `$env:USERPROFILE = "C:\Users\<you>"`.

### Q3: API key 错误 (401/403)
**解决**:
- 确认 `$env:DEEPSEEK_API_KEY` 或 `$env:ANTHROPIC_AUTH_TOKEN` 已设置 (二选一即可)
- 切换 provider: 编辑 `~/.qianxun/config.json` 的 `active_provider` 字段
- 检查 key 是否过期: 试 `curl https://api.deepseek.com/v1/models` 或对应 endpoint

### Q4: Tauri 第一次跑很慢
**原因**: 下载 tauri 2.x 依赖链 (~1.5GB) + 编译 webview2-com + windows-rs 等原生 crate
**解决**: 耐心等 5-10 分钟, 后续增量编译 <10s

### Q5: Tauri 启动后 "Daemon: offline"
**原因**: 默认 `daemonUrl = 'http://127.0.0.1:23900'`, 但 daemon 没启动或端口不对
**解决**: 启动 daemon (`--daemon --port 23900`), 或在 Tauri 设置页改 daemonUrl

### Q6: Worker timeout 30 分钟被 kill
**原因**: mavis team plan hard cap 30 分钟
**解决**:
- 让 worker 早期写 deliverable.md (每 5 分钟一次 commit)
- 实时延长: `mavis team plan extend-timeout <plan_id> <task_id> --minutes 60` (单次 ≤60 分钟)
- 新 plan yaml: `timeout_ms: 7200000` (2h), 模板见 `.mavis/plans/plan-template.yaml`

---

## 8. 测试

```powershell
# 全 workspace 单测
cargo test --workspace

# 只跑 daemon
cargo test -p qianxun --bin qx daemon::

# 只跑 vps server
cargo test -p qianxun --bin qx server::

# 只跑 client (Stage 4 新增)
cargo test -p qianxun --bin qx client::

# qianxun-memory 闭环
cargo test -p qianxun-memory
```

期望输出: 60+ tests 全过 (Stage 1-4 累积).

---

## 9. 数据清理 (开发期)

```powershell
# 清掉 daemon 持久化 (重新开始)
Remove-Item "$env:USERPROFILE\.qianxun\daemon.db" -ErrorAction SilentlyContinue

# 清掉 vps server 持久化
Remove-Item "$env:USERPROFILE\.qianxun\vps.db" -ErrorAction SilentlyContinue

# 清掉 Tauri 用户态 (localStorage)
# 浏览器 DevTools → Application → Storage → Clear site data
```

---

## 10. 文档索引

| 文档 | 位置 | 内容 |
|---|---|---|
| 项目规则 | `CLAUDE.md` | 技术栈 + LLM Provider 配置 + 模块结构 |
| 三子项目规划 | `docs/30_子项目规划/{01-daemon.md, 02-vps-server.md, 03-tauri-desktop.md}` | 详细设计 |
| 共享契约 | `docs/30_子项目规划/_shared-contract.md` | API + 数据模型 + 协调规则 |
| 子系统状态 | `docs/10_事实源/{memory-state, daemon-state, ...}.md` | 真实状态快照 |
| Tauri README | `qianxun-desktop/README.md` | Tauri 端命令 + Stage 2-5 TODO |
| 阶段路线 | `docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md` | A-G 路线 + Stage 1-4 对应 |

---

**TL;DR**:
- Daemon: `.\target\release\qx.exe --daemon --port 23900` + `curl http://127.0.0.1:23900/v1/system/health`
- VPS: `.\target\release\qx.exe --server --port 23901`
- Tauri: `cd qianxun-desktop && pnpm dev` 或 `pnpm tauri dev`

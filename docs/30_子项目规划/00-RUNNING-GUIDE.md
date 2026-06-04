# 千寻运行说明 (Running Guide)

> 最后更新: 2026-06-03 | 状态: **Stage 1-10 全部完成**, 三端生产可用
>
> **53 个 commit** on origin / **283+ 个测试** 全过 / **26+10+8 = 44 个 daemon endpoint**
> / 10 个 Web Console 面板 / 1 个 Chat 视图 / 强密码 admin 登录 / 6 步 graceful shutdown.

本文档说明如何在本地开发 / 生产环境编译、运行、测试、部署、排查千寻的三个子项目:

- **Daemon** — 本机唯一 Agent runtime (`127.0.0.1:23900`), 含 Web Console SPA
- **VPS Server** — 远端控制面 (`127.0.0.1:23901`), 含 vanilla JS Web UI
- **Tauri Desktop** — 桌面客户端 (Svelte 5 + shadcn-svelte + Tauri 2.0 + stronghold)

---

## 0. 前置条件

| 工具 | 最低版本 | 用途 |
|---|---|---|
| **Rust** | 1.85+ (rust-toolchain.toml 锁 stable) | Daemon + VPS Server + Tauri Rust 端编译 |
| **Node.js** | 18+ (推荐 22) | Tauri 前端 SvelteKit dev/build + Web Console |
| **pnpm** | 9+ | Tauri 前端包管理 (lockfile 是 `pnpm-lock.yaml`) |
| **Tauri CLI** | 2.0+ (`cargo install tauri-cli --version "^2.0"`) | `pnpm tauri dev` / `pnpm tauri build` 调它 |
| **Webview2** | Windows 10+ 已自带 | Tauri 在 Windows 渲染所需 |
| **LLM API key** | 任一: `DEEPSEEK_API_KEY` 或 `ANTHROPIC_AUTH_TOKEN` | provider 鉴权 |
| **Playwright** | (可选) 装 `pnpm --dir qianxun/src/daemon/ui exec playwright install chromium` | 跑 E2E |

### 0.1 设置 API key

```powershell
# 走 minimax (推荐, Anthropic 兼容)
$env:ANTHROPIC_AUTH_TOKEN = "sk-xxx"

# 或走 deepseek
$env:DEEPSEEK_API_KEY = "sk-xxx"

# 也可以直接写 ~/.qianxun/config.json 的 "providers.deepseek.api_key" 或
# "providers.minimax.api_key" 字段, 启动时自动读 (Stage 1-10 全支持).
```

### 0.2 全局配置 (`~/.qianxun/config.json`)

格式示例 (Stage 7 升级后):
```json
{
    "active_provider": "minimax",   // 启动时 active 的 provider
    "providers": {
        "deepseek": { "api_key": "sk-...", "model": "deepseek-v4-flash", "base_url": "https://api.deepseek.com/anthropic" },
        "minimax":  { "api_key": "sk-...", "model": "MiniMax-M3",        "base_url": "https://api.minimaxi.com/anthropic" }
    },
    "agent":      { "max_turns": 50, "max_retries": 3 },
    "budget":     { "max_input_tokens": 100000, "max_output_tokens": 4096 },
    "compaction": { "enabled": true, "model_window": 1000000, ... }
}
```

切换 active provider **不需要重启 daemon**: 通过 Web Console `/llm` 面板点 "Activate", 或 `PUT /v1/config` (Stage 7a). 旧配置 stage 改 `active_provider` 字段也支持.

---

## 1. 编译 (一次性)

```powershell
cd E:\git\maxu\qianxun

# 1a. 编译所有 workspace crate (核心 + 记忆 + CLI/daemon/server 入口 + Tauri Rust)
cargo build --workspace --release
# 产物: target/release/qx.exe (主二进制) + target/release/qianxun_desktop.exe (Tauri)

# 1b. 编译 Tauri 前端
cd qianxun-desktop
pnpm install
pnpm run build     # SvelteKit → dist/

# 1c. 编译 Web Console 前端 (供 daemon serve 用)
cd ../qianxun/src/daemon/ui
pnpm install
pnpm run build     # SvelteKit → build/ 或 dist/
```

`target/release/qx.exe` 是**单二进制**, 同时支持 4 种入口:
- `--daemon --port 23900` (HTTP + Web Console)
- `--server --port 23901` (VPS Server)
- 默认 / `--tui` (REPL CLI)
- `--acp-mode` (Zed 集成)
- `--standalone` (TUI 内嵌, 不连 daemon)

---

## 2. 启动 Daemon (本机 Agent runtime + Web Console)

### 2a. 首次启动 (生成 admin 凭据)

```powershell
cd E:\git\maxu\qianxun
.\target\release\qx.exe --daemon --port 23900
```

**输出** (Stage 10a admin password 模式):
```
[admin-auth] First-time setup: generated admin credential.
[admin-auth] Password (save this — you can change it after login):
[admin-auth]   <24 字符的随机密码, e.g. mj4ap0Wvhh4xi7zUXU2uRw>
[admin-auth] Stored at: C:\Users\maxu\.qianxun\admin.cred
[daemon] Daemon starting on 127.0.0.1:23900
[daemon] Web UI disabled (no --ui-dist / QIANXUN_UI_DIST)
[provider] creating provider: id=minimax model=MiniMax-M3 base_url=https://api.minimaxi.com/anthropic
[daemon] session store initialized at C:\Users\maxu\.qianxun\daemon.db
[daemon] restored 0 session(s) from disk
[daemon] LLM provider manager initialized: 2 providers, active=minimax
```

**首次启动的密码必须保存** (写到 `~/.qianxun/admin.cred` 是 bcrypt hash, 不可逆).
后续用 Web Console 登录后, 在 `/settings` 面板 → "修改密码" 改.

### 2b. 启用 Web Console (推荐)

Daemon 启动时可以 serve SvelteKit SPA, 浏览器访问 `http://127.0.0.1:23900/ui/`:

```powershell
# 方法 1: CLI flag
.\target\release\qx.exe --daemon --port 23900 --ui-dist .\qianxun\src\daemon\ui\build

# 方法 2: 环境变量
$env:QIANXUN_UI_DIST = "E:\git\maxu\qianxun\qianxun\src\daemon\ui\build"
.\target\release\qx.exe --daemon --port 23900

# 路径不存在时 (e.g. 没 build UI): daemon 仍启动, /ui/* 返 503 + JSON
# 提示. 不 panic.
```

### 2c. 验证启动

```powershell
# 健康检查 (公开, 不用 auth)
curl http://127.0.0.1:23900/v1/system/health
# → {"status":"ok"}

# 根路径 (Stage 7 bugfix: 返服务自描述 JSON, 不是 401)
curl http://127.0.0.1:23900/
# → {"name":"qianxun-daemon","version":"0.1.0","endpoints":{...},"auth":"Bearer <jwt>..."}

# 登录拿 JWT (Stage 10a: 密码登录)
$resp = Invoke-WebRequest -Method POST -Uri http://127.0.0.1:23900/v1/auth/login -ContentType "application/json" -Body ('{"password":"<从 stderr 抄的密码>"}')
$token = ($resp.Content | ConvertFrom-Json).token
Write-Host "Token: $token"

# 用 token 调受保护 endpoint
curl -H "Authorization: Bearer $token" http://127.0.0.1:23900/v1/system/status
# → {"status":"running","version":"0.1.0","stage":"stage-10b-graceful-shutdown",...}
```

数据落盘位置:
- `~/.qianxun/daemon.db` — SQLite, 3 张 `daemon_` 前缀表 (sessions / snapshots / event_log)
- `~/.qianxun/admin.cred` — JSON `{password_hash, token_secret}`, mode 0o600 (Unix) / 限制 ACL (Windows)
- `~/.qianxun/mem.db` — `qianxun-memory` SQLite, 8 张表 (observations / sessions / summaries / raw_observations / tags + FTS 索引)

---

## 3. Web Console (10 个管理面板)

打开浏览器访问 `http://127.0.0.1:23900/ui/` (启用 `--ui-dist` 后):

**首次访问弹 admin 密码框**, 输 stderr 印的 24 字符密码 → 进主界面.

### 3.1 10 个面板 (按 sidebar 顺序)

| # | 路由 | 功能 | 关键操作 |
|---|---|---|---|
| 1 | `/chat` | 3 栏 (项目/会话/聊天) + 流式 SSE | 选 provider → 新建 session → 发 prompt |
| 2 | `/llm` | LLM providers 管理 | 列出 / 测试连接 / 切换 active / 增删 |
| 3 | `/skills` | Skills 列表 | reload / 启停 / 详情 |
| 4 | `/mcp` | MCP servers | 增 (stdio/HTTP) / 删 / 测试连接 |
| 5 | `/tools` | 内置工具 | 列表 / 详情 / 试用 (invoke) |
| 6 | `/memory` | 记忆管理 | session 列表 / 搜索 / 观察详情 / 删 |
| 7 | `/sessions` | Chat sessions | 列表 / cancel / pause / delete |
| 8 | `/config` | Config 查看 | (Stage 9c 只读, 改通过 PUT /v1/config) |
| 9 | `/system` | System 状态 | 资源指标 (CPU/mem/conns/uptime) + 日志 tail |
| 10 | `/settings` | Settings | 主题 (light/dark/system) / 语言 (zh-CN/en) / 改密码 / about |

### 3.2 主题 + 语言切换

- 主题: 立即生效 (mode-watcher 加/去 `dark` class), 持久化到 localStorage
- 语言: 立即切换 UI 文案 (svelte-i18n), 持久化到 localStorage
- 切换按钮在 TopBar 右上角

### 3.3 离线检测

`+layout.svelte` 顶部红 banner 出现条件: daemon 健康检查失败. 自动 30s 复检 + 手动 "重试" 按钮.

### 3.4 移动端响应式 (Stage 9c)

- `< sm (640px)`: Sidebar 隐藏, 汉堡菜单 + drawer
- `>= lg (1024px)`: 桌面 (Sidebar 完整展开)
- 中间断点 (sm-md): Sidebar icon-only

---

## 4. Tauri Desktop (桌面客户端)

### 4a. 启动

```powershell
cd E:\git\maxu\qianxun\qianxun-desktop

# Dev 模式 (HMR, 浏览器开新窗口)
pnpm tauri dev
# → native window 启动, 顶栏显示 "Daemon: offline" (因为没连 Daemon)
# → 设置页配 Daemon URL http://127.0.0.1:23900 后, 状态变 connected

# 仅前端 dev (无 native window, 浏览器)
pnpm dev
# → http://127.0.0.1:5173/

# 生产打包 (跨平台 installer)
pnpm tauri build
# → src-tauri/target/release/bundle/{msi,nsis,dmg,deb,appimage}/
```

### 4b. Tauri 项目结构

- `src/` — SvelteKit 前端 (Svelte 5 + Tailwind v4 + shadcn-svelte)
- `src/lib/stores/` — `connection.svelte.ts` (4 态) / `session.svelte.ts` (12 事件) / `vps.svelte.ts` (3 态) / `settings.svelte.ts`
- `src/lib/sse/` — `parser.ts` (12 事件解析) + `client.ts` (SSE 消费)
- `src/lib/ipc/` — `bridge.ts` (setSecret/getSecret/deleteSecret 走 Tauri invoke)
- `src/lib/components/chat/` — MessageBubble / InputBox / ConnectionBanner
- `src/lib/i18n/` — 起步 zh-CN + en, 80+ key
- `src-tauri/` — Rust 端, 2 invoke command + stronghold 凭据加密

### 4c. Stronghold 凭据加密 (Stage 6a + Stage 10b)

- 加密存: `setSecret(key, value, password)` 调 `invoke("set_secret", ...)` 走 Tauri stronghold vault
- 解密读: `getSecret(key, password)` 返原 value (密码错返 null)
- 删除: `deleteSecret(key, password)` (Stage 10b 新增) — 用于换 VPS token / 撤权 API key
- Web fallback (dev): base64 编码到 localStorage (不真加密, 仅脱敏明文)

---

## 5. VPS Server (远端控制面, 暂缓升级)

```powershell
.\target\release\qx.exe --server --port 23901
```

VPS Server 提供 endpoint:
```powershell
# 健康
curl http://127.0.0.1:23901/api/health

# 用户登录 (Stage 1 已实现, JWT)
curl -X POST http://127.0.0.1:23901/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"admin"}'

# 列出所有 teams
curl http://127.0.0.1:23901/api/teams \
  -H "Authorization: Bearer <token>"

# WebSocket 端点
wscat -c ws://127.0.0.1:23901/api/ws
```

数据落盘: `~/.qianxun/vps.db` (SQLite, 6 张表: `users` / `teams` / `team_members` / `team_projects` / `project_assignments` / `devices`).

**VPS Web UI (vanilla JS)** 暂不升级, 仍是 Stage 6c 那 992 行, 按用户决定保留原状.

---

## 6. 三端协同

```
┌─────────────────────────────────────────────┐
│ Tauri Desktop (前端 + Rust 端)              │
│   ↓ invoke('daemon_health_fetch', url)        │
│   ↓ invoke('set_secret'/'get_secret')         │
│   ↓ listen('daemon://state-changed')          │
└──────────┬───────────────────────────────────┘
           │ HTTP / SSE
           ↓
┌─────────────────────────────────────────────┐
│ qx --daemon (本机端口 23900)                 │
│   ├─ Web Console SPA (Stage 7a/7b/9c)         │
│   ├─ 26+10+8 = 44 endpoint                   │
│   ├─ JWT 鉴权 (Stage 6a) + admin 密码 (10a)  │
│   ├─ SSE 流 (12 事件)                         │
│   └─ Graceful shutdown 6 步 (Stage 10b)      │
└──────────┬───────────────────────────────────┘
           │ SQLite
           ↓
~/.qianxun/daemon.db (3 张 daemon_ 表)
~/.qianxun/admin.cred (bcrypt hash + JWT secret)
~/.qianxun/mem.db (qianxun-memory 8 表)

(可选)
┌─────────────────────────────────────────────┐
│ qx --server (远端 VPS 端口 23901)            │
│   ├─ Vanilla JS 控制台 (Stage 6c)             │
│   ├─ WebSocket Hub                            │
│   └─ Team / Project 路由 + RBAC              │
└──────────┬───────────────────────────────────┘
           │ SQLite
           ↓
~/.qianxun/vps.db (6 张表)
```

---

## 7. Graceful Shutdown (Stage 10b)

Daemon 收到 SIGINT / SIGTERM / Ctrl-C 时, 自动跑 6 步优雅关闭 (5s 内完成):

```
[daemon] received SIGINT (Ctrl-C)   (或 SIGTERM on Unix)
[daemon] step 1: shutdown_tx broadcast  (auto)
[daemon] step 2: axum 停止 accept 新连接   (auto, via with_graceful_shutdown)
[daemon] step 3: waiting for active connections to drain (max 30s)
[daemon] step 3: all connections drained in 312ms
[daemon] step 4: cancelling active sessions
[daemon] step 4: cancelled 3/3 session(s)
[daemon] step 5: flushing session store (WAL checkpoint)
[daemon] step 5: store flushed successfully
[daemon] step 6: graceful shutdown complete
```

**关键**:
- 不会丢活跃 session — `shutdown_all()` mark 所有 in-memory runtime 为 paused + 触发 SSE stop signal
- SQLite WAL 强制 checkpoint — `PRAGMA wal_checkpoint(TRUNCATE)` 确保 SIGTERM 后不丢数据
- 30s drain timeout 后 warn 继续, 不卡死

**Windows 注意**: `taskkill /F` 是 SIGKILL 等价, 走不到 graceful. 用 Ctrl-C in terminal 触发 SIGINT handler 才是 graceful 路径.

---

## 7b. Dev / Release / Build 工作流 (Stage 12 — `scripts/run.py`)

**核心**: 单 Python 脚本 (`scripts/run.py`) 统一所有编译/部署/启动, **完全删了** `qianxun/src/daemon/build.rs` (之前 `cargo build` 会自动调 pnpm, 跨语言依赖 + 双重跑). 改用 py 脚本显式控制, vite 自带 watch (dev) + 自带 build (release).

**Vite 自带 watch/build, py 脚本只 orchestrate**:
- `pnpm run dev` — Vite dev server 自带文件 watch + HMR
- `pnpm run build` — Vite 一行命令产 static
- py 脚本不重实现这两件事, 只管 spawn / kill / log / 模式分发

### 7b.1 `scripts/run.py` 4 模式 (单 entry point)

```bash
# 默认 (dev): 后台启 vite + 前台 cargo run daemon, daemon 反代 /ui → vite
python scripts/run.py

# release: pnpm build + cargo build --release + 跑 release 二进制
python scripts/run.py --release

# 只 build debug (CI 用), build 完退出
python scripts/run.py --build

# 只 build release (release CI 用)
python scripts/run.py --release --build

# 通用修饰
python scripts/run.py --port 23910         # 自定义端口
python scripts/run.py --no-vite            # dev 模式不启 vite (假设 vite 已在跑)
python scripts/run.py --skip-build         # 跳过 pnpm + cargo build (assume 已 build)
python scripts/run.py --ui-dev http://127.0.0.1:5174  # 反代 URL (默认)
python scripts/run.py --ui-dist <path>     # release 模式覆盖 UI dist 路径
```

### 7b.2 Dev 模式实际跑什么

```
06:07:09.347 ▸ step  模式 = DEV (debug + vite watch + 反代)
06:07:09.347 ℹ info  daemon 端口 : 23900 (--ui-dev → http://127.0.0.1:5174)
06:07:09.347 ℹ info  vite 端口   : 5174  (SvelteKit paths.base='/ui')
06:07:09.347 ℹ info  浏览器入口  : http://127.0.0.1:23900/ui
06:07:09.347 ℸ step  启 vite dev server (后台)
         (Vite 启动 818ms)
06:07:10.785 ✓   ok  vite:5174 up (via 127.0.0.1)
06:07:10.786 ▸ step  启 cargo run daemon (前台)
         (cargo 编译 + 启动, ~10-15s 首次, ~1-3s 增量)
06:10:13.914 ✓   ok  daemon:23900 up (via localhost)
06:10:13.914 ✓   ok  全部就绪 → http://127.0.0.1:23900/ui
06:10:13.914 ℹ info   改 svelte 后浏览器 Cmd-R (vite 自动 watch)
```

- 后台启 `pnpm.cmd run dev` (Vite 监听 5174, base='/ui', HMR 可用)
- 前台启 `cargo run` (daemon 监听 23900, `--ui-dev=http://127.0.0.1:5174`)
- daemon 反代 `/ui/*` → vite dev server
- Ctrl-C 优雅关 2 个子进程 (Windows: `taskkill /F /T /PID`, POSIX: `killpg SIGTERM`)

**浏览器入口**: `http://127.0.0.1:23900/ui` (跟 prod 一致, dev 体验跟生产同源).
**Vite HMR 备用**: `http://127.0.0.1:5174/ui` (要走 vite WS HMR 时用, 反代不支持 WS).

**改 svelte 后**: 浏览器手动 Cmd-R (Vite 已经在 dev 监听文件变更, 第二次请求直接命中新代码). 启动从 60s → 5s.

### 7b.3 Release 模式

```
06:11:00.001 ▸ step  模式 = RELEASE
06:11:00.001 ℹ info  step 1/3: pnpm build
06:11:00.001 ℹ info  $ pnpm.cmd run build
         (pnpm build ~10-30s)
06:11:25.123 ✓   ok  step 1/3: pnpm build done in 25.1s
06:11:25.123 ℹ info  step 2/3: cargo build --release
         (cargo --release 1-3min 首次)
06:13:48.789 ✓   ok  step 2/3: cargo build --release done in 143.7s
06:13:48.789 ▸ step  step 3/3: 跑 release daemon
         (前台运行, Ctrl-C 退出)
```

### 7b.4 日志格式 (人类易读本地时间)

- 本地时区 `HH:MM:SS.mmm` 毫秒精度 (`datetime.now().strftime("%H:%M:%S.") + microsecond // 1000`)
- 5 级 + 颜色 + emoji:
  - `▸ step` 紫色加粗 (模式切换/分阶段)
  - `ℹ info` 青色 (普通信息)
  - `✓ ok` 绿色 (成功)
  - `⚠ warn` 黄色 (警告)
  - `✗ err` 红色 (错误)
- 子命令前缀 `$ cmd args` 加粗, 步骤进度 `step 1/3:` 提示当前在哪步
- 全部 `flush=True` 实时打印, 不缓冲

### 7b.5 前置条件 + 端口冲突 + 系统代理

- **Python 3.8+** (用 stdlib, 无新依赖)
- **pnpm + node ≥ 18** (Vite 8 要求)
- 端口预检: 启动前 check 23900 (daemon) + 5174 (vite) 占用, 占用时返错给指引
- **系统代理干扰 (Windows 常见)**: `http_proxy=127.0.0.1:1080` (公司代理) 会让 curl/Invoke-WebRequest 走代理, 代理对 loopback 返 502. `run.py` **自动 unset** 这些 env var (`HTTP_PROXY`/`HTTPS_PROXY`/`http_proxy`/`https_proxy`/`ALL_PROXY`/`all_proxy`/`NO_PROXY`/`no_proxy`), 子进程走直连 loopback. 你自己手 curl 测时也得显式 `-Proxy $null` (PowerShell) 或 `curl --noproxy *` 跳过代理.

### 7b.6 为啥删 build.rs

之前 `qianxun/src/daemon/build.rs` 在 `cargo build` 时自动调 pnpm install + pnpm build, 跟 `scripts/run.py` 重复 + 跨语言依赖 + dev 跟 release 走两套 skip 逻辑. 删了之后:
- 单一入口 (`scripts/run.py`) 显式控制, 看脚本就知道发生了什么
- CI 不用装 pnpm 跑 `cargo build` (要么用 `python scripts/run.py --build`, 要么预 build)
- dev 模式不再需要 `QIANXUN_SKIP_UI_BUILD` 这个 hack (build.rs 都不存在了)

---

## 8. 测试

### 8a. Cargo (Daemon + VPS)

```powershell
# 全 workspace
cargo test --workspace
# 期望: 90+ tests pass (含 4 ignored = Stage 8 真 LLM 集成)

# 单模块
cargo test -p qianxun --bin qx daemon::           # 91 pass
cargo test -p qianxun --bin qx server::           # VPS (Stage 5 范围)
cargo test -p qianxun --bin qx client::           # 4 pass (Stage 6b)
cargo test -p qianxun-memory                      # memory 闭环

# 真 LLM 集成 (默认 ignore, 需显式 --include-ignored)
cargo test -p qianxun --bin qx daemon::llm_integration -- --include-ignored
# 4 pass (minimax + deepseek 真接)
```

### 8b. Tauri (Vitest + Stronghold E2E)

```powershell
cd E:\git\maxu\qianxun\qianxun-desktop

pnpm test                                       # 39 vitest pass
pnpm run check                                  # svelte-check 0/0

# Stronghold Rust 集成 (Argon2 KDF ~30s, 默认 ignore)
cd src-tauri
cargo test --test stronghold_e2e                 # 5 pass
cargo test --test stronghold_e2e -- --ignored   # 6 pass (含 1 慢的 set_twice_overwrites)
```

### 8c. Web Console (Vitest + Playwright)

```powershell
cd E:\git\maxu\qianxun\qianxun\src\daemon\ui

pnpm vitest run                                  # 154 pass
pnpm run check                                  # svelte-check 0/0

# Playwright E2E (Stage 8c)
pnpm exec playwright install chromium            # 一次性
pnpm exec playwright test                        # 5 spec pass (login / llm / skills / mcp / ops-panels)
```

### 8d. 总测试 (2026-06-03 实测)

| 端 | 数量 | 备注 |
|---|---|---|
| Daemon (cargo) | 91 | 含 4 ignored = Stage 8 真 LLM |
| Tauri (vitest) | 39 | 完整 Svelte 5 单测 |
| Tauri (stronghold) | 5 fast + 1 ignored | Rust 集成 |
| Web Console (vitest) | 154 | 全部 Svelte 5 + 10 面板 |
| Web Console (Playwright) | 5 | E2E |
| **合计** | **283+** | 全过 |

---

## 9. API Quick Reference (44 个 Daemon endpoint)

完整 schema 在 `docs/30_子项目规划/_shared-contract.md` §3.1/§3.1.1. 这里是分类速查.

### 9.1 Auth (3) — Stage 10a

| Method | Path | 跳过 auth | 用途 |
|---|---|---|---|
| POST | `/v1/auth/login` | ✓ | 密码 → 24h JWT |
| POST | `/v1/auth/change-password` | | 改密码 (需 old + new) |
| POST | `/v1/auth/logout` | | 清前端 token, daemon stateless |

### 9.2 System (5) — Stage 7b

| Method | Path | 跳过 auth | 用途 |
|---|---|---|---|
| GET | `/` | ✓ | 服务自描述 JSON (Stage 7 bugfix) |
| GET | `/v1/system/health` | ✓ | k8s probe |
| GET | `/v1/system/status` | ✓ | 运行时状态 |
| GET | `/v1/system/metrics` | | 资源 (CPU/mem/conns/uptime) |
| GET | `/v1/system/logs?lines=N` | | 日志 tail (默认 100, 上限 1000) |
| GET | `/v1/system/admin/rotate-token` | | 旋转 admin token (Stage 9c) |

### 9.3 LLM Providers (8) — Stage 7a

```
GET    /v1/llm/providers
GET    /v1/llm/providers/{id}
POST   /v1/llm/providers
PUT    /v1/llm/providers/{id}
DELETE /v1/llm/providers/{id}
POST   /v1/llm/providers/{id}/activate
POST   /v1/llm/providers/{id}/test
```

### 9.4 Chat / Session (5)

```
POST   /v1/chat/session                    # 创建
GET    /v1/chat/session/{id}              # 查
DELETE /v1/chat/session/{id}              # 删
POST   /v1/chat/session/{id}/prompt       # SSE 流
POST   /v1/chat/session/{id}/cancel       # 取消
POST   /v1/chat/session/{id}/pause        # 暂停
GET    /v1/chat/sessions                  # 列
```

### 9.5 Tools / Skills / MCP (5)

```
GET    /v1/tools
POST   /v1/tools/{name}/invoke            # 试用 (Stage 7a)
GET    /v1/skills                          # 列表
POST   /v1/skills                          # reload
POST   /v1/skills/{name}/toggle            # 启停
GET    /v1/mcp/servers
POST   /v1/mcp/servers
DELETE /v1/mcp/servers/{id}                # Stage 7a
POST   /v1/mcp/servers/{id}/test           # Stage 7a
```

### 9.6 Memory (3)

```
GET    /v1/memory/sessions
POST   /v1/memory/search
DELETE /v1/memory/observations/{id}        # Stage 7b
DELETE /v1/memory/sessions/{id}            # Stage 7b
```

### 9.7 Config (2)

```
GET    /v1/config                          # Stage 1
PUT    /v1/config                          # Stage 7b (含 hot-reload 信号)
```

### 9.8 Static UI (1)

```
GET    /ui/*                              # SvelteKit SPA (Stage 7a, 需 --ui-dist)
```

**SSE 12 事件 schema** 见 `_shared-contract.md` §3.2: `message_start` / `content_block_start` / `text_delta` / `thinking_delta` / `tool_use_delta` / `tool_use_complete` / `tool_result` / `content_block_stop` / `usage` / `message_delta` / `message_stop` / `error`.

---

## 10. 部署 (生产)

### 10a. 单 binary 部署 (推荐)

**Linux systemd**:

```ini
# /etc/systemd/system/qx-daemon.service
[Unit]
Description=Qianxun Daemon
After=network.target

[Service]
Type=simple
User=qianxun
Environment="ANTHROPIC_AUTH_TOKEN=sk-xxx"
Environment="QIANXUN_UI_DIST=/opt/qianxun/ui"
ExecStart=/opt/qianxun/qx --daemon --port 23900 --ui-dist /opt/qianxun/ui
Restart=on-failure
RestartSec=5s
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now qx-daemon
sudo systemctl status qx-daemon
sudo journalctl -u qx-daemon -f  # 跟踪日志
```

**Windows Service** (Stage 4a 留接口, 完整 dispatcher 留 4b):

```powershell
# 安装 (Stage 4a 写, 实测可用)
.\target\release\qx.exe --install-service
# → 写到 HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\QianxunDaemon

# 启停
sc start QianxunDaemon
sc stop QianxunDaemon
sc query QianxunDaemon
```

### 10b. Docker 部署 (VPS Server)

`docker-compose.yml` (Stage 4 已落地, 1 核 512MB 跑得动):

```yaml
version: '3.8'
services:
  qx-server:
    image: ghcr.io/imxood/qx-server:latest
    ports:
      - "23901:23901"
    environment:
      - JWT_SECRET=<change-me>
      - DATABASE_URL=sqlite:///data/vps.db
    volumes:
      - qx-data:/data
    restart: unless-stopped

volumes:
  qx-data:
```

```bash
docker compose up -d
docker compose logs -f qx-server
```

### 10c. 防火墙

- Daemon 绑 `127.0.0.1` (默认), **不暴露**外网. 远程访问走 VPS Server (经 auth + 转发)
- VPS Server 绑 `0.0.0.0:23901`, 需要防火墙白名单 (公司 VPN / 团队 IP 段)
- Tauri 桌面 + Web Console 都是用户面, 走 daemon 本地端口

---

## 11. 故障排查 (Troubleshooting)

### Q1: 端口 23900 被占用

```
Error: Address already in use (os error 10048)
```

**解决**:
- 换端口: `qx --daemon --port 23910`
- 杀旧进程: `Stop-Process -Name qx -Force` (Windows) / `pkill qx` (Linux)

### Q2: 启动 daemon 报 "cannot determine ~/.qianxun home dir"

**解决**: 设 `USERPROFILE` (Windows) 或 `HOME` (Unix) env var.

### Q3: API key 错误 (401/403 from upstream)

**解决**:
- 确认 `DEEPSEEK_API_KEY` 或 `ANTHROPIC_AUTH_TOKEN` 已设置 (二选一即可)
- 切换 provider: Web Console `/llm` 面板点 "Activate", 或编辑 `~/.qianxun/config.json` 的 `active_provider` 字段
- 测 key: Web Console `/llm` → 点 "测试连接" 按钮 (Stage 7a 加), 返 `{ok: true, latency_ms: <n>}` 即通

### Q4: 登录 admin 报 401 "密码错误"

**原因**: admin 密码忘了 / `~/.qianxun/admin.cred` 损坏 / 重新首启动后密码没更新

**解决** (3 选 1):
- Web Console 登录后, `/settings` 面板 → "修改密码" 改
- CLI: `curl -X POST -H "Authorization: Bearer <valid_token>" /v1/auth/change-password -d '{"old_password":"...","new_password":"..."}'`
- 强制重置 (慎用, 会清 admin 凭据): 删 `~/.qianxun/admin.cred`, 重启 daemon, stderr 印新密码

### Q5: Tauri 第一次跑很慢

**原因**: 下载 tauri 2.x 依赖链 (~1.5GB) + 编译 webview2-com + windows-rs 等
**解决**: 耐心等 5-10 分钟, 后续增量编译 <10s

### Q6: Tauri 启动后 "Daemon: offline"

**原因**: 默认 `daemonUrl = 'http://127.0.0.1:23900'`, 但 daemon 没启动或端口不对
**解决**:
- 启 daemon: `qx --daemon --port 23900`
- 或在 Tauri 设置页改 daemonUrl
- 测连通: `curl http://<daemonUrl>/v1/system/health` 应返 `{"status":"ok"}`

### Q7: Web Console 顶部红 banner "无法连接 daemon"

**原因**: daemon 进程挂了 / 端口不对 / 防火墙
**解决**:
- "重试" 按钮手动 check
- 看 daemon 进程是否在跑 (Task Manager / `ps aux | grep qx`)
- Web Console 30s 自动复检

### Q8: Graceful shutdown 卡住不退出

**原因**: 有活跃 SSE 连接不关 (e.g. 客户端 30s+ 不读流)
**解决**:
- Daemon 30s drain timeout 后自动 warn 继续, 仍会退出
- 看 `[daemon] step 3: timed out, N active conn(s) remain after 30s` 日志, 找是哪个客户端

### Q9: Worker timeout 30 分钟被 kill (mavis team plan)

**原因**: mavis team plan hard cap 30 分钟
**解决** (Stage 6c 验证多次):
- 早期写 deliverable.md (每 5 分钟一次 commit)
- 实时延长: `mavis team plan extend-timeout <plan_id> <task_id> --minutes 60`
- 新 plan yaml: `timeout_ms: 7200000` (2h), 模板见 `.mavis/plans/plan-template.yaml`
- **跨 stage 已知**: 30 min hard cap 是 engine 强制的, worker 写完代码会 commit 但 deliverable.md 可能丢; 主人 (Mavis) 手动 override_accept + 自己 commit, 详见 mavis memory

### Q10: 89+ cargo warnings 噪音

**原因**: Stage 1-6 累积, 大部分是 dead_code / unused_imports
**解决**: 暂不修 (影响 0, 修了反而可能引入新 bug). Stage 8+ 计划分批清, 按 `cargo fix --bin "qx" -p qianxun --tests` 跑一次能修 8 处.

### Q11: `--ui-dist` 路径错

**症状**: `Web UI dist path does not exist: ... (/ui/* will return 503)`
**解决**:
- 跑 `pnpm --dir qianxun/src/daemon/ui build` 先 build UI
- 确认路径是 SvelteKit 输出的 `build/` (adapter-static) 或 `dist/`, 不是源码目录
- Web Console 是 SPA 模式, fallback 到 index.html, 单页应用

### Q12: SQLite "database is locked"

**原因**: WAL 模式下多 writer 冲突 (Stage 7a in-memory LlmProviderManager 多 writer 罕见)
**解决**:
- 看 `~/.qianxun/daemon.db-wal` 文件大小 (正常 < 1MB)
- `PRAGMA wal_checkpoint(TRUNCATE);` 手动 checkpoint
- 删 `-wal` 和 `-shm` 文件 (强制重置, 慎用, 会丢未 commit 数据)

### Q13: 强密码没保存

**症状**: 首次启动 stderr 印的 24 字符密码丢了
**解决**:
- 删 `~/.qianxun/admin.cred`, 重启 daemon, 会重新生成
- 上面的 admin token 也跟着变, 之前签的 JWT 全部失效 (需重新登录)

### Q14: VPS Server 启动失败

**症状**: `VPS server requires team_db initialization`
**解决**:
- 跑 `qx --server --init-db` 初始化 SQLite schema
- 删 `~/.qianxun/vps.db` 重置 (会丢 team / project 数据)

---

## 12. 数据清理 (开发期)

```powershell
# 清 daemon 持久化 (重置)
Remove-Item "$env:USERPROFILE\.qianxun\daemon.db*" -ErrorAction SilentlyContinue

# 清 admin 凭据 (下次启动会重置密码)
Remove-Item "$env:USERPROFILE\.qianxun\admin.cred" -ErrorAction SilentlyContinue

# 清 vps server
Remove-Item "$env:USERPROFILE\.qianxun\vps.db*" -ErrorAction SilentlyContinue

# 清 Tauri 用户态 (localStorage)
# 浏览器 DevTools → Application → Storage → Clear site data

# 清 Web Console 用户态
# 浏览器 DevTools → Application → Storage → Clear site data
# (会清 localStorage 里的 admin token / theme / language 偏好)
```

---

## 13. 文档索引

| 文档 | 位置 | 内容 |
|---|---|---|
| **本文件** | `docs/30_子项目规划/00-RUNNING-GUIDE.md` | 三子项目编译/运行/测试/部署/排查 (Stage 1-10) |
| 项目规则 | `CLAUDE.md` | 技术栈 + LLM Provider 配置 + 模块结构 |
| 三子项目规划 | `docs/30_子项目规划/{01-daemon.md, 02-vps-server.md, 03-tauri-desktop.md}` | 详细设计 |
| **Web Admin Console 规划** | `docs/30_子项目规划/01b-daemon-web-console.md` | 10 面板 / 17 endpoint / 3 sub-stage (Stage 7a/7b/7c 范围) |
| 共享契约 | `docs/30_子项目规划/_shared-contract.md` | API + 数据模型 + 协调规则 |
| 子系统状态 | `docs/10_事实源/{memory-state, daemon-state, ...}.md` | 真实状态快照 |
| Tauri README | `qianxun-desktop/README.md` | Tauri 端命令 + Stage 2-5 TODO |
| 阶段路线 | `docs/20_工作项/2026-06-01_TUI性能与Agent开发工具优化/阶段路线.md` | A-G 路线 + Stage 1-4 对应 |
| **工作项: Web Console** | `docs/20_工作项/2026-06-02_DaemonWebAdminConsole规划/README.md` | Stage 7 实施跟踪 |

---

**TL;DR**:
- **Daemon**: `qx --daemon --port 23900` + (可选) `--ui-dist <path>` 启 Web Console
  - 首次启动 stderr 看 admin 密码
  - 浏览器 `http://127.0.0.1:23900/ui/` 进 10 面板管理
  - Ctrl-C / SIGTERM 触发 6 步 graceful shutdown
- **Tauri**: `cd qianxun-desktop && pnpm tauri dev`
- **VPS**: `qx --server --port 23901` (控制台 vanilla JS, 暂缓升级)
- **测试**: `cargo test --workspace` + `pnpm --dir qianxun-desktop test` + `pnpm --dir qianxun/src/daemon/ui vitest run` = 283+ 全过
- **故障**: 见 §11 (14 个常见问题)

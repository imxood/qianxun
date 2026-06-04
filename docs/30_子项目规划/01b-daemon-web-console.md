# Daemon Web Admin Console (Track A 扩展)

> 创建: 2026-06-02 | 状态: 规划中 | Mavis 协调
>
> 本文件是 `docs/30_子项目规划/01-daemon.md` 的**扩展**, 详细化 Phase 4b
> 的 "Web UI 静态文件" 一项. 涉及 daemon Rust 端 + 新建 SvelteKit
> 项目 `qianxun/src/daemon/ui/`.

## §1 背景与定位

### 1.1 为什么需要 Web Console

千寻 daemon 是**本机唯一持有 AgentLoop / API Key / Memory / Tools / Skills / MCP 的进程**.
Tauri 桌面端是**用户面** (跟 daemon 聊天), 但**管理面**缺失:
- LLM provider 怎么加 / 删 / 切?  现在要手动改 `~/.qianxun/config.json`
- 装了个新 skill 怎么加载?       现在要重启 daemon
- 启了一个 MCP server 怎么管?   现在只有 CLI add, 无 list / delete
- daemon 状态怎么看?            现在只能 curl `/v1/system/status` 看 JSON
- 记忆能不能清一下?             现在没有 UI, 只能 SQL 直连

Web Console 是 daemon 自带的**管理面板**, 浏览器访问 `http://127.0.0.1:23900/ui/` 即用.
跟 Tauri 桌面端的差异:

| 维度 | Tauri 桌面 | Web Console |
|---|---|---|
| 角色 | **用户面** (聊天/工作流) | **管理面** (LLM/Skills/MCP/Tools) |
| 部署 | 装机客户端 | daemon 启动时 serve 静态文件 |
| 用户 | 终端用户 (每天打开) | 运维/高级用户 (偶尔打开) |
| 数据 | 流式聊天 (主要) | 列表/表单/CRUD (主要) |
| 栈 | Svelte 5 (独立 Tauri 窗口) | Svelte 5 (浏览器 SPA, daemon 进程内 serve) |

### 1.2 不做什么 (non-goals)

- **不做聊天主入口** — 聊天归 Tauri 桌面 / TUI. Web Console 只在
  Stage 7c 加一个**次要** Chat 视图 (给远程调试用).
- **不做多用户/多 daemon 聚合** — 那是 VPS Server 控制台的职责.
  Web Console 只管**当前本机 daemon**.
- **不做远程控制** — Web Console 只连本机 daemon (`127.0.0.1:23900`),
  不暴露给外网 (绑 `127.0.0.1` 而非 `0.0.0.0`).
- **不替换 VPS 控制台** — VPS 控制台 (vanilla JS, Stage 6c) 仍管 VPS 侧
  (team / 项目 / 设备), Web Console 仍管 daemon 侧 (LLM / Skills / MCP / Tools).
  两个 UI **不共享代码**, 但共享 `_shared-contract.md` 数据模型 §6.

## §2 架构概览

### 2.1 单二进制自包含

```
┌─────────────────────────────────────────────────────┐
│  qx.exe / qx binary (单进程)                        │
│                                                     │
│  ┌──────────────────────────────────────────────┐   │
│  │  axum router                                 │   │
│  │  ├─ /         → 301 → /ui/                   │   │
│  │  ├─ /ui/      → tower-http ServeDir          │   │
│  │  │              (qianxun/src/daemon/ui/dist) │   │
│  │  │              SPA fallback to index.html   │   │
│  │  ├─ /v1/*     → JSON API (JWT auth)          │   │
│  │  └─ /healthz  → 200 (liveness, public)       │   │
│  └──────────────────────────────────────────────┘   │
│                                                     │
│  ┌──────────────────────────────────────────────┐   │
│  │  AgentLoopHost + Skills + MCP + Memory + ...  │   │
│  └──────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
                      ▲
                      │ HTTP / SSE
                      │
                ┌─────┴──────┐
                │ 浏览器     │
                │ (用户)     │
                └────────────┘
```

### 2.2 静态文件加载策略

**dev**: Vite dev server (`:5173`) 跑 `qianxun/src/daemon/ui/`, 用 vite proxy
转发 `/v1/*` 到 daemon (`:23900`). 两个进程.

**prod**: SvelteKit `pnpm build` 产出静态 `dist/` (单 SPA bundle), daemon
启动时 `ServeDir::new(ui_dist_path).fallback(ServeFile::new("index.html"))`
serve 出去. 一个进程.

**dist 路径解析** (启动时):
1. CLI flag `--ui-dist <path>` (覆盖默认)
2. 环境变量 `QIANXUN_UI_DIST`
3. 默认: `<exe 同级>/ui/` (release) 或 `<workspace>/qianxun/src/daemon/ui/dist/` (debug)
4. 路径不存在 → 启动 warn, `/ui/*` 返 503 + 提示 `pnpm --dir qianxun/src/daemon/ui build`

**单二进制 embedding** (Stage 8+ 选做, 不在 Stage 7 范围): 用 `rust-embed`
把 dist/ 编进二进制. 现在先 on-disk, 简化.

### 2.3 前端项目结构 (新建)

```
qianxun/src/daemon/ui/           # SvelteKit SPA
├── package.json
├── svelte.config.js              # adapter-static (无 SSR)
├── vite.config.ts                # dev: 5173, prod: build to dist/
├── tailwind.config.js
├── src/
│   ├── app.html
│   ├── app.css                   # tailwind 入口
│   ├── lib/
│   │   ├── components/
│   │   │   ├── ui/               # shadcn-svelte (button, card, dialog, ...)
│   │   │   ├── layout/           # Sidebar, TopBar, Layout
│   │   │   └── common/           # DataTable, Empty, Loading, Error
│   │   ├── stores/               # auth, theme, i18n
│   │   ├── api/                  # fetchWithAuth, llm.ts, skills.ts, mcp.ts, ...
│   │   ├── i18n/                 # zh-CN.json, en.json
│   │   └── types/                # 跟 daemon 共享的 schema (TS type)
│   └── routes/
│       ├── +layout.svelte        # Sidebar + TopBar + slot
│       ├── +page.svelte          # / → 重定向到 /llm (默认页)
│       ├── llm/                  # LLM 管理
│       │   ├── +page.svelte      # 列表
│       │   └── [id]/+page.svelte # 详情/编辑
│       ├── skills/               # Skills 管理
│       ├── mcp/                  # MCP 管理
│       ├── tools/                # Tools 管理
│       ├── memory/               # Memory 管理
│       ├── sessions/             # Chat sessions 管理
│       ├── config/               # Config 管理
│       ├── system/               # System 状态
│       ├── settings/             # Settings (主题/语言/token)
│       └── chat/                 # Chat (Stage 7c 选做)
└── tests/
    └── *.test.ts
```

## §3 Web Console 模块清单

按"管理面"角色, 10 个模块. 标 ★ 的是**核心** (Stage 7a), 标 ☆ 的是**次要** (Stage 7b/7c).

| # | 模块 | 路由 | 核心功能 | stage |
|---|---|---|---|---|
| ★1 | **LLM 管理** | `/llm` | Provider 列表 / 新增 / 编辑 / 删除 / 切 active / 注入 key (keyring) / 测试连接 | 7a |
| ★2 | **Skills 管理** | `/skills` | Skill 列表 / 详情 (manifest) / 启停 / 重载 | 7a |
| ★3 | **MCP 管理** | `/mcp` | Server 列表 / 新增 (stdio/HTTP) / 编辑 / 删除 / 测试连接 | 7a |
| ★4 | **Tools 管理** | `/tools` | Tool 列表 / 详情 (schema) / 试用 (invoke) | 7a |
| ☆5 | **Memory 管理** | `/memory` | Session 列表 / 搜索 / 观察详情 / 删除观察 | 7b |
| ☆6 | **Sessions 管理** | `/sessions` | Chat session 列表 / 详情 (messages + tokens) / pause/resume/cancel/delete | 7b |
| ☆7 | **Config 管理** | `/config` | 当前 config 查看 / 编辑表单 / 持久化 / hot-reload | 7b |
| ☆8 | **System 状态** | `/system` | Dashboard 卡片: 健康 / uptime / 资源 (CPU/内存) / 活跃连接数 / 日志查看 (v2) | 7b |
| ☆9 | **Settings** | `/settings` | 主题 (light/dark/system) / 语言 (zh-CN/en) / token 配置 / 关于 | 7c |
| ☆10 | **Chat (次要)** | `/chat` | 项目列表 + 3 栏布局 + 流式聊天 (复用 Tauri 组件) | 7c |

### 3.1 关键概念区分: Chat Session vs Memory Session

**两个完全不同的概念, URL 相似但语义不同, 容易混淆**:

| 维度 | Chat Session (`/v1/chat/sessions/{id}`) | Memory Session (`/v1/memory/sessions/{id}`) |
|---|---|---|
| **存储** | `~/.qianxun/daemon.db` (daemon SessionStore) | `~/.qianxun/mem.db` (qianxun-memory SQLite) |
| **生命周期** | 单次聊天会话 (用户开 + 聊 + 关) | 跨聊天的长程记忆 (自动从 chat 提取 + consolidate) |
| **数据** | 完整 message 列表 + snapshot + event log | 观察 (observations) + 摘要 (summaries) + 原始记录 |
| **Web UI 入口** | `/chat` (主入口) + `/sessions` (管理) | `/memory` (管理) |
| **典型操作** | cancel / pause / delete 单个 session | 搜索 observations / 删 整个 memory session |
| **持久化** | 用户主动 (关 chat 触发) | 自动 (memory observer 钩子) |

**关系**: Chat Session 中的某些事件 (e.g. user 输入, LLM 响应) 会通过
`MemoryObserver` 钩子自动转成 Memory Session 的 observations. 一个 Chat Session
可能产生 0+ 个 observations, 这些 observations 归属对应的 Memory Session.

**新增 endpoint** (Stage 12):
- `GET /v1/memory/sessions/{id}/observations` — 列某 Memory Session 的所有 observations
  (Web UI 点 Memory 面板 session 后, 右侧观察详情)
- 这 endpoint 容易跟 `/v1/chat/session/{id}/prompt` 混淆 (路径都是 `/session/{id}/...`).
  实际差别: chat 的是 conversation 消息流, memory 的是 observations 观察记录.

## §4 API Gap 分析 (Stage 7 范围)

### 4.1 已有 endpoint (Stage 1-6 已实现)

| 端点 | 方法 | 状态 | 用途 |
|---|---|---|---|
| `/v1/system/health` | GET | ✅ 已有 | 公开健康检查 |
| `/v1/system/status` | GET | ✅ 已有 | 公开状态 |
| `/v1/config` | GET | ✅ 已有 | 读 config |
| `/v1/tools` | GET | ✅ 已有 | 工具列表 |
| `/v1/skills` | GET | ✅ 已有 | 技能列表 |
| `/v1/mcp/servers` | GET / POST | ✅ 已有 | 增 MCP |
| `/v1/memory/sessions` | GET | ✅ 已有 | 记忆会话 |
| `/v1/memory/search` | POST | ✅ 已有 | 记忆搜索 |
| `/v1/chat/session` | POST | ✅ 已有 | 创建会话 |
| `/v1/chat/session/{id}` | GET / DELETE | ✅ 已有 | 查/删会话 |
| `/v1/chat/session/{id}/prompt` | POST | ✅ 已有 | SSE 流 |

### 4.2 需要**新增**的 endpoint (Stage 7 实现)

| 端点 | 方法 | 优先级 | 用途 | 归属 |
|---|---|---|---|---|
| `/ui/*` | GET | ★ | 静态文件 serve (SPA + fallback) | Stage 7a (router) |
| `/v1/llm/providers` | GET | ★ | 列出所有 LLM provider | Stage 7a |
| `/v1/llm/providers/{id}` | GET | ★ | 单个 provider 详情 (key **不**返) | Stage 7a |
| `/v1/llm/providers` | POST | ★ | 新增 provider (key 写 keyring) | Stage 7a |
| `/v1/llm/providers/{id}` | PUT | ★ | 更新 provider (含 key 替换) | Stage 7a |
| `/v1/llm/providers/{id}` | DELETE | ★ | 删除 provider | Stage 7a |
| `/v1/llm/providers/{id}/activate` | POST | ★ | 切 active | Stage 7a |
| `/v1/llm/providers/{id}/test` | POST | ★ | 测试连接 (发个最小请求) | Stage 7a |
| `/v1/skills` | POST | ★ | 重载全部 skills | Stage 7a |
| `/v1/skills/{name}/toggle` | POST | ★ | 启/停 skill (写 memory 或 config) | Stage 7a |
| `/v1/mcp/servers/{id}` | DELETE | ★ | 删 MCP server | Stage 7a |
| `/v1/mcp/servers/{id}/test` | POST | ★ | 测试 MCP 连接 | Stage 7a |
| `/v1/tools/{name}/invoke` | POST | ★ | 试用 tool (测试用, daemon 直接调, 不走 LLM) | Stage 7a |
| `/v1/chat/sessions` | GET | ☆ | 列所有会话 (含 status filter) | Stage 7b |
| `/v1/chat/session/{id}/cancel` | POST | ☆ | 取消正在跑的 prompt | Stage 7b |
| `/v1/chat/session/{id}/pause` | POST | ☆ | 暂停会话 (Stage 8 完整, Stage 7b 留接口) | Stage 7b |
| `/v1/config` | PUT | ☆ | 写 config (含 hot-reload 通知) | Stage 7b |
| `/v1/memory/observations/{id}` | DELETE | ☆ | 删观察 | Stage 7b |
| `/v1/memory/sessions/{id}` | DELETE | ☆ | 删整个 session | Stage 7b |
| `/v1/memory/sessions/{id}/observations` | GET | ★ (Stage 12 补) | 列某 session 下的 observations (Memory 面板点 session 后看观察详情) | Stage 7b + 12 |
| `/v1/system/metrics` | GET | ☆ | 资源指标 (CPU/内存/conns/uptime) | Stage 7b |
| `/v1/system/logs` | GET | ☆ | 日志查看 (tracing JSON 输出) | Stage 7b |

**预估** 18 个新 endpoint. 30 min cap → 拆 3 stage, 每 stage 5-6 个 + 1 个 UI 模块.

> **2026-06-03 补 (Stage 12)**: `GET /v1/memory/sessions/{id}/observations` 实际
> 归属 Stage 7b 计划但当时漏实现. Svelte 端 `memory.ts:listObservations`
> 早已调, daemon router 没注册, 返 404. Stage 12 补:
> 1. `qianxun-memory` 加 `MemoryCore::list_observations()` 方法 + `Observation` struct
> 2. `qianxun/src/daemon/router.rs` 加 `memory_session_observations` handler + 3 个 cargo 测试
> 3. Svelte `MemoryObservation` type 修正对齐 daemon schema (id/session_id/timestamp/data/created_at)
> 4. memory 页面 UI 字段渲染更新
> 5. `is_auth_skipped_path` 加 `/_app/*` 防御 (若 SvelteKit `paths.base` 改了)
> 6. 2 个 Svelte integration test 覆盖 (mock fetch, 验证 URL + 字段)

> **2026-06-03 文档清理**: 旧 01b §4.1 表里列了 `GET /v1/system/config`, 但
> 代码里没这个 endpoint (只有 `/v1/config`). RUNNING-GUIDE 已删该行.

## §5 Stage 拆分 (3 sub-stage, 30 min cap)

### Stage 7a — 架构 + 4 核心管理面板 (LLM / Skills / MCP / Tools)

**Track A (Daemon Rust 端)**:
1. `qianxun/src/daemon/router.rs`: 加 `.nest_service("/ui", ServeDir::new(ui_dist).fallback(...))`
2. `qianxun/src/daemon/service.rs` 或 `mod.rs`: 加 `--ui-dist` CLI flag + `QIANXUN_UI_DIST` env var
3. `qianxun/src/daemon/llm_providers.rs` (新): 增 `LlmProviderManager` (CRUD + keyring)
4. `qianxun/src/daemon/router.rs`: 加 8 个 LLM endpoint (4 上面 §4.2 表)
5. `qianxun/src/daemon/router.rs`: 加 3 个 Skills/MCP/Tools endpoint (reload, toggle, delete, invoke)
6. `qianxun-core/src/`: 加 keyring 集成 (`keyring` crate) — env var fallback 保留

**Track A' (新建 Web UI)**:
7. `qianxun/src/daemon/ui/` 整个新项目: SvelteKit + Svelte 5 + Vite + Tailwind + shadcn-svelte + adapter-static
8. 路由: `+layout.svelte` (Sidebar + TopBar) + `/llm` + `/skills` + `/mcp` + `/tools`
9. API client: `lib/api/{llm,skills,mcp,tools}.ts` 走 `fetchWithAuth(url, { Bearer })`
10. 集成: vite dev proxy `/v1/*` → `127.0.0.1:23900`

**验证** (跟其它 stage 一致):
- `cargo build -p qianxun --bin qx` 0 错误
- `cargo test daemon::router daemon::llm` 全过 (含新 endpoint)
- `pnpm --dir qianxun/src/daemon/ui install` 0 错
- `pnpm --dir qianxun/src/daemon/ui build` 0 错, dist/ 产出
- `pnpm --dir qianxun/src/daemon/ui test` (vitest) ≥ 8 测试 (4 模块 × 2)
- 起 daemon, 浏览器访问 `http://127.0.0.1:23900/ui/` 看到 sidebar + 4 面板

**预估代码量**: ~1500 行 Rust + ~1500 行 TS/Svelte
**30 min cap 内能完成**: 紧但可行, 跟 Stage 6c 规模相当 (那 1874 行是上限参考).

### Stage 7b — 4 次要面板 (Memory / Sessions / Config / System)

**Track A (Daemon Rust 端)**:
1. 加 8 个 endpoint (上面 §4.2 表的 ☆ 项: list sessions, cancel, pause, put config, delete obs, delete memory session, metrics, logs)
2. `qianxun/src/daemon/metrics.rs` (新): CPU/内存/连接数 (用 `sysinfo` crate, 评估传递依赖)
3. `qianxun/src/daemon/router.rs`: 8 个新 endpoint 路由

**Track A' (Web UI)**:
4. Web UI 加 4 路由 + 视图: `/memory` / `/sessions` / `/config` / `/system`
5. 主题系统集成 (mode-watcher + Tailwind dark mode)
6. i18n 起步 (svelte-i18n + zh-CN.json + en.json)

**验证**:
- cargo test ≥ 5 新 endpoint 测试
- vitest ≥ 8 新面板测试
- 浏览器能切 light/dark/system 主题, 切 zh-CN/en

**预估代码量**: ~800 行 Rust + ~1200 行 TS/Svelte
**30 min cap**: 宽松, 留 buffer 应对 sysinfo 传递依赖评估.

### Stage 7c — Chat + Settings + **Web 响应式 (非移动端)**

> **2026-06-02 调整**: 用户决定**移动端 (iOS + Android) 用 Flutter**, 不在
> Web Console 里做原生移动端. Stage 7c 因此**只做 Web 响应式** (浏览器
> 在窄屏下能用), 完整 Flutter 移动端留 Stage 8 独立项目.

**Track A' (Web UI)**:
1. Settings 面板: `/settings` (主题切换 UI + 语言切换 + token 配置 + about)
2. Chat 视图: `/chat` (项目列表 + 3 栏布局 + 流式聊天) — 复用 `qianxun-desktop/src/lib/components/ui` 组件
3. **Web 响应式** (Tailwind md/lg 断点) + 移动端浏览器导航 (汉堡菜单, drawer)
   - 移动端浏览器 (Chrome/Safari 窄屏) 能用, **不**打包成 app
4. 错误边界 + 离线检测 (daemon unreachable 时显示)
5. PWA manifest 起步 (后续 Stage 8 Flutter 完整实现)

**验证**:
- vitest ≥ 6 (chat 流 + settings 切换 + Web 响应式断点)
- 浏览器 (含窄屏模拟器) 走通所有 10 路由
- Lighthouse mobile score ≥ 80

**预估代码量**: ~200 行 Rust (无) + ~1500 行 TS/Svelte
**30 min cap**: 紧, 可能要拆 7c / 7d (但优先尝试 7c 一气呵成)

## §6 鉴权 & 安全

### 6.1 复用现有 JWT 机制

Web Console 跟 Tauri 桌面端 / TUI / ACP 用**同一套** `Authorization: Bearer <jwt>`:
- daemon 启动时生成 / 读 `QIANXUN_JWT_SECRET` (HS256)
- Web Console 首次访问 `/ui/` 跳转 `/ui/login` 弹**密码框** (不是 token 框)
- daemon 校验**单一管理密码** (env var `QIANXUN_ADMIN_PASSWORD`, bcrypt 比对)
- 校验通过后 daemon 签发 JWT (sub=admin, role=admin, exp=24h) 给浏览器
- 浏览器 localStorage 存 token, 后续 API 走 `Authorization: Bearer <token>`

**Stage 7a 简化**: 用现有 JWT 机制, **不**做密码框 — daemon 启动时随机生成 token
打印到 stderr, 用户复制到浏览器. Stage 7b/8 加密码框 + bcrypt.

### 6.2 绑定 127.0.0.1 (默认)

`qx --daemon` 默认绑 `127.0.0.1:23900` (不绑 `0.0.0.0`).
Stage 8+ 加 `--bind 0.0.0.0` flag + 强制要求 admin 密码.

### 6.3 CSP (Content Security Policy)

Web Console 走 `tower-http::set_header` 加 CSP:
- `default-src 'self'` (不引外部 CDN)
- `script-src 'self'` (无 inline script)
- `style-src 'self' 'unsafe-inline'` (Tailwind 需要)
- `connect-src 'self'` (API 同源)
- `img-src 'self' data:` (允许内联 favicon)

## §7 构建 & 部署集成

### 7.1 workspace 集成

`qianxun/src/daemon/ui/` 是 SvelteKit 项目, **不**进 Cargo workspace (Cargo 不管 node 依赖).
但 `qianxun/Cargo.toml` 加 `build.rs` 跑 `pnpm --dir ui build` (在 cargo build 时自动 build UI).

或者更简单: 在 `qianxun/build.rs` 调 `pnpm --dir qianxun/src/daemon/ui build`,
**仅**在 `cargo build --release` 时跑, debug 跳过 (dev 模式用 Vite dev server).

### 7.2 dev workflow

```bash
# Terminal 1: daemon (debug build, 不需要 UI 静态文件)
cargo run -- --daemon --port 23900

# Terminal 2: Vite dev server (HMR)
pnpm --dir qianxun/src/daemon/ui dev
# 浏览器访问 http://127.0.0.1:5173 (vite dev)
# Vite proxy /v1/* → 127.0.0.1:23900
```

### 7.3 prod workflow

```bash
# Build daemon (含 UI build via build.rs)
cargo build --release -p qianxun --bin qx

# Run daemon (serve dist/)
./target/release/qx --daemon --port 23900
# 浏览器访问 http://127.0.0.1:23900/ui/
```

### 7.4 跨平台 build

- **Windows**: build.rs 调 `pnpm.cmd` 而不是 `pnpm` (Cargo 不自动加 .exe 后缀)
- **macOS / Linux**: 正常 `pnpm`
- **CI**: 先 `pnpm install --frozen-lockfile` 再 `cargo build`

## §8 跨子项目一致性

### 8.1 跟 Tauri 桌面端共享

| 共享项 | 实现 |
|---|---|
| **shadcn-svelte 组件** | 从 `qianxun-desktop/src/lib/components/ui/` **复制**到 `qianxun/src/daemon/ui/src/lib/components/ui/` (Stage 7c 起). 后续 Stage 8 抽到独立 `qianxun-ui/` 包 |
| **API client schema** (TS type) | 复制 `qianxun-desktop/src/lib/types/api.ts` → 同样复制. 不共享代码, 共享 schema |
| **API base URL** | 都走 `http://127.0.0.1:23900` |
| **JWT 鉴权** | 同一个 secret, 同一个 Claims schema |

### 8.2 跟 VPS Server 控制台的关系

| 项 | Web Console (本机 daemon) | VPS 控制台 (vanilla JS, 暂不动) |
|---|---|---|
| 栈 | Svelte 5 + Tailwind + shadcn-svelte | vanilla JS (Stage 6c) |
| 管啥 | LLM/Skills/MCP/Tools (本机 daemon) | Team/项目/设备 (VPS 远端) |
| 端口 | 23900 (daemon) | 23901 (VPS) |
| API | `/v1/llm/*`, `/v1/skills`, `/v1/mcp/*`, `/v1/tools` | `/api/admin/teams`, `/api/projects`, `/api/devices` |
| 鉴权 | JWT (sub=admin) | JWT (sub=user, role=owner/admin/member) |

**互不依赖**, 但都遵守 `_shared-contract.md` §6 数据模型.

## §9 风险 & 缓解

| 风险 | 等级 | 缓解 |
|---|---|---|
| keyring crate 传递依赖 (libsecret/Windows credential manager) | 中 | 评估 Stage 7a 起, env var fallback 保留 |
| sysinfo crate 传递依赖 (system metrics) | 中 | Stage 7b 评估, 评估不过则改用 `/proc/self/status` 手读 |
| rust-embed 编 dist 进二进制 (Stage 8) | 低 | Stage 7 不做, 保留扩展点 |
| SvelteKit adapter-static build 慢 (>30s) | 低 | build.rs 仅 release 跑, debug 跳过 |
| 浏览器跨域 (Vite dev :5173 → daemon :23900) | 低 | vite.config 设 `server.proxy`, 同源 e2e |
| Web Console 跟 Tauri 桌面组件不一致 | 中 | 复制 + schema 校验, Stage 8 抽独立包 |

## §10 开放问题

| 问题 | 决策点 | 建议 |
|---|---|---|
| Web Console 默认端口是否跟 daemon 端口分开? | daemon `--ui-port` 单独控制, 还是共用 23900? | **共用 23900** (路径 `/ui/`), 简化部署 |
| LLM key 注入走 keyring 还是 daemon 进程内加密? | 哪个 Rust 库? 跨平台一致性? | **先用 env var**, Stage 7b 评估 keyring |
| Chat 视图放 Web Console 是不是冗余 (Tauri 已有)? | Stage 7c 是否做? | **做简化版**, 主要给远程调试 (ssh 端口转发后浏览器看) |
| Web Console 国际化做不做? | Stage 7b i18n? | **做基础 2 语言** (zh-CN + en), 后续按需加 |
| System 日志查看是否在 Stage 7b 范围? | `/v1/system/logs` 实现复杂度? | **Stage 7b 做 tail 模式 (最近 N 行)**, 完整搜索留 Stage 8 |
| **移动端用 Flutter 还是 Tauri 移动?** | 哪个栈? | **Flutter** (2026-06-02 用户决定), 留 Stage 8 独立项目 |
| **Stage 8 Flutter 移动端从哪起步?** | 项目结构 + 共享 daemon API + 跟 Web 端 UI 一致? | 详见 [未来规划 §11](#11-未来规划-stage-8-flutter-移动端) |

## §11 未来规划 (Stage 8 — Flutter 移动端)

> **2026-06-02 新增**: 用户决定移动端用 Flutter, 不在 Web Console 里做
> 原生移动端. 下面是 Stage 8 的**概要** (不展开, 留给专门的 `08-flutter-mobile.md`).

### 11.1 一句话定位

**千寻 Flutter 移动端** — iOS + Android 一次写, Material 3 设计, 消费
daemon HTTP API (跟 Web Console / Tauri 桌面端**同一套** backend).

### 11.2 跟三端的角色分工

| 端 | 栈 | 角色 | 部署 |
|---|---|---|---|
| **Web Console** | Svelte 5 + Tailwind + shadcn-svelte | 管理面 (daemon 本地) | daemon 进程内 serve |
| **Tauri 桌面** | Tauri 2.0 + Svelte 5 | 用户面 (桌面聊天) | 装机客户端 |
| **Flutter 移动** | Flutter + Material 3 | 用户面 (移动聊天) | iOS App Store + APK |
| **VPS Server** | axum + vanilla JS 控制台 | 控制面 (team/项目) | Docker |

### 11.3 Flutter 项目结构 (Stage 8 规划, 待详化)

```
qianxun-mobile/                       # Flutter 项目根
├── pubspec.yaml
├── lib/
│   ├── main.dart                     # MaterialApp + 路由
│   ├── core/
│   │   ├── api/                      # daemon HTTP client (Dio + interceptor)
│   │   ├── auth/                     # JWT 存储 (flutter_secure_storage)
│   │   └── theme/                    # Material 3 主题 (千寻品牌色)
│   ├── features/
│   │   ├── chat/                     # 聊天 (复用 Web 端 sse 流式逻辑)
│   │   ├── projects/                 # 项目列表
│   │   ├── settings/                 # 设置 (主题/语言/token)
│   │   └── console/                  # 管理面 (LlmProviders/Skills/MCP/Tools 列表)
│   └── shared/                       # widgets, utils
├── ios/                              # iOS Runner
├── android/                          # Android Runner
└── tests/                            # widget_test, integration_test
```

### 11.4 Stage 8 拆分 (预估, 30 min cap 模板)

| Stage | 范围 |
|---|---|
| **8a** | Flutter 脚手架 + Material 3 主题 + API client + Auth (JWT 存 secure storage) |
| **8b** | 聊天功能 (SSE 消费 + MessageBubble + InputBox) + 离线队列 (sqflite) |
| **8c** | 项目列表 + 设置 + 简易 console (只读) |
| **8d** | iOS build 配置 + Android build 配置 + 真机/模拟器测试 + 发布脚本 |

### 11.5 跟现有端的关系

- **不复用 Web 组件** (Flutter 不是 web, 重新写) — 但共享 API 契约 (`_shared-contract.md` §3.1/§3.2)
- **不替换 Tauri 桌面** — Flutter 只移动端, Tauri 只桌面
- **不引 PWA** (Web 响应式) — Flutter 是 native, 不走 PWA
- **共享 design tokens** — 千寻品牌色 `--accent: #ff7a3d` 同步到 Flutter ThemeData

### 11.6 详细规划待写

不放在本文件 (`01b-daemon-web-console.md` 是 Web Console 专属), 后续
单独建 `docs/30_子项目规划/04-flutter-mobile.md` 详细化.

---

## §12 与现有文档的关系

- **01-daemon.md** Phase 4b 一节更新 (从"暂不实施" → "Stage 7 拆 7a/7b/7c 实施")
- **01-daemon.md** §4.2 架构图 (第 108 行) 加 `/ui/*` 详细说明
- **_shared-contract.md** §3.1 加 §4.2 列表的 17 个新 endpoint
- **00-RUNNING-GUIDE.md** 加 Web Console 启动 + 访问说明
- **02-vps-server.md** §3 加"两套 UI 不共享代码" 声明
- **(待写) 04-flutter-mobile.md** Stage 8 详细设计

---

**下一步**: 写完本文件后, 同步更新上述 4 个文件. 然后开 Stage 7a plan
(参考 Stage 6a/6b/6c 模板, 30 min cap, 拆 2-3 task).

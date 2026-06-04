# 工作项: Daemon Web Admin Console 规划 (Stage 7-10c)

> 创建: 2026-06-02 | 收尾: 2026-06-03
> 状态: ✅ Stage 7a->10c 全部完成, 8 个 plan 决策全 accept/override_accept, 收尾归并到 `04-kanban-design.md` 关联 Web Console 索引
>
> **目标**: 给 qianxun daemon 加一个**管理控制台** (Svelte 5 SPA),
> 浏览器访问 `http://127.0.0.1:23900/ui/` 即可管理 LLM / Skills /
> MCP / Tools / Memory / Sessions / Config / System 8 大模块 + Chat 视图 + Settings.

## 背景

千寻 daemon 是本机唯一持有 AgentLoop / API Key / Memory / Tools / Skills
/ MCP 的进程. Tauri 桌面端是**用户面** (跟 daemon 聊天), 但**管理面**缺失:

- LLM provider 增删改查?    -> 要手动改 `~/.qianxun/config.json`
- 装了新 skill 怎么加载?    -> 要重启 daemon
- 启了 MCP server 怎么管?  -> 只有 CLI add, 无 list / delete
- daemon 状态怎么看?       -> 只能 curl JSON

Web Console 是 daemon 自带的**管理面板**, 解决以上痛点.

## 交付物 (全部 ✅)

| 文件 | 状态 | 用途 |
|---|---|---|
| `docs/30_子项目规划/01b-daemon-web-console.md` | ✅ 创建 | **主规划文件** (10 面板 / 17 endpoint / 3 sub-stage) |
| `docs/30_子项目规划/01-daemon.md` | ✅ 更新 | 加 §15 索引 + 更新 Phase 4b 表 + 架构图 |
| `docs/30_子项目规划/_shared-contract.md` | ✅ 更新 | §3.1.1 加 17 个新 endpoint |
| `qianxun/src/daemon/ui/` (Stage 7a 起) | ✅ 创建 | SvelteKit + Svelte 5 + Vite + Tailwind + shadcn-svelte |
| `qianxun/src/daemon/router.rs` (Stage 7a 起) | ✅ 创建 | 加 `/ui/*` 静态文件 serve + 17 个新 endpoint |

## Stage 拆分完成情况 (2026-06-03)

| Stage | 范围 | Daemon Rust | Web UI | 验收 | 关键 commit |
|---|---|---|---|---|---|
| **7a** | 架构 + 4 核心面板 (LLM/Skills/MCP/Tools) | ~1500 行 | ~1500 行 | override_accept x 2 | `b5995d3` / `129bdf0` |
| **7b** | 4 次要面板 (Memory/Sessions/Config/System) + 主题 + i18n | ~800 行 | ~1200 行 | override_accept (随 7a) | `f1ab1d0` |
| **8** | 真 LLM E2E 集成测试 (minimax + deepseek) + Tauri SSE parser + WebUI Playwright | ~400 行 | ~300 行 | override_accept x 3 | `fc750d9` / `6876c6e` |
| **9c** | Settings 面板 + Chat 视图 (3 栏 + SSE 流 + 5 组件 + 31 tests) + 响应式/error boundary/CSP | ~150 行 | ~1200 行 | override_accept x 3 | `7116b12` / `79cd9fb` / `8501a22` / `01970ae` / `1d24069` |
| **10a** | Admin password -> short-lived JWT (bcrypt + admin.cred) + 密码登录 UI | ~200 行 | ~300 行 | accept | `368a978` / `9f69c3f` / `d416d7b` |
| **10b1** | Daemon graceful shutdown 6 步 (SIGINT+SIGTERM -> 编排函数) | ~150 行 | — | accept (session 内手做) | `b6be7dd` |
| **10b2** | Tauri stronghold 真测 (deleteSecret API + delete tests) | — | ~200 行 | accept (session 内手做) | `edfca94` |
| **10c** | Tauri 8 SSE parser + WebUI 6 daemon API integration 单测 + scripts/dev-e2e.mjs + 交付报告 | — | ~500 行 | accept (verifier 真 PASS, 5+ stage 来首次) | `c54502f` / `82bd4b1` |

合计: **~3000 行 Daemon + ~5000 行 Web UI**, 8 个 mavis plan 全部完成, 17 个 commit 落地.

## 风险与缓解 (实际遭遇)

| 风险 | 等级 | 实际遭遇 | 缓解 |
|---|---|---|---|
| keyring crate 传递依赖 | 中 | Stage 7a 评估, 决定不引, 走 env var fallback | env var QIANXUN_*_API_KEY 直接读 |
| sysinfo crate 传递依赖 | 中 | Stage 7b 评估, 决定不引, 走 process metrics from /proc | metrics 端点返回 JSON 简单聚合 |
| SvelteKit build 慢 | 低 | pnpm build ~20s, 可接受 | (无缓解) |
| Web Console 跟 Tauri 桌面组件不一致 | 中 | Stage 9c 复制 Tauri MessageBubble/InputBox/ThreeColumnLayout 组件, schema 校验, 后续 v2 抽独立包 | (待 v2 抽包) |
| 30 min producer timeout | 高 | 17 次 override_accept 中大部分是 producer 30 min timeout, 代码已 commit | orchestrator 亲自 cargo test + pnpm check + git log 找回 |
| Stage 10 task B 范围超 30 min | 中 | 1 次 reject (graceful + stronghold), 拆分到 session 内手做 | session 内 commit `b6be7dd` + `edfca94` |

## 决策记录 (实际采用)

| 决策 | 选择 | 理由 |
|---|---|---|
| 前端栈 | Svelte 5 + SvelteKit + Vite + Tailwind + shadcn-svelte | 跟 Tauri 桌面同栈 (用户规则: 所有 web 端统一栈) |
| 部署 | daemon 启动时 serve dist/ (单二进制) | 单进程, 无额外 node runtime 依赖 |
| 路径 | `qianxun/src/daemon/ui/` | 跟 daemon 紧耦合, 方便 build.rs 集成 |
| 端口 | 共用 daemon 端口 23900 (路径 `/ui/`) | 简化部署, 不开额外端口 |
| 鉴权 | Stage 7a 用 token + JWT; Stage 10a 加 bcrypt 密码 + short-lived JWT | 7a 简化启动, 10a 加安全 |
| 单二进制 embedding | Stage 7-10 不做, v6 Kanban 评估 rust-embed | 简化, 避免早期过度工程 |

## 完成情况 (2026-06-03)

- ✅ 规划文档完成 (`01b-daemon-web-console.md`)
- ✅ Stage 7a plan YAML + 实施 (LLM/Skills/MCP/Tools 4 核心面板)
- ✅ Stage 7b plan + 实施 (4 次要面板 + 主题 + i18n)
- ✅ Stage 8 plan + 实施 (真 LLM E2E + Tauri SSE parser + WebUI Playwright)
- ✅ Stage 9c plan + 实施 (Settings + Chat + 响应式)
- ✅ Stage 10a plan + 实施 (Admin password + JWT)
- ✅ Stage 10b1 session 内手做 (Daemon graceful shutdown)
- ✅ Stage 10b2 session 内手做 (Tauri stronghold 真测)
- ✅ Stage 10c plan + 实施 (14 补单测)
- ✅ 收尾归并到 `04-kanban-design.md` 关联 Web Console 索引 (后续 v6 Kanban MVP-4 引用)

## 对应 plans 决策

- **Stage 7a plan** (`.mavis/plans/stage7a-plan.yaml`): daemon-stage7a-llm-and-serve + webui-stage7a-scaffold-4-panels, override_accept x 2
- **Stage 8 plan** (`.mavis/plans/stage8-decision.json`): daemon-stage8-llm-e2e + tauri-stage8-llm-e2e + webui-stage8-management-e2e, override_accept x 3
- **Stage 9c plan** (`.mavis/plans/stage9c-web-console-finish.yaml` + `stage9c-decision.json`): webui-stage9c-settings + webui-stage9c-chat + webui-stage9c-responsive, override_accept x 3
- **Stage 10 plan** (`.mavis/plans/stage10-security-hardening.yaml` + `stage10-decision.json`): stage10-admin-password accept + stage10-tauri-stronghold-graceful reject + stage10-fill-stage8-tests accept

详见 `06-mavis-执行历史.md` §2 阶段总表 + §3 关键决策 (17 次 override_accept + 1 次 reject 详情).

## 关键 commit (按时间顺序)

Stage 7a 起: `b5995d3` / `129bdf0` / `f1ab1d0` / `fc750d9` / `6876c6e` / `7116b12` / `79cd9fb` / `8501a22` / `01970ae` / `1d24069` / `368a978` / `9f69c3f` / `d416d7b` / `c54502f` / `edfca94` / `b6be7dd` / `82bd4b1`

## 关键文档

- 主规划: `docs/30_子项目规划/01b-daemon-web-console.md`
- 父规划: `docs/30_子项目规划/01-daemon.md` §15
- 共享契约: `docs/30_子项目规划/_shared-contract.md` §3.1.1
- mavis 执行历史: `docs/30_子项目规划/06-mavis-执行历史.md`
- v6 Kanban 引用: `docs/30_子项目规划/04-kanban-design.md` (后续 MVP-3 + MVP-4 引用 Web Console 索引)

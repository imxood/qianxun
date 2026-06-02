# 工作项: Daemon Web Admin Console 规划 (Stage 7)

> 创建: 2026-06-02 | 状态: 规划完成, 待开工 | Owner: TBD
>
> **目标**: 给 qianxun daemon 加一个**管理控制台** (Svelte 5 SPA),
> 浏览器访问 `http://127.0.0.1:23900/ui/` 即可管理 LLM / Skills /
> MCP / Tools / Memory / Sessions / Config / System 8 大模块.

## 背景

千寻 daemon 是本机唯一持有 AgentLoop / API Key / Memory / Tools / Skills
/ MCP 的进程. Tauri 桌面端是**用户面** (跟 daemon 聊天), 但**管理面**缺失:

- LLM provider 增删改查?    → 要手动改 `~/.qianxun/config.json`
- 装了新 skill 怎么加载?    → 要重启 daemon
- 启了 MCP server 怎么管?  → 只有 CLI add, 无 list / delete
- daemon 状态怎么看?       → 只能 curl JSON

Web Console 是 daemon 自带的**管理面板**, 解决以上痛点.

## 交付物

| 文件 | 状态 | 用途 |
|---|---|---|
| `docs/30_子项目规划/01b-daemon-web-console.md` | ✅ 创建 | **主规划文件** (10 面板 / 17 endpoint / 3 sub-stage) |
| `docs/30_子项目规划/01-daemon.md` | ✅ 更新 | 加 §15 索引 + 更新 Phase 4b 表 + 架构图 |
| `docs/30_子项目规划/_shared-contract.md` | ✅ 更新 | §3.1.1 加 17 个新 endpoint |
| `qianxun/src/daemon/ui/` (Stage 7a 起) | ⏸️ 待开工 | SvelteKit + Svelte 5 + Vite + Tailwind + shadcn-svelte |
| `qianxun/src/daemon/router.rs` (Stage 7a 起) | ⏸️ 待开工 | 加 `/_ui/*` 静态文件 serve + 17 个新 endpoint |

## Stage 拆分

| Stage | 范围 | Daemon Rust | Web UI | 30 min cap |
|---|---|---|---|---|
| **7a** | 架构 + 4 核心面板 (LLM/Skills/MCP/Tools) | ~1500 行 | ~1500 行 | 紧 (上限) |
| **7b** | 4 次要面板 (Memory/Sessions/Config/System) + 主题 + i18n | ~800 行 | ~1200 行 | 宽松 |
| **7c** | Settings + Chat (次要) + 移动端 | ~200 行 | ~1500 行 | 紧 |

## 风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| keyring crate 传递依赖 | 中 | Stage 7a 评估, env var fallback 保留 |
| sysinfo crate 传递依赖 | 中 | Stage 7b 评估, 不行改手读 /proc |
| SvelteKit build 慢 | 低 | build.rs 仅 release 跑 |
| Web Console 跟 Tauri 桌面组件不一致 | 中 | 复制 + schema 校验, Stage 8 抽独立包 |

## 决策记录

| 决策 | 选择 | 理由 |
|---|---|---|
| 前端栈 | Svelte 5 + SvelteKit + Vite + Tailwind + shadcn-svelte | 跟 Tauri 桌面同栈 (用户规则: 所有 web 端统一栈) |
| 部署 | daemon 启动时 serve dist/ (单二进制) | 单进程, 无额外 node runtime 依赖 |
| 路径 | `qianxun/src/daemon/ui/` | 跟 daemon 紧耦合, 方便 build.rs 集成 |
| 端口 | 共用 daemon 端口 23900 (路径 `/ui/`) | 简化部署, 不开额外端口 |
| 鉴权 | Stage 7a 用 token + JWT; Stage 7b+ 加密码 | 7a 简化启动, 7b 加安全 |
| 单二进制 embedding | Stage 7 不做, Stage 8 评估 rust-embed | 简化, 避免早期过度工程 |

## 下一步

1. ✅ 规划文档完成 (本工作项)
2. ⏸️ 写 Stage 7a plan YAML (跟 Stage 6a/6b/6c 同模板, 30 min cap, 3 worker)
3. ⏸️ 跑 Stage 7a (LLM/Skills/MCP/Tools 4 核心面板)
4. ⏸️ 跑 Stage 7b (4 次要面板 + 主题 + i18n)
5. ⏸️ 跑 Stage 7c (Settings + Chat + 移动端)

## 关键文档

- 主规划: `docs/30_子项目规划/01b-daemon-web-console.md`
- 父规划: `docs/30_子项目规划/01-daemon.md` §15
- 共享契约: `docs/30_子项目规划/_shared-contract.md` §3.1.1

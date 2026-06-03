# TUI 性能与 Agent 开发工具优化 (归档)

> 创建时间: 2026-06-01 | 归档: 2026-06-03
> 状态: ⚠️ 阶段路线 A-G 实际未执行, 实际走的是 Web Console + Stage 8-10 路线

## 归档原因

本工作项 2026-06-01 创建时, 规划了 7 个阶段 (A-G), 计划 2026-06-02 ~ 2026-06-03 执行. 但实际推进中:

- **TUI 性能最小闭环 (Phase B) 走的是 session 内手做, 不走 plan** — 关键 commit `f724653` + `8f613ec` + `28bb68a` + `e40b8e1` 在 2026-06-01 ~ 2026-06-02 完成 (4 项关键能力: 脏标记驱动渲染 + 增量行缓存 + 帧率限制 + 工具折叠).
- **Agent 开发工具优化 (Phase C-G) 走的是 Daemon Web Admin Console 路线, 不是 TUI 路线** — 实际落地为 Stage 7a-10c (8 个 mavis plan), 详见 `2026-06-02_DaemonWebAdminConsole规划/` 和 `06-mavis-执行历史.md` §2.

本工作项的路线 A-G 内容保留, 作为未来 TUI 单机性能优化的参考, 但实际执行路线已分叉到 Web Console.

## 目标 (归档时的目标)

- 优化 `qx` TUI 的渲染性能和交互体验, 支撑长回复, 工具输出和持续对话.
- 将 `qx` 逐步设计成更好的 Agent 开发工具, 以 Daemon 作为统一运行时.
- 修正文档事实源与真实代码状态不一致的问题, 避免后续路线判断失真.

## 范围

- 覆盖独立 TUI, Daemon runtime, Memory, MCP, Skills, Agent Patterns, Tool Policy 和测试体系.
- 不在本工作项中直接定义 VPS Server 和 Web UI 的完整产品方案.
- 不把 1M 上下文设计为 TUI 全量渲染目标. 全量内容应进入 transcript/session store/context manager, TUI 只渲染可见窗口.

## 关联文档

- `docs/architecture.md`
- `docs/daemon-design.md`
- `docs/memory-design.md`
- `docs/mcp-design.md`
- `docs/skills-design.md`
- `docs/agent-pattern-design.md`
- `docs/20_工作项/2026-06-01_qx交互式TUI调研/`
- `docs/20_工作项/2026-06-02_DaemonWebAdminConsole规划/` — 实际走 Web Console 路线的规划工作项

## 产出

- [阶段路线.md](阶段路线.md) — 路线 A-G 完整内容, 留作未来参考

## 当前关键结论 (2026-06-01 起草时)

- 当前 TUI 已接近 Codex 风格的 inline 交互, 但仍存在长输出下逐帧重算渲染行的问题.
- 当前 Daemon 是 HTTP 骨架, 还不是唯一 Agent runtime.
- Memory, MCP, Skills 的设计文档与真实代码存在不同程度偏差, 需要先治理状态标注.
- 更好的 Agent 开发工具应采用 Daemon-first: TUI/ACP/Web 作为 thin client, 统一复用 session, memory, tools, provider 和 context policy.

## 实际落地 (2026-06-01 ~ 2026-06-03)

### Phase A 部分落地

- ✅ 修正 `docs/README.md` Phase 状态 (commit `f8916e0 docs: 文档事实源治理 — 修正模块状态表 + 创建 10_事实源/`)
- ✅ 建立 `docs/10_事实源/` (memory-state.md / daemon-state.md / 架构设计.md 等)
- ⚠️ 架构设计.md 大部分内容已迁到 `docs/30_子项目规划/`, 旧文件未完全删除

### Phase B 落地 (TUI 性能最小闭环)

- ✅ 脏标记驱动渲染 (commit `28bb68a perf: 脏标记驱动渲染 + 增量行缓存 + 帧率限制 + 工具折叠 + 基准测试`)
- ✅ 增量行缓存
- ✅ 帧率限制
- ✅ 工具折叠
- ✅ 越界 panic 修复 (commit `e40b8e1 fix: live_message_lines cached_lines 越界 panic + 初始消息缓存同步`)
- ✅ 交互层重构 (commit `8f613ec feat: 完成 TUI 交互层重构`)
- ✅ Inline Viewport + 回滚缓冲 + 消息队列 (commit `f724653 refactor(tui): Inline Viewport + 回滚缓冲 + 消息队列`)

### Phase C-G 走 Web Console 路线 (不走 TUI 路线)

实际工作项: `2026-06-02_DaemonWebAdminConsole规划/` (Stage 7a-10c, 8 个 mavis plan 全部完成).

- ✅ SvelteKit SPA + 4 核心管理面板 (LLM/Skills/MCP/Tools) — Stage 7a
- ✅ 4 次要面板 (Memory/Sessions/Config/System) + 主题/i18n — Stage 7b
- ✅ 真 LLM E2E 集成测试 (minimax + deepseek) — Stage 8
- ✅ Settings 面板 + Chat 视图 (3 栏 + SSE 流 + 5 组件 + 31 tests) — Stage 9c
- ✅ Admin password → short-lived JWT (bcrypt) + 密码登录 UI — Stage 10a
- ✅ Daemon graceful shutdown 6 步 + Tauri stronghold 真测 + 14 补单测 — Stage 10b/10c

## 对应 plans 决策

路线型工作项, 无对应 mavis plan. 阶段路线 A-G 内容由 session 内手工起草, 关键时间窗 2026-06-01. 实际落地走的是:

- **TUI 性能部分 (Phase B)**: session 内手做, 不走 plan. 4 项关键 commit 在 2026-06-01 ~ 2026-06-02 完成.
- **Agent 开发工具部分 (Phase C-G)**: 走 mavis plan 编排, 拆为 Stage 7a-10c 共 8 个 plan, 详见 `06-mavis-执行历史.md` §2 阶段总表 + §6 工作项对应表 + `2026-06-02_DaemonWebAdminConsole规划/` 工作项.

## 关联文档

- `docs/30_子项目规划/06-mavis-执行历史.md` — mavis 编排执行历史
- `docs/30_子项目规划/04-kanban-design.md` — 多 Agent Kanban 架构 (v6, 含 TUI Kanban 视图 MVP-4)
- `docs/20_工作项/2026-06-02_DaemonWebAdminConsole规划/` — 实际走 Web Console 路线的规划工作项
- `docs/20_工作项/2026-06-01_qx交互式TUI调研/` — TUI 调研 (P0 已完成, 收尾)
- `docs/10_事实源/架构设计.md` — 当前架构事实源

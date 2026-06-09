# TUI 性能与 Agent 开发工具优化

> 创建时间: 2026-06-01
> 状态: 方案已形成, 待拆分执行

## 目标

- 优化 `qx` TUI 的渲染性能和交互体验, 支撑长回复, 工具输出和持续对话.
- 将 `qx` 逐步设计成更好的 Agent 开发工具, 以 `qianxun-runtime` 作为统一运行时 (TUI/ACP/Desktop 共享 RuntimeApi).
- 修正文档事实源与真实代码状态不一致的问题, 避免后续路线判断失真.

## 范围

- 覆盖独立 TUI, Runtime, Memory, MCP, Skills, Agent Patterns, Tool Policy 和测试体系.
- 不在本工作项中直接定义 VPS Server 和 Web UI 的完整产品方案.
- 不把 1M 上下文设计为 TUI 全量渲染目标. 全量内容应进入 transcript/session store/context manager, TUI 只渲染可见窗口.

## 关联文档

- `docs/10_事实源/runtime-state.md` (Runtime 子系统状态)
- `docs/10_事实源/memory-state.md` (Memory 子系统状态)
- `docs/10_事实源/skills-state.md` (Skills 子系统状态)
- `docs/30_决策/ADR-0003_desktop_2mode.md` (现行架构决策)
- `docs/30_子项目规划/_shared-contract.md` (RuntimeApi 契约)

## 产出

- 阶段路线已并入 04b-tauri-runtime-integration.md (2026-06-09 文档清理)

## 当前关键结论

- 当前 TUI 已接近 Codex 风格的 inline 交互, 但仍存在长输出下逐帧重算渲染行的问题.
- 当前 daemon 退居"非桌面"场景 (VPS / 远程), 桌面端通过 qianxun-runtime 共享 runtime (ADR-0003).
- Memory, MCP, Skills 的设计文档与真实代码存在不同程度偏差, 需要先治理状态标注.
- 更好的 Agent 开发工具应采用 Runtime-first: TUI/ACP/Desktop 共享 `RuntimeApi`, 统一复用 session, memory, tools, provider 和 context policy.

## 下一步

1. 把 TUI 跟 `qianxun-runtime` 集成 (沿用 desktop 端 RuntimeApi, 走 in-process).
2. 补充 TUI 性能验收记录.
3. 稳定结论迁入正式事实源, 阶段性过程继续留在本工作项.

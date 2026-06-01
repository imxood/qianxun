# TUI 性能与 Agent 开发工具优化

> 创建时间: 2026-06-01
> 状态: 方案已形成, 待拆分执行

## 目标

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

## 产出

- [阶段路线.md](阶段路线.md)

## 当前关键结论

- 当前 TUI 已接近 Codex 风格的 inline 交互, 但仍存在长输出下逐帧重算渲染行的问题.
- 当前 Daemon 是 HTTP 骨架, 还不是唯一 Agent runtime.
- Memory, MCP, Skills 的设计文档与真实代码存在不同程度偏差, 需要先治理状态标注.
- 更好的 Agent 开发工具应采用 Daemon-first: TUI/ACP/Web 作为 thin client, 统一复用 session, memory, tools, provider 和 context policy.

## 下一步

1. 先执行 `阶段路线.md` 中 Phase A 和 Phase B.
2. Phase B 完成后补充 TUI 性能验收记录.
3. 稳定结论迁入正式事实源, 阶段性过程继续留在本工作项.

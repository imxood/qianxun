# qx 交互式 TUI 调研

> 创建时间: 2026-06-01 | 收尾: 2026-06-03
> 状态: ✅ P0 完成, v2 留作 TUI 性能路线 A-G 一部分, 收尾

## 目标

- 学习 `ratatui 0.30` 的基本用法, 形成面向本项目的学习文档.
- 分析 `qx` 当前 CLI/TUI 代码, 梳理交互式能力缺口.
- 对每一个交互效果给出是否支持的建议和实现方式.

## 范围

- 覆盖独立 CLI 模式下的交互式 TUI.
- 不覆盖 ACP 协议客户端 UI, VPS Server Web UI, Daemon HTTP API 的协议设计.
- 不在本工作项中直接改造 TUI 代码.

## 产出

- [ratatui学习文档.md](ratatui学习文档.md) ✅
- [qx交互效果分析.md](qx交互效果分析.md) ✅
- [记录.md](记录.md) ✅

## 当前关键结论 (P0 已落地, 2026-06-03)

- 当前 `qx` 独立 CLI 入口实际调用 `qianxun/src/tui/mod.rs` (约 1730 行 ratatui + Inline Viewport + 脏标记渲染). 旧的 `qianxun/src/cli/cli.rs` 保存了较完整的 REPL 能力, 但当前不可达, `cargo check` 也显示它的大量代码未使用.
- 项目已引入 `ratatui 0.30.0` + `crossterm 0.28.1`, 依赖统一.
- 关键 commit: `f724653 refactor(tui): Inline Viewport + 回滚缓冲 + 消息队列` / `8f613ec feat: 完成 TUI 交互层重构` / `28bb68a perf: 脏标记驱动渲染 + 增量行缓存 + 帧率限制 + 工具折叠 + 基准测试` / `e40b8e1 fix: live_message_lines cached_lines 越界 panic + 初始消息缓存同步`
- 4 项关键能力落地: 脏标记驱动渲染 + 增量行缓存 + 帧率限制 + 工具折叠, 详见 `docs/10_事实源/` (待 v2 阶段迁入)

## 完成情况 (2026-06-03)

- ✅ 学习 ratatui 0.30 基础用法 (产出 `ratatui学习文档.md`)
- ✅ 分析 qx 当前 CLI/TUI 代码 (产出 `qx交互效果分析.md`, 梳理交互式能力缺口)
- ✅ 对 12 个交互效果给出建议和实现方式 (产出 `记录.md`)
- ✅ P0 首轮实现 (独立 ratatui 路径, commit `f724653` + `8f613ec` + `28bb68a` + `e40b8e1`)

## 后续 v2 留作 TUI 性能路线 A-G 一部分

P0 完成, v2 路线 (会话列表 / 历史输入 / 多行输入 / 权限审批 / 滚动体验 / 大树优化 / 渲染缓存) 合并到工作项 `2026-06-01_TUI性能与Agent开发工具优化/` (本次归档到 `90_历史/`), 留作未来单机性能优化参考.

## 对应 plans 决策

调研型工作项, 无对应 mavis plan. TUI 重构在 session 内手工完成, 关键时间窗 2026-05-31 ~ 2026-06-01. 后续 TUI 相关工作 (跟 Kanban 视图相关) 走 mavis 编排, 见 `06-mavis-执行历史.md` §2 阶段总表 + §6 工作项对应表.

## 关联文档

- `docs/30_子项目规划/06-mavis-执行历史.md` — mavis 编排执行历史
- `docs/30_子项目规划/04-kanban-design.md` — 多 Agent Kanban 架构 (含 TUI Kanban 视图 MVP-4)
- `docs/10_事实源/` — TUI 稳定事实源 (待 v2 阶段迁入)
- `docs/90_历史/2026-06-01_TUI性能与Agent开发工具优化_未执行/` — TUI 性能路线 A-G 归档

# qx 交互式 TUI 调研

> 创建时间: 2026-06-01
> 状态: P0 首轮实现完成, 待交互试用反馈

## 目标

- 学习 `ratatui 0.30` 的基本用法, 形成面向本项目的学习文档.
- 分析 `qx` 当前 CLI/TUI 代码, 梳理交互式能力缺口.
- 对每一个交互效果给出是否支持的建议和实现方式.

## 范围

- 覆盖独立 CLI 模式下的交互式 TUI.
- 不覆盖 ACP 协议客户端 UI, VPS Server Web UI, Daemon HTTP API 的协议设计.
- 不在本工作项中直接改造 TUI 代码.

## 产出

- [ratatui学习文档.md](ratatui学习文档.md)
- [qx交互效果分析.md](qx交互效果分析.md)
- [记录.md](记录.md)

## 当前关键结论

- 当前 `qx` 独立 CLI 入口实际调用 `qianxun/src/tui/mod.rs`, 这是一个 raw mode + ANSI 手写命令弹窗原型.
- 旧的 `qianxun/src/cli/cli.rs` 保存了较完整的 REPL 能力, 但当前不可达, `cargo check` 也显示它的大量代码未使用.
- 项目已引入 `ratatui 0.30.0`, 但依赖树同时存在 `crossterm 0.28.1` 和 `0.29.0`; 后续 TUI 改造前应统一事件和 backend 版本.

## 下一步

1. 试用当前 `qx` TUI, 验证输入, 命令面板, 模式切换, 流式回复和工具调用展示.
2. 第二轮补齐会话列表, 历史输入, 多行输入, 权限审批和更完整的滚动体验.
3. 稳定后将 TUI 架构事实迁入 `docs/10_事实源/`.

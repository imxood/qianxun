---
状态: 生效
适用范围: qianxun/src/tui/mod.rs
最后更新: 2026-06-01
---

# TUI 架构状态

## 一句话摘要
基于 ratatui 的完整 TUI，含对话视图、命令弹窗、流式输出、脏标记驱动的渲染引擎。

## 源文件
-  — 单文件，约 1730 行

## 技术栈
- ratatui 0.30（modular：core + widgets + macros）
- crossterm 事件处理
- tui-input 多行输入
- tokio mpsc 通道（Agent 事件流）
- Viewport::Inline 渲染（无 alternate screen）

## 当前状态

| 特性 | 状态 | 说明 |
|------|------|------|
| 对话历史视图 | ✅ | 角色着色、自动滚动、Wrap |
| 命令弹窗 | ✅ | / 触发、实时过滤、方向键导航、编号 1-9 |
| 多行输入 | ✅ | tui-input，Shift+Enter 换行，Emacs 快捷键 |
| 流式输出 | ✅ | mpsc 通道 + 逐行提交 |
| 脏标记渲染 | ✅ | 无变化时不 draw，空闲 poll(200ms) |
| 增量行缓存 | ✅ | cached_lines 每消息缓存，流式 delta 只更新单条 |
| Live 区限制 | ✅ | 仅传递可见行给 Paragraph |
| 工具输出折叠 | ✅ | >10 行工具消息截断 + 文件写入 |
| 分块 scrollback | ✅ | insert_before 每块 ≤200 行 |
| 帧率限制 | ✅ | 最大 ~30fps |
| Agent 打断 | ✅ | Esc/Ctrl+C |
| 消息排队 | ✅ | 生成时 Enter 排队 |
| 确认弹窗 | ✅ | 退出/重置确认 |
| Markdown 渲染 | 🔧 | 未实现 |
| 性能基准 | ✅ | 10KB/100KB/1MB 帧耗时测试 |

## 性能数据（TestBackend 80×24）
| 大小 | 帧耗时 |
|------|--------|
| 10KB | ~447µs |
| 100KB | ~451µs |
| 1MB | ~447µs |

## 已知缺口
- 无 Markdown 语法高亮
- 无 Diff 渲染
- 无 @ 文件搜索
- 单文件结构，可考虑按组件拆分

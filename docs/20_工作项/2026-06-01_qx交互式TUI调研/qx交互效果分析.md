# qx 交互效果分析

> 调研日期: 2026-06-01

## 1. 结论

`qx` 需要支持一个真正的交互式 TUI, 不是只支持命令补全弹窗. 推荐支持范围:

- P0 必须支持: 主布局, 输入编辑, 命令面板, 消息发送, 流式输出, 工具调用展示, 状态栏, 模式切换, 取消, 确认弹窗, resize.
- P1 应支持: 会话列表, help/skills/tools/memory/workspace 面板, token/context 指示, 历史滚动, retry/edit.
- P2 可后置: URL 自动抓取提示, 文件引用预览, 右侧调试详情面板, 主题配置.
- N 暂不支持: 在独立 TUI 中复刻 ACP 编辑器交互, 复杂鼠标操作, 多会话并排.

## 2. 当前代码事实

- `qianxun/src/main.rs` 在独立 CLI 模式调用 `cli::run::run_repl().await`.
- `qianxun/src/cli/run.rs` 当前直接调用 `crate::tui::run().await`.
- `qianxun/src/tui/mod.rs` 当前只实现 raw mode 输入和斜杠命令弹窗, 不连接 AgentLoop.
- `qianxun/src/cli/cli.rs` 保留了旧 REPL 的完整能力, 包括 `/help`, `/reset`, `/usage`, `/skills`, `/tools`, `/memory`, `/sessions`, `/retry`, `/edit`, `/mode`, `/plan`, 消息处理, 会话保存和 Ctrl+C 取消, 但当前不可达.
- `qianxun-core/src/output.rs` 已有 `OutputSink` trait, 是 TUI 接入 agent 流式输出的正确边界.
- `qianxun-core/src/agent/engine.rs` 已通过 `OutputSink` 输出文本, thinking, 工具调用, token, 状态, 错误和 turn finished.

## 3. 效果支持矩阵

| 效果 | 是否支持 | 优先级 | 实现方式 |
|---|---:|---:|---|
| 全屏主布局 | 支持 | P0 | `ratatui::init/restore` + `Layout::vertical`, 分为 header/body/input/status. |
| 启动横幅 | 支持 | P1 | header 区用 `Paragraph` 或 `Block`, 显示模型, 模式, 技能数, MCP 数. 不需要每轮重复打印. |
| 消息滚动区 | 支持 | P0 | body 区维护 `Vec<MessageView>`, 使用 `Paragraph` 或分段渲染, 加 `ScrollbarState`. |
| 输入框基本编辑 | 支持 | P0 | 使用 `tui-input`, 支持插入, 删除, 左右移动, Home/End, Unicode 宽度. |
| 多行输入 | 支持 | P0 | `Enter` 发送, `Alt+Enter` 或 `Shift+Enter` 换行; 输入区高度按内容在 1-6 行内自适应. |
| 历史输入上下翻 | 支持 | P1 | 维护 `VecDeque<String>` 和 history index, Up/Down 只在命令面板关闭且输入为空/特定状态时触发. |
| 斜杠命令面板 | 支持 | P0 | 输入以 `/` 开头时显示居于输入框上方的 popup, `List + ListState + Clear`, 支持过滤, 上下选择, Tab/Enter 补全. |
| 命令执行 | 支持 | P0 | 把旧 `handle_slash` 能力迁移为纯命令处理函数, TUI 只负责收集命令和展示结果. |
| `/mode` 和 `/plan` 模式切换 | 支持 | P0 | `AppState.mode: Mode`, 状态栏常显; 执行消息前用 `mode.tool_filter()`. 切换时重建 system prompt 或明确下一轮生效. |
| 用户消息提交 | 支持 | P0 | Enter 后将输入转为 `UserMessage`, 清空输入, spawn agent task, UI 状态进入 `Running`. |
| LLM 流式文本 | 支持 | P0 | 新增 `TuiOutputSink`, `on_text` 发送 `TextDelta`, UI 将 delta 追加到当前 assistant 消息并重绘. |
| thinking 展示 | 支持但默认折叠 | P1 | `on_thinking` 累积, `on_thinking_flush` 生成可折叠消息块; 默认只显示摘要和长度. |
| 工具调用卡片 | 支持 | P0 | `on_tool_call` 生成 `ToolCallView`, 显示工具名, id, 参数摘要; 后续 status 更新执行中/完成/失败. |
| 工具结果详情 | 支持摘要 | P1 | 当前引擎未通过 `OutputSink` 暴露完整 tool result; 可先显示状态和错误, 后续扩展 `OutputSink::on_tool_result`. |
| 状态栏和进度提示 | 支持 | P0 | `on_status`, token, agent state 更新 status 区; 简单 spinner 可用 tick 事件驱动. |
| Ctrl+C 取消生成 | 支持 | P0 | 保留 `cancel_flag`; TUI key/signal 事件触发 `cancel_flag.store(true)`, 当前消息标记 Cancelled. |
| Esc 行为 | 支持 | P0 | 有弹窗先关闭弹窗; 无弹窗且输入为空则可请求退出确认; 有输入则清空输入或退出编辑态. |
| 退出确认和终端恢复 | 支持 | P0 | `/quit`, Ctrl+D 或确认退出后保存会话, shutdown tools, 调用 restore. panic hook 也应恢复终端. |
| reset 确认弹窗 | 支持 | P0 | 用 modal 呈现 `y/N`, 不再阻塞 `stdin.read_line`; modal 状态截获键盘事件. |
| 权限/审批弹窗 | 支持 | P0 | 写文件, 删除, 执行命令, 网络等高风险工具需要审批; 当前 core 只有模式过滤, 后续应在工具执行前暴露 permission request. |
| `/help` 面板 | 支持 | P1 | help 作为 overlay 或 body 临时 panel, 使用 `Paragraph` + 滚动. |
| `/skills` 面板 | 支持 | P1 | `List/Table`, 触发 `check_skill_reload`, 显示自动/手动注入说明和技能列表. |
| `/tools` 面板 | 支持 | P1 | `Table`, 按 builtin/MCP/category 分组, 可显示当前模式下是否可用. |
| `/memory` 面板 | 支持 | P1 | 异步读取 memory context, 加载中状态, 结果进入 overlay. |
| `/workspace` 面板 | 支持 | P1 | 展示 workspace root, prompt 文件, 上下文摘要; 长内容可滚动. |
| `/usage` token 指示 | 支持 | P1 | 状态栏常显简要 token, `/usage` 打开详情; context zone 可用颜色区分 safe/warning/danger. |
| `/sessions` 列表/恢复/删除 | 支持 | P1 | `Table + TableState`; Enter 恢复, Delete 删除需确认; 保留模糊匹配命令入口. |
| `/retry` | 支持 | P1 | 使用 `last_message` 重发; UI 显示“重试上一条”系统事件. |
| `/edit` | 支持 | P1 | 将上一条消息填回输入框进入编辑态, 用户确认后重新发送. 不建议另起阻塞 read_line. |
| `$FILE:path` 展开提示 | 支持 | P2 | 发送前解析并在输入区上方显示 chips/preview; 展开失败显示 inline error. |
| URL 自动抓取提示 | 支持 | P2 | 发送前检测 URL, 后台抓取, status 显示加载; 默认可关闭, 避免输入提交被网络阻塞. |
| resize 自适应 | 支持 | P0 | 处理 `Event::Resize`, 重新计算 layout; 小宽度下隐藏 header 次要信息. |
| 鼠标滚动 | 可后置支持 | P2 | `EnableMouseCapture` 后处理 scroll, 初期可只支持键盘 PgUp/PgDn. |
| 主题配置 | 可后置支持 | P2 | 先内置一套低对比但清晰的色彩 token; 后续配置化. |
| ACP 编辑器交互复刻 | 暂不支持 | N | ACP 是协议模式, 独立 TUI 不应复刻编辑器内 UI. |
| 多会话并排 | 暂不支持 | N | 当前会话模型和终端空间不适合, 先支持单会话切换. |

## 4. 推荐的最小闭环

P0 实现顺序:

1. 依赖统一: 使用 `ratatui::crossterm` 或升级 workspace `crossterm`.
2. 建立状态模型:

```rust
struct AppState {
    mode: Mode,
    running: bool,
    agent_running: bool,
    input: tui_input::Input,
    messages: Vec<MessageView>,
    command_palette: CommandPaletteState,
    modal: Option<ModalState>,
    status: StatusLine,
}
```

3. 建立事件模型:

```rust
enum UiEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
    Agent(AgentUiEvent),
}
```

4. 建立 `TuiOutputSink`, 把 `OutputSink` 事件转换成 `AgentUiEvent`.
5. `render(frame, &mut AppState)` 完整绘制 header/body/input/status/popup/modal.
6. 接入旧 REPL 的命令处理和 `processing_loop::handle_user_message`.

## 5. 不建议的路线

- 不建议继续在 `tui/mod.rs` 中扩展 ANSI 手写局部刷新. 该方式无法稳定解决流式输出, resize, 弹窗层级, 滚动和异步事件.
- 不建议直接复活旧 `cli::cli::Repl` 的 `read_line` 循环. 它会阻塞输入, 也无法在 agent 输出期间保持输入框和状态栏稳定.
- 不建议一开始做复杂多面板 IDE 式界面. 先完成单会话 TUI 主链路, 再加 P1/P2 面板.

## 6. 验证要求

实现时至少需要:

- `cargo check -p qianxun`
- 命令面板过滤/选择单元测试.
- `ratatui::backend::TestBackend` 渲染测试:
  - 80x24 正常布局.
  - 40x12 小窗口布局.
  - 工具调用卡片.
  - modal 覆盖.
  - 输入框多行.

本次调研运行 `cargo check -p qianxun` 时, 编译最终因写入 `target/debug/deps/libqx-*.rmeta` 被拒绝访问而失败; 在失败前已暴露大量未使用警告, 其中包括旧 `cli::cli::Repl` 不可达和 `/plan` 分支存在 unreachable pattern.

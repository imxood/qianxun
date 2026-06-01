# ratatui 学习文档

> 调研日期: 2026-06-01
> 目标版本: `ratatui 0.30.0`

## 1. 定位

`ratatui` 是 Rust 终端 UI 库. 它负责屏幕绘制, 布局, 文本样式和 widget 渲染, 但不内置输入事件模型. 输入通常由 `crossterm::event` 提供, 应用自己维护状态并在每一帧完整重绘 UI.

官方资料入口:

- API 文档: https://docs.rs/ratatui
- 官方网站: https://ratatui.rs/
- 事件处理概念: https://ratatui.rs/concepts/event-handling/
- 布局资料: https://ratatui.rs/recipes/layout/
- widget 资料: https://ratatui.rs/recipes/widgets/

## 2. 本项目依赖现状

当前 workspace 使用:

```toml
ratatui = { version = "0.30", default-features = false, features = ["crossterm"] }
crossterm = "0.28"
tui-input = "0.15"
```

`Cargo.lock` 显示:

- `ratatui 0.30.0`
- `ratatui-crossterm 0.1.0`
- `crossterm 0.28.1`
- `crossterm 0.29.0`

这意味着项目直接依赖的 `crossterm` 和 `ratatui-crossterm` 选中的 `crossterm` 版本并不一致. 后续实现时建议二选一:

- 优先方案: 使用 `ratatui::crossterm` re-export, 让事件和 backend 使用同一个 `crossterm` 版本.
- 备选方案: 将 workspace 的 `crossterm` 升到与 `ratatui-crossterm` 一致的版本.

## 3. 最小应用结构

`ratatui 0.30` 推荐的基础结构是:

```rust
fn main() -> std::io::Result<()> {
    ratatui::run(|mut terminal| {
        loop {
            terminal.draw(render)?;
            if should_quit()? {
                break Ok(());
            }
        }
    })
}

fn render(frame: &mut ratatui::Frame) {
    frame.render_widget("Hello", frame.area());
}
```

如果需要更细粒度控制, 可以手动 `ratatui::init()` 和 `ratatui::restore()`. 对 `qx` 更建议手动控制, 因为需要异步 LLM 流, Ctrl+C 取消, 日志和退出保存.

关键规则:

- 终端初始化和恢复必须成对出现.
- 主循环每次 draw 都完整绘制当前 UI, 不依赖局部 ANSI 清屏.
- UI 状态放在 `AppState`, render 函数只读状态.
- 输入事件改变状态, 异步 agent 事件也改变状态, 然后触发重绘.

## 4. Terminal, Frame, Buffer

核心链路:

```text
Terminal::draw
  -> Frame
  -> render_widget / render_stateful_widget
  -> Buffer diff
  -> backend flush 到终端
```

对 `qx` 的意义:

- 不再手写 `\x1b[A\x1b[K` 清除命令弹窗.
- 命令弹窗, 状态栏, 输出区都作为 widget 按区域重绘.
- 光标位置由 `frame.set_cursor_position` 或相关 API 控制, 避免输入框和输出流互相覆盖.

## 5. 布局

`Layout` 用 `Constraint` 划分区域. 常用约束:

- `Length(n)`: 固定高度或宽度.
- `Min(n)`: 至少占用.
- `Fill(1)`: 剩余空间.
- `Percentage(n)`: 百分比.

`qx` 建议基础布局:

```rust
use ratatui::layout::{Constraint, Layout};

let [header, body, input, status] = Layout::vertical([
    Constraint::Length(3),
    Constraint::Fill(1),
    Constraint::Length(input_height),
    Constraint::Length(1),
]).areas(frame.area());
```

在 `body` 内可再拆:

- 默认: 一个消息滚动区.
- 调试模式: 左侧消息, 右侧状态/工具/记忆面板.

## 6. 常用 Widget

适合 `qx` 的内置 widget:

- `Paragraph`: 消息文本, 输入框, 错误详情, 帮助内容.
- `Block`: 边框, 标题, 分组.
- `List` + `ListState`: 命令面板, 会话列表, 工具列表.
- `Table`: `/sessions`, `/tools` 的结构化列表.
- `Tabs`: 可选的详情视图切换, 如 `消息 / 工具 / 记忆`.
- `Gauge` 或 `LineGauge`: token/context 使用率.
- `Scrollbar`: 长消息和历史滚动.
- `Clear`: 渲染弹窗前清除底层区域.

实现原则:

- 简单文本用 `Paragraph`.
- 需要选中态的列表用 `StatefulWidget`.
- 弹窗使用 `Clear + Block + List/Paragraph`.
- 不为每种消息创建复杂自定义 widget, 先用统一 `MessageView` 数据结构渲染.

## 7. 输入事件

`ratatui` 不处理键盘, `crossterm` 负责事件. 基础模式:

```rust
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

match event::read()? {
    Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
        KeyCode::Enter => {}
        KeyCode::Esc => {}
        KeyCode::Char(c) => {}
        _ => {}
    },
    Event::Resize(_, _) => {}
    _ => {}
}
```

`qx` 不应在 agent 流式生成时阻塞在 `event::read()`. 建议把事件流拆成两个通道:

- 终端输入任务: 读取键盘/resize/tick, 发送 `UiEvent`.
- agent 输出任务: 把 `OutputSink` 事件发送到同一个 UI channel.
- 主 UI loop: `tokio::select!` 接收事件, 更新 `AppState`, draw.

## 8. 文本输入

项目已引入 `tui-input 0.15`, 适合先替代手写 `String`:

- 支持光标移动, 删除, 插入.
- 降低 Unicode 宽度和编辑边界错误.
- 可配合 `Paragraph` 渲染输入内容.

`qx` 输入框建议状态:

```rust
struct InputState {
    input: tui_input::Input,
    mode: InputMode,
    history_index: Option<usize>,
    multiline: bool,
}
```

## 9. 异步输出和 OutputSink

当前核心引擎通过 `qianxun_core::output::OutputSink` 输出:

- `on_text`
- `on_thinking`
- `on_tool_call`
- `on_token_usage`
- `on_error`
- `on_turn_finished`
- `on_status`

TUI 改造时不应让 `OutputSink` 直接打印终端. 应实现一个 `TuiOutputSink`, 它只把事件发送到 UI:

```rust
enum AgentUiEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ThinkingFlush,
    ToolCall { id: String, name: String, args: serde_json::Value },
    TokenUsage(TokenUsage),
    Status(String),
    Error(String),
    TurnFinished(StopReason),
}
```

这样 UI 可以安全地把 LLM 流, 工具事件和用户输入绘制到同一套布局里.

## 10. 测试方式

Ratatui 支持用 `TestBackend` 做快照式验证:

- 构造固定宽高的终端.
- 渲染某个 `AppState`.
- 断言 buffer 内容或使用 insta 快照.

建议最小测试覆盖:

- 命令面板过滤和选中态.
- 输入框光标和多行高度.
- 工具调用卡片渲染.
- 错误状态和取消状态.
- 小宽度窗口下文本不越界.

## 11. qx 采用 ratatui 的建议

优先路线:

1. 统一 `crossterm` 版本或改用 `ratatui::crossterm`.
2. 建立 `AppState`, `UiEvent`, `render(frame, &state)`.
3. 用 `tui-input` 承接输入框.
4. 用 `TuiOutputSink` 替代 `CliOutputSink` 的直接 `print/eprint`.
5. 先完成 P0 效果, 再补会话列表, 面板, token gauge 等 P1 效果.

不建议继续扩展当前 `qianxun/src/tui/mod.rs` 的手写 ANSI 方案. 原因是它只能局部清行, 无法稳定处理流式输出, resize, 滚动区, 弹窗层叠和异步事件.

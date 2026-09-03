//! 终端域（M4）：portable-pty 多会话宿主。
//!
//! 每个标签 = 一个 PTY 会话：reader 线程把输出推给前端
//! （`terminal://output`，按 id 过滤），退出推 `terminal://exit`。
//! 标签切换不销毁会话——前端保活 xterm 实例，这里保活 PTY。

pub mod commands;

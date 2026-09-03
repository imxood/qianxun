//! 笔记域（M5）：Markdown 笔记库。
//!
//! 纯文件存储（ADR-006）：一个目录 = 一个库，`.md` 文件 + YAML frontmatter
//! （title/tags）。所有写入走原子替换；不做隐藏数据库。

pub mod commands;

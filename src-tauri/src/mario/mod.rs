//! 马里奥实验室的进化状态(docs/13):数据根 `mario/` 目录的读写 IPC。
//! webview 无文件能力,UI 经此存取 champion 策略/手册/审计日志;
//! 无头闭环(e2e/mario-loop.ts)直接 fs 操作同一目录,共享 champion。

pub mod commands;

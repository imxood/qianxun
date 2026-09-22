//! 工具管理域（R001）：Moli 无头浏览器的检测与安装。
//!
//! 三源检测（优先级从高到低）：settings 自备路径 → 数据目录受管安装
//! （`<data>/tools/moli/`）→ 系统 PATH。检测即真实拉起 `--version`
//! 探测，不做任何存在性猜测。

pub mod moli;

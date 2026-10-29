//! 情报域（docs/17）：主题注册表 + 每日抓取 + 按天落盘 + 收藏/已读 + Agent 出刊。
//!
//! 形态约定：
//! - 启动补抓（当天一次）+ 日期哨兵跨天自动补抓 + `intel_refresh` 手动刷新；
//! - 数据按天按主题一文件（`days/{date}/{topic}.json`），读视图 = 读文件；
//! - 条目原始字段永不改写（来源 URL / 原文标题常驻），刷新只更新热度；
//! - 写入一律 `atomic::write`，进程被杀不产生半截文件。

pub mod commands;
pub mod digest;
pub mod extract;
pub mod fetch;
pub mod llm;
pub mod prompts;
pub mod scheduler;
pub mod store;
pub mod topics;
pub mod types;

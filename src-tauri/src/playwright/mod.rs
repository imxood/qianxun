//! 内置 playwright 工具（browser use）：把官方 @playwright/mcp 作为
//! stdio MCP server 注入千寻所辖 DSH 的 web profile。
//!
//! 部署形态（与 websearch/bridge 同一套模型：宿主托管、宿主写 patch）：
//! - vendor 三件套（@playwright/mcp + playwright + playwright-core，sync
//!   脚本随包同步）拷贝到 `<profile>/node_modules/`——免 npx 运行时下载；
//! - patch 追加 `dsh-mcp-client` 条目：`serverName: playwright`，stdio
//!   拉起 cli.js；Agent 获得 `mcp__playwright__*` 全套浏览器工具；
//! - 浏览器选择（与 browser_* 时代同一策略）：环境页 moli 三源解析命中
//!   且文件存在 → `--executable-path`；否则 Windows → `--browser msedge`；
//! - 登录态持久化：`--user-data-dir` 指向千寻数据目录，登录一次后续免登。

pub mod commands;

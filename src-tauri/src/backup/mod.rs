//! 数据备份域：把「千寻设置 + DSH 全部用户数据」打包成一个 zip，
//! 或从这样的包里还原（目标场景：同一台电脑上的还原）。
//!
//! 备份范围（数据根 `~/.qianxun` 相对路径，dev 版为 `~/.qianxun_dev`）：
//! - `settings.json`：千寻自身设置；
//! - `dsh-home/**`：DSH 用户数据——`settings.yaml`（agents/providers/模型
//!   配置、agent-default-model、权限等）、`.credentials.yaml`（agents 的
//!   API Key）、`storages/`（工作区注册表 + 会话投影缓存）、`sessions/`
//!   （会话记录）、`attachments/`、`mobile-access/`、`profiles/` 的插件清单。
//!
//! 排除（可重建，避免包体虚胖）：`node_modules`、`logs`、`*.tmp`。
//! DSH 程序本体（dsh-runtime）、自管 Node、pnpm 工具都不进包——还原后
//! 沿用现有安装。
//!
//! 安全设计：
//! - 包顶层 `manifest.json` 记录 kind/schemaVersion/统计，还原前强校验；
//! - 还原前把当前状态打成 `backups/pre-restore-<时间戳>.zip`（仅保留
//!   最近一份），失败可手动放回；
//! - 还原走「解压到临时目录 → 旧数据改名让位 → 新数据就位」，避免把
//!   数据根长时间留在半还原状态；
//! - 包内条目白名单：只接受 `manifest.json`、`settings.json`、
//!   `dsh-home/**`，其余一律拒绝（不信任来源不明的备份包）。
//!
//! ⚠ 备份包含 API Key 明文，用户须自行妥善保管备份文件。

pub mod commands;

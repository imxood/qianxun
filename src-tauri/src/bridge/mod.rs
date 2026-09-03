//! DSH 插件桥（M6）：把千寻笔记能力以 `qx-bridge` 插件注入
//! 千寻所辖 DSH 的 web profile。
//!
//! 部署形态（零构建、零 junction）：
//! - 插件源内嵌于二进制（`assets/`，include_str!），落盘到
//!   `<DSH_HOME>/profiles/web/node_modules/qx-bridge/`——node 就地解析；
//! - profile 的 `cordis.patch.yml` 追加根条目 `{insert: [{id: qx-bridge,
//!   name: qx-bridge, config: {vault}}]}`；
//! - 重启 DSH 生效。DSH 重装（pnpm 清理 node_modules）后需重新部署，
//!   状态命令可见、外壳启动时自愈。

pub mod commands;

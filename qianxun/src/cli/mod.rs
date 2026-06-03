// cli/cli.rs: 同名嵌套 mod 是历史包袱 (旧 REPL 整块), 留 Phase 4 拆分后改名.
#![allow(clippy::module_inception)]

pub mod cli;
pub mod config;
pub mod output;
pub mod run;
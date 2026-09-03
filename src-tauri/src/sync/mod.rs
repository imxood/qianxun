//! 同步域（S1，第一阶段）：笔记库走 git。
//!
//! 范围（ADR-013）：只同步 vault（git 仓）；截图目录、settings 白名单
//! 属第二阶段；`.credentials.yaml`、sessions **永不**同步。
//! 千寻不内嵌 git 实现——调用系统 git（存在性探测，缺了给出指引）。

pub mod commands;

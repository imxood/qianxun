// health domain — 本地 mock + 远程 daemon health 探活
//
//   check.rs:  health_check (本地, 返 connected)
//   fetch.rs:   daemon_health_fetch (远程, 3s 超时, 失败返 offline)
//   types.rs:   DaemonState (4 态) + HealthStatus (7 字段)
//   mock.rs:    offline_status() helper (网络错误/解析失败降级用)
//
// 跟 qianxun-desktop/src/lib/types/ipc.ts §4.1.2 完全对齐.
// 跟 docs/30_子项目规划/03-tauri-desktop.md §4.1.2 / §10.1 完全统一.
// 不接 RuntimeState (Stage 2 最小集, 真状态机留 4a 后续).
//
// 注: tauri::command macro 生成的 __cmd__xxx 辅助项不会通过 `pub use` 传递,
// 所以 health_check / daemon_health_fetch 不在 mod.rs re-export,
// lib.rs::generate_handler! 用完整路径 `commands::health::check::health_check`.

pub mod check;
pub mod fetch;
mod mock;
mod types;
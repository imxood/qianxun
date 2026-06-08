// commands — 4 domain 平行
//
//   health:    本地 mock + 远程 daemon health 探活 (Stage 2, 不接 RuntimeState)
//   stronghold: iota_stronghold 凭据加密 vault (Argon2 + ChaCha20, §11.3)
//   runtime:   5 个真 command 走 RuntimeApi (sub-task #3, 收口 daemon HTTP + Tauri command)
//   events:    见同级目录 (emit 事件 schema 收口)
//
// 每个 domain 单独子目录 + mod.rs 收口, lib.rs invoke_handler 注册时
// 直接 `commands::health::check::health_check` 即可, 不暴露 internal 文件路径.
//
// runtime 必须 pub, 因为 lib.rs invoke_handler 用 `commands::runtime::xxx::yyy` 完整路径
// (tauri::command macro 不能 pub use 跨文件传递, sub-task #2 已踩).

pub mod health;
pub mod runtime;
pub mod stronghold;

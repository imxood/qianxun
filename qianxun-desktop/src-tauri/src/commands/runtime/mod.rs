// qianxun-desktop/src-tauri/src/commands/runtime/mod.rs
// runtime domain — 5 个真 command, 通过 RuntimeApi 调 qianxun-runtime 业务.
//
// 5 个文件 (1:1 对应 RuntimeApi trait 5 个方法):
//   - sessions.rs  list_sessions    → list_sessions_impl (RuntimeApi)
//   - send.rs      send_message     → send_message_impl + emit events
//   - plans.rs     create_plan      → create_plan_impl
//   - cancel.rs    cancel_session   → cancel_session_impl
//   - load.rs      load_session     → load_session_impl
//
// 设计:
//   - 业务 100% 在 qianxun-runtime, 这边只是 thin adapter (参数 + 返回 + emit event)
//   - Tauri State<Arc<RuntimeState>> 注入, 直接当 RuntimeApi trait 用
//   - send_message 不阻塞 command 返回, 起 spawn task 消费 mpsc 通道
//   - 错误统一成 String 返给前端 (Tauri command 必须是 Serialize, RuntimeApiError 不是)
//
// 不在本 mod:
//   - Tauri command macro 必须在函数定义处, 不能 pub use 跨文件传递 (跟 sub-task #2 同坑)
//   - lib.rs invoke_handler 用 `commands::runtime::sessions::list_sessions` 完整路径

pub mod background;
pub mod cancel;
pub mod load;
pub mod plans;
pub mod send;
pub mod sessions;

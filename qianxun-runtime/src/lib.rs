// qianxun-runtime/src/lib.rs
// 千寻运行时 — 6 核心模块 (agent_host / service / persistence / session_runtime / output_sink / sse) + RuntimeState
// 复用给 qianxun binary (CLI / daemon / tui / acp / server / client) 跟 qianxun-desktop (Tauri 2.x webview)
// 跟 ADR-0003 (合并 desktop + 2-mode 互斥) 一致

pub mod agent_host;
pub mod output_sink;
pub mod persistence;
pub mod session_runtime;
pub mod sse;
pub mod state;
pub use agent_host::{AgentLoopHost, SharedState};
pub use output_sink::DaemonOutputSink;
pub use persistence::SessionStore;
pub use session_runtime::{SessionId, SessionRuntime};
pub use sse::{SseEvent, SseEventBuilder};
pub use state::RuntimeState;

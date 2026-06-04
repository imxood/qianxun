//! # Thin client for connecting to a running `qx daemon` (HTTP + SSE).
//!
//! 拆自 client/mod.rs (1213 行, 2026-06-04 Commit 15). 6 个 src + 1 个 tests:
//! - mod.rs (本文件, 43 行): 顶层 use + SseStream type alias + 6 mod + 6 pub use re-export
//! - types.rs (149 行): ClientError + 5 DTO + SseEvent enum (12 variant) + impl PromptRequest
//! - daemon_client.rs: DaemonClient struct + 6 impl DaemonClient 块 (健康/会话/prompt/取消/重连)
//! - reconnect.rs: RECONNECT_BACKOFF + ReconnectState + ReconnectTracker + ReconnectHandle + t_after
//! - sse_parser.rs: parse_sse_stream + extract_sse_frames + parse_data_payload
//! - probe.rs: detect_local_daemon (本地 daemon 探测)
//! - repl.rs: run_thin_repl + consume_sse_stream_print (REPL 私有 helper)
//! - tests/mod.rs: 12 个 test fn (跨 6 个主题: health / session / stream / reconnect / token / url)

#![allow(dead_code)]

use std::pin::Pin;

use futures::stream::Stream;

mod daemon_client;
mod probe;
mod reconnect;
mod repl;
mod sse_parser;
mod types;

pub use daemon_client::DaemonClient;
pub use probe::detect_local_daemon;
pub use reconnect::{
    next_backoff, ReconnectHandle, ReconnectState, ReconnectTracker, RECONNECT_BACKOFF,
};
pub use repl::run_thin_repl;
pub use sse_parser::{extract_sse_frames, parse_data_payload, parse_sse_stream};
pub use types::{
    ClientError, HealthStatus, PromptMessage, PromptRequest, Session, SessionCreated,
    SessionsList, SseEvent,
};

/// Boxed stream of `Result<SseEvent, ClientError>`. 跨 `await` 边界持有.
pub type SseStream =
    Pin<Box<dyn Stream<Item = Result<SseEvent, ClientError>> + Send>>;

#[cfg(test)]
mod tests;

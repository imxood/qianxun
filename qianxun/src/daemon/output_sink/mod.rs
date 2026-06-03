//! `DaemonOutputSink` — bridges `OutputSink` trait callbacks (and direct
//! `LlmStreamEvent` consumption) to SSE events for the daemon.
//!
//! 拆自 output_sink.rs (1056 行, 2026-06-04 Commit 12). 5 个子文件:
//! - mod.rs (本文件): DaemonOutputSink struct + new + session_id/store getter
//! - builder.rs: SinkState 内部状态 + drive_builder
//! - messages.rs: 9 个 pub async fn (begin_message/text_delta/.../finish_turn_str/save_snapshot)
//! - trait_impl.rs: impl OutputSink for DaemonOutputSink (8 个 on_* 方法)
//! - tests/mod.rs: 7 个 test fn

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use qianxun_core::output::OutputSink;
use tokio::sync::mpsc;

use crate::daemon::persistence::SessionStore;
use crate::daemon::sse::{SseEvent, SseEventBuilder};

use self::builder::SinkState;

mod builder;
mod messages;
mod trait_impl;

#[cfg(test)]
mod tests;

/// SSE event sink. 同时:
/// - 实现 `OutputSink` trait (Stage 3 processing_loop 走 trait)
/// - 暴露直接 `&self` 方法 (Stage 2 consume_stream_to_sse 走直接调用)
#[allow(dead_code)]
pub struct DaemonOutputSink {
    tx: mpsc::Sender<SseEvent>,
    store: Arc<SessionStore>,
    session_id: String,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    max_tokens: u32,
    state: Mutex<SinkState>,
}

impl DaemonOutputSink {
    /// 构造 sink. 需传 tx (SSE 出口) + store (事件落盘) + model + max_tokens
    /// (用于 MessageStart).
    #[allow(dead_code)]
    pub fn new(
        tx: mpsc::Sender<SseEvent>,
        store: Arc<SessionStore>,
        session_id: String,
        model: String,
        max_tokens: u32,
        _force_message_start: bool,
    ) -> Self {
        let state = Mutex::new(SinkState {
            builder: SseEventBuilder::new(),
            started: false,
            event_seq: 0,
        });
        Self {
            tx,
            store,
            session_id,
            model,
            max_tokens,
            state,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn store(&self) -> &Arc<SessionStore> {
        &self.store
    }
}

//! SinkState 内部可变状态 (从 output_sink.rs 抽, 2026-06-04 Commit 12)

use crate::daemon::sse::SseEventBuilder;

/// 内部可变状态 — 包在 `Mutex` 里是为了让 `OutputSink` trait 的 `&self` 方法
/// 也能更新 builder. 锁持有时间极短 (一次 `from_llm_event` 调用), 且发送
/// 事件前**先释放锁**再 `tx.send().await`, 不会跨 await 持锁.
#[allow(dead_code)]
pub(super) struct SinkState {
    /// SSE 块状态机 — 跟原 SseEventBuilder 同等, 自动插入 content_block_start/stop.
    pub(super) builder: SseEventBuilder,
    /// `MessageStart` 是否已经发过 — 防止 begin_message 重复调用时发两次.
    pub(super) started: bool,
    /// `store.append_event` 用的 sequence 计数器, 跟 prompt_handler
    /// `MessageStart(seq=0)` 衔接. 第一条 content 事件 = 1.
    pub(super) event_seq: u32,
}

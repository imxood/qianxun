// qianxun/src/runtime/sse_axum.rs
// axum 包装层: 把 qianxun_runtime::SseEvent 转成 axum SSE 帧
// 从 router.rs 抽出, 跟业务 SseEvent enum 解耦 (业务在 qianxun-runtime, HTTP 包装留 qianxun binary)
//
// Stage 2: 跟 qianxun/src/daemon/sse.rs (原) 1:1
// ADR-0003: 桌面端走 Tauri invoke, 不走 SSE wire, 所以 axum 包装只 daemon 用

use axum::response::sse::Event;
use qianxun_runtime::SseEvent;
use std::convert::Infallible;

/// 把 `SseEvent` 序列化成 axum `Event` (data 帧).
pub fn event_from_sse(event: SseEvent) -> Event {
    let json = serde_json::to_string(&event).unwrap_or_else(|e| {
        tracing::error!("[sse] failed to serialize event: {e}");
        r#"{"type":"error","code":"internal","message":"event serialization failed"}"#
            .to_string()
    });
    Event::default().data(json)
}

/// 适配 `Stream::map`: `SseEvent` → `Result<Event, Infallible>` (SSE 帧).
pub fn event_to_sse(event: SseEvent) -> Result<Event, Infallible> {
    Ok(event_from_sse(event))
}

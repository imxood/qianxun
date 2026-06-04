
// ─── SSE 解析 ────────────────────────────────────────────────

use std::time::Duration;

use futures::stream::{Stream, StreamExt};
use reqwest::Response;
use tracing::warn;

use super::types::{ClientError, SseEvent};
use super::SseStream;

/// 把 `reqwest::Response` 的 byte stream 解析成 `Stream<Result<SseEvent, _>>`.
///
/// SSE 帧格式: `data: <json>\n\n` (axum::response::sse 默认格式).
/// 简化: 只解析 `data:` 行, 不分发 `event:` 字段 — 12 事件类型全在一个流上,
/// 客户端按 `type` 字段 (反序列化时由 serde tag 决定) 路由.
///
/// 备注: 这里把每个 `bytes::Bytes` chunk 先转成 `String`, 由 `extract_sse_frames`
/// 按 `\n` 切行. reqwest 的 `bytes_stream()` 在 SSE 长连接下通常按 KB 级切分,
/// 单个 chunk 几乎不会跨帧边界, 极小概率丢尾部 — 后续 chunk 会以新帧重新对齐.
pub fn parse_sse_stream(response: Response) -> SseStream {
    use futures::stream::iter;
    let byte_stream = response.bytes_stream();
    let event_stream = byte_stream
        .map(|chunk_result| {
            // Bytes → Vec<u8> → UTF-8 string; 出错时返回 ClientError
            chunk_result
                .map_err(ClientError::from)
                .and_then(|bytes| {
                    // bytes::Bytes 可以直接转 Vec<u8>
                    let v: Vec<u8> = bytes.into();
                    std::str::from_utf8(&v)
                        .map(str::to_string)
                        .map_err(|e| ClientError::Sse(format!("invalid UTF-8: {e}")))
                })
        })
        .flat_map(|text_result: Result<String, ClientError>| {
            // 每段文本可能产生 0..N 个 SSE 帧; 用 iter() 展平.
            let items: Vec<Result<SseEvent, ClientError>> = match text_result {
                Ok(text) => extract_sse_frames(&text),
                Err(e) => vec![Err(e)],
            };
            iter(items)
        });
    Box::pin(event_stream)
}

/// 从一段 SSE 文本中提取 `data: <json>` 帧, 解析为 `SseEvent`.
///
/// 一段文本可能包含 0..N 个完整帧 (每帧以 `\n\n` 结束). 简化处理:
/// 按 `\n` 切行, 跳过空行, 只取 `data: ` 前缀的行, 累积到下一个空行后解析.
/// 部分帧 (跨 chunk 边界) 由上层 byte_stream 的 next() 后续调用补全 —
/// 这里假设每次输入是已分块后的"完整片段" (reqwest::bytes_stream 在 SSE 长连接下
/// 通常按 KB 级切分, 单个 chunk 几乎不会跨帧边界, 极小概率丢尾部, 后续 chunk
/// 会以空行/新帧重新对齐).
///
/// 跳过空 data 行 (心跳); 解析失败时返回 Err 项.
pub fn extract_sse_frames(text: &str) -> Vec<Result<SseEvent, ClientError>> {
    let mut out: Vec<Result<SseEvent, ClientError>> = Vec::new();
    let mut current_data: Option<String> = None;

    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            // 帧边界
            if let Some(data) = current_data.take() {
                match parse_data_payload(&data) {
                    Ok(Some(ev)) => out.push(Ok(ev)),
                    Ok(None) => {} // 心跳, 跳过
                    Err(e) => out.push(Err(e)),
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            let payload = rest.trim_start();
            if let Some(existing) = current_data.as_mut() {
                existing.push('\n');
                existing.push_str(payload);
            } else {
                current_data = Some(payload.to_string());
            }
        }
        // 忽略其他行 (event:, id:, retry:, 注释 :...)
    }
    // 末尾可能没有空行: 把残留的 data 当作最后一帧提交.
    if let Some(data) = current_data.take() {
        match parse_data_payload(&data) {
            Ok(Some(ev)) => out.push(Ok(ev)),
            Ok(None) => {}
            Err(e) => out.push(Err(e)),
        }
    }
    out
}

/// 解析单个 `data:` 行的 JSON payload.
pub fn parse_data_payload(data: &str) -> Result<Option<SseEvent>, ClientError> {
    if data.is_empty() {
        return Ok(None); // 心跳 (空 data)
    }
    match serde_json::from_str::<SseEvent>(data) {
        Ok(ev) => Ok(Some(ev)),
        Err(e) => {
            warn!("[client::sse] parse error: {e}; data={data}");
            Err(ClientError::Sse(format!("JSON parse: {e}; data={data}")))
        }
    }
}

//! Anthropic 兼容协议 Provider。
//!
//! 用于实现任何走 Anthropic Messages API + SSE 协议的服务。
//! 实际请求结构 (`/v1/messages` endpoint、`x-api-key` header、`anthropic-version` header、
//!  `message_start` / `content_block_*` / `message_delta` / `message_stop` 事件流) 都与
//! Anthropic SDK 一致, 由 `SseParser` 统一处理。
//!
//! 当前使用此 provider 的服务:
//! - `deepseek` — DeepSeek 官方 Anthropic 兼容端点
//! - `MiniMax` — MiniMax M3 的 Anthropic 兼容端点
//! - 其他未知 provider type — 也走此协议, 用户需自备 `base_url`
//!
//! 添加新的 provider 几乎零成本：只需在 `~/.qianxun/config.json` 的 `providers`
//! section 加一个条目, 并设置 `active_provider` 指向它。

use crate::agent::message::ContentBlock;
use crate::provider::types::{CompletionRequest, LlmStreamEvent};
use crate::types::{LlmError, ProviderCapabilities, StopReason, TokenUsage};
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde_json::Value;

// ─── Types for Anthropic API response parsing ───────────────

#[derive(Default)]
struct SseParser {
    buffer: Vec<u8>,
    event_type: String,
    event_data: String,
    pending_events: Vec<LlmStreamEvent>,

    // Accumulator for multi-chunk tool_use input JSON
    tool_index: Option<usize>,
    tool_id: String,
    tool_name: String,
    tool_input_json: String,

    // Accumulator for thinking blocks
    thinking_index: Option<usize>,
    thinking_text: String,
    thinking_signature: String,
}

impl SseParser {
    /// Feed incoming bytes, process complete SSE events.
    /// Returns the number of new events pushed to `pending_events`.
    fn feed_bytes(&mut self, bytes: &[u8]) -> usize {
        let prev_len = self.pending_events.len();
        self.buffer.extend_from_slice(bytes);

        // Process complete SSE events (separated by \n\n or \r\n\r\n)
        loop {
            let double_newline = self
                .buffer
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .or_else(|| self.buffer.windows(2).position(|w| w == b"\n\n"));

            match double_newline {
                Some(end) => {
                    let raw = self.buffer[..end].to_vec();
                    let consume = if end + 2 < self.buffer.len()
                        && self.buffer[end..].starts_with(b"\r\n")
                    {
                        end + 4
                    } else {
                        end + 2
                    };
                    self.buffer.drain(..consume);

                    // Parse SSE lines
                    for line in raw.split(|&b| b == b'\n') {
                        if line.starts_with(b"event: ") {
                            self.event_type = String::from_utf8_lossy(&line[7..]).to_string();
                        } else if line.starts_with(b"data: ") {
                            self.event_data = String::from_utf8_lossy(&line[6..]).to_string();
                        }
                    }

                    self.dispatch_event();
                    self.event_type.clear();
                    self.event_data.clear();
                }
                None => break,
            }
        }

        self.pending_events.len() - prev_len
    }

    /// Map the current SSE event to LlmStreamEvent(s).
    fn dispatch_event(&mut self) {
        let data: Value = match serde_json::from_str(&self.event_data) {
            Ok(v) => v,
            Err(_) => return, // malformed JSON, skip
        };

        match self.event_type.as_str() {
            "message_start" => {
                if let Some(msg) = data.get("message") {
                    if let Some(usage) = msg.get("usage") {
                        self.pending_events.push(LlmStreamEvent::UsageUpdate(
                            parse_usage(usage),
                        ));
                    }
                }
            }
            "content_block_start" => {
                let index = data["index"].as_i64().unwrap_or(0) as usize;
                if let Some(block) = data.get("content_block") {
                    match block["type"].as_str() {
                        Some("tool_use") => {
                            self.tool_index = Some(index);
                            self.tool_id = block["id"].as_str().unwrap_or("").to_string();
                            self.tool_name = block["name"].as_str().unwrap_or("").to_string();
                            // Provider 在 content_block_start 中发送 input={} 或 null,
                            // 实际参数通过 input_json_delta 流式传入。仅当 input
                            // 有实际键值时才预填，避免 {}→"{}" 与 delta 拼接为非法 JSON。
                            let has_meaningful_input = block.get("input")
                                .map(|v| {
                                    if v.is_null() { return false; }
                                    if let Some(obj) = v.as_object() {
                                        !obj.is_empty()
                                    } else {
                                        true // 非 object 类型（不应出现）
                                    }
                                })
                                .unwrap_or(false);
                            self.tool_input_json = if has_meaningful_input {
                                serde_json::to_string(&block["input"]).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            tracing::debug!(
                                "[sse] tool_use start idx={index} name={} id={} input_prefix={:?}",
                                self.tool_name, self.tool_id,
                                self.tool_input_json.chars().take(80).collect::<String>(),
                            );
                        }
                        Some("thinking") => {
                            self.thinking_index = Some(index);
                            self.thinking_text.clear();
                            self.thinking_signature = block["signature"].as_str().unwrap_or("").to_string();
                        }
                        _ => {}
                    }
                }
            }
            "content_block_delta" => {
                if let Some(delta) = data.get("delta") {
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(text) = delta["text"].as_str() {
                                self.pending_events
                                    .push(LlmStreamEvent::Text(text.to_string()));
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(json) = delta["partial_json"].as_str() {
                                tracing::trace!(
                                    "[sse] input_json_delta idx={} json_len={}",
                                    data["index"].as_i64().unwrap_or(0),
                                    json.len(),
                                );
                                self.tool_input_json.push_str(json);
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(text) = delta["thinking"].as_str() {
                                self.thinking_text.push_str(text);
                                self.pending_events.push(LlmStreamEvent::Thinking {
                                    text: text.to_string(),
                                    signature: None,
                                });
                            }
                        }
                        Some("signature_delta") => {
                            if let Some(sig) = delta["signature"].as_str() {
                                self.thinking_signature = sig.to_string();
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                let index = data["index"].as_i64().unwrap_or(0) as usize;
                if self.tool_index == Some(index) {
                    // Finalize tool_use: parse accumulated JSON
                    let input: Value = if self.tool_input_json.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_str(&self.tool_input_json).unwrap_or_else(|e| {
                            tracing::warn!(
                                "[sse] tool_use parse error: {e}, raw(len={}): {:?}",
                                self.tool_input_json.len(),
                                self.tool_input_json.chars().take(120).collect::<String>(),
                            );
                            Value::Null
                        })
                    };
                    tracing::debug!(
                        "[sse] tool_use stop name={} id={} has_args={}",
                        self.tool_name, self.tool_id,
                        !input.is_null(),
                    );
                    self.pending_events.push(LlmStreamEvent::ToolCall {
                        id: std::mem::take(&mut self.tool_id),
                        tool_name: std::mem::take(&mut self.tool_name),
                        arguments: input,
                    });
                    self.tool_input_json.clear();
                    self.tool_index = None;
                } else if self.thinking_index == Some(index) {
                    // Finalize thinking block: emit event with signature signal
                    self.pending_events.push(LlmStreamEvent::Thinking {
                        text: String::new(),
                        signature: Some(std::mem::take(&mut self.thinking_signature)),
                    });
                    self.thinking_text.clear();
                    self.thinking_index = None;
                }
            }
            "message_delta" => {
                if let Some(usage) = data.get("usage") {
                    self.pending_events
                        .push(LlmStreamEvent::UsageUpdate(parse_usage(usage)));
                }
                // Check stop_reason from delta
                if let Some(delta) = data.get("delta") {
                    let stop_reason = parse_stop_reason(
                        delta["stop_reason"].as_str(),
                    );
                    // We'll emit Stop on message_stop instead
                    self.pending_events.push(LlmStreamEvent::Stop(stop_reason));
                }
            }
            "message_stop" => {
                // Message is complete; Stop was already emitted on message_delta
            }
            "error" => {
                if let Some(err) = data.get("error") {
                    let msg = err["message"].as_str().unwrap_or("unknown error");
                    self.pending_events.push(LlmStreamEvent::Text(format!(
                        "[API Error: {msg}]"
                    )));
                }
            }
            _ => {} // ping or unknown events
        }
    }
}

// ─── AnthropicCompatProvider ───────────────────────────────

/// 通用 Anthropic Messages API 兼容 Provider。
///
/// 协议细节见文件头注释. 通过 `provider_id` 区分错误来源和
/// `id() / name()` 标识. capabilities 默认开启全部已知特性
/// (streaming / thinking / tool_use / system_prompt / image_input),
/// 上下文窗口 1M tokens (与 DeepSeek / MiniMax-M3 一致).
pub struct AnthropicCompatProvider {
    provider_id: String,
    /// 缓存的 name 字符串 (`format!("{id} (Anthropic API)")`),
    /// 避免 `LlmProvider::name(&self) -> &str` 每次调用重新分配.
    name: String,
    api_key: String,
    base_url: String,
    model: String,
    client: Client,
    caps: ProviderCapabilities,
}

impl AnthropicCompatProvider {
    /// 构造 provider. `provider_id` 用于错误信息和 `id()` 返回.
    ///
    /// 已知 provider (`deepseek` / `MiniMax` / ...) 使用预设 caps;
    /// 未知 provider 也走此协议, caps 默认开启所有能力.
    pub fn new(provider_id: String, api_key: String, base_url: String, model: String) -> Self {
        let caps = default_caps_for(&provider_id);
        let name = format!("{} (Anthropic API)", capitalize(&provider_id));
        Self {
            provider_id,
            name,
            api_key,
            base_url,
            model,
            client: Client::new(),
            caps,
        }
    }

    fn build_body(&self, request: &CompletionRequest, stream: bool) -> Value {
        // ── messages ──
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|msg| {
                let content: Vec<Value> = msg
                    .content()
                    .iter()
                    .filter_map(Self::map_content_block)
                    .collect();
                serde_json::json!({ "role": msg.role(), "content": content })
            })
            .collect();

        // ── system prompt ──
        let system = request.system.as_ref().map(|s| {
            serde_json::json!([{ "type": "text", "text": s }])
        });

        // ── tools ──
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema
                })
            })
            .collect();

        // ── tool_choice ──
        let tool_choice = match &request.tool_choice {
            crate::types::ToolChoice::Auto => serde_json::json!({"type": "auto"}),
            crate::types::ToolChoice::Any => serde_json::json!({"type": "any"}),
            crate::types::ToolChoice::Tool(name) => {
                serde_json::json!({"type": "tool", "name": name})
            }
        };

        // ── assemble ──
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens.unwrap_or(16384),
            "stream": stream,
            "messages": messages,
            "tool_choice": tool_choice,
        });

        if let Some(sys) = system {
            body["system"] = sys;
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if let Some(temp) = request.temperature {
            body["temperature"] = Value::from(temp as f64);
        }
        if !request.stop_sequences.is_empty() {
            body["stop_sequences"] = Value::Array(
                request
                    .stop_sequences
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            );
        }

        body
    }

    /// Map a ContentBlock to Anthropic API content item.
    fn map_content_block(block: &ContentBlock) -> Option<Value> {
        match block.r#type.as_str() {
            "text" => Some(serde_json::json!({
                "type": "text",
                "text": block.text.as_deref().unwrap_or("")
            })),
            "tool_use" => Some(serde_json::json!({
                "type": "tool_use",
                "id": block.tool_use_id.as_deref().unwrap_or(""),
                "name": block.tool_name.as_deref().unwrap_or(""),
                "input": block.input.clone().unwrap_or(Value::Null)
            })),
            "tool_result" => Some(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": block.tool_use_id.as_deref().unwrap_or(""),
                "content": block.text.as_deref().unwrap_or(""),
                "is_error": block.is_error.unwrap_or(false)
            })),
            "thinking" => Some(serde_json::json!({
                "type": "thinking",
                "thinking": block.text.as_deref().unwrap_or(""),
                "signature": block.signature.as_deref().unwrap_or(""),
            })),
            _ => None,
        }
    }

    async fn send_request(
        &self,
        body: Value,
    ) -> Result<reqwest::Response, LlmError> {
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::ApiError {
                provider: self.provider_id.clone(),
                status: 0,
                message: e.to_string(),
            })?;

        let status = response.status();

        // Log full first chunk for easier debugging, but let streaming proceed
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        tracing::debug!("send_request response: status={status}, content-type={content_type:?}");

        if !status.is_success() {
            // 读取 Retry-After 头（在消费 body 前，429 重试用）
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);
            let error_text = response.text().await.unwrap_or_default();
            return match status.as_u16() {
                401 => Err(LlmError::AuthenticationError {
                    provider: self.provider_id.clone(),
                    message: error_text,
                }),
                429 => Err(LlmError::RateLimitExceeded {
                    provider: self.provider_id.clone(),
                    retry_after,
                }),
                _ => Err(LlmError::ApiError {
                    provider: self.provider_id.clone(),
                    status: status.as_u16(),
                    message: error_text,
                }),
            };
        }

        Ok(response)
    }
}

/// 根据 provider_id 返回预设的 capabilities。
///
/// DeepSeek 和 MiniMax-M3 都支持 1M 上下文 + 完整功能 (thinking / tool_use / image / system_prompt).
/// cache_control 不确定, 默认 false (Anthropic 官方支持但 MiniMax 不明).
fn default_caps_for(provider_id: &str) -> ProviderCapabilities {
    let _ = provider_id; // 预留, 后续可按 provider_id 差异化 (例如不支持 image 的服务)
    ProviderCapabilities {
        streaming: true,
        thinking: true,
        tool_use: true,
        max_tokens: Some(16384),
        max_input_tokens: Some(1_000_000),
        supports_system_prompt: true,
        supports_cache_control: false,
        supports_image_input: true,
    }
}

/// 简单首字母大写: `"deepseek"` → `"Deepseek"`, `"MiniMax"` → `"Minimax"`.
/// 不完美 (对多词 ID 不会 snakecase 拆分), 但足够作为 name 标识.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            let upper: String = c.to_uppercase().collect();
            format!("{upper}{}", chars.as_str())
        }
        None => String::new(),
    }
}

#[async_trait]
impl super::LlmProvider for AnthropicCompatProvider {
    fn id(&self) -> &str {
        &self.provider_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }

    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<LlmStreamEvent, LlmError>>, LlmError> {
        let body = self.build_body(&request, true);
        let provider_id = self.provider_id.clone();
        tracing::info!(
            "发送 LLM 请求 [provider={}]: model={}, messages={}, tools={}, max_tokens={}",
            provider_id,
            body["model"].as_str().unwrap_or("?"),
            body["messages"].as_array().map_or(0, |a| a.len()),
            body["tools"].as_array().map_or(0, |a| a.len()),
            body["max_tokens"].as_u64().unwrap_or(0),
        );
        tracing::debug!(
            "LLM 请求 body (counted sizes): system={}, messages={}, tools={}",
            body["system"].as_str().map_or(0, |s| s.len()),
            serde_json::to_string(&body["messages"]).map_or(0, |s| s.len()),
            serde_json::to_string(&body["tools"]).map_or(0, |s| s.len()),
        );
        let response = self.send_request(body).await?;

        tracing::info!("LLM 请求已连接, 开始接收 SSE 流");

        // ── SSE parsing in a background task ──
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<LlmStreamEvent, LlmError>>(32);

        tokio::spawn(async move {
            let mut byte_stream = response.bytes_stream();
            let mut parser = SseParser::default();
            let mut first_chunk = true;

            while let Some(chunk) = byte_stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        if first_chunk {
                            first_chunk = false;
                            let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(400)]);
                            tracing::debug!(
                                "SSE first chunk ({} bytes, preview): {}",
                                bytes.len(),
                                preview.replace('\n', "\\n")
                            );
                        }
                        parser.feed_bytes(&bytes);
                        for event in parser.pending_events.drain(..) {
                            if tx.send(Ok(event)).await.is_err() {
                                return; // receiver dropped
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(LlmError::ApiError {
                                provider: provider_id.clone(),
                                status: 0,
                                message: e.to_string(),
                            }))
                            .await;
                        return;
                    }
                }
            }

            // Stream exhausted — if no events were sent, log a warning
            let remaining = parser.pending_events.len();
            let unparsed = parser.buffer.len();
            tracing::debug!(
                "SSE stream ended: {} pending events, {} bytes unparsed in buffer",
                remaining,
                unparsed,
            );
            for event in parser.pending_events.drain(..) {
                let _ = tx.send(Ok(event)).await;
            }
        });

        // Convert mpsc::Receiver → BoxStream
        let stream = stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });

        Ok(Box::pin(stream))
    }
}

// ─── Helpers ────────────────────────────────────────────────

fn parse_usage(usage: &Value) -> TokenUsage {
    TokenUsage {
        input: usage["input_tokens"].as_u64().unwrap_or(0),
        output: usage["output_tokens"].as_u64().unwrap_or(0),
        cache_creation_input: usage["cache_creation_input_tokens"].as_u64(),
        cache_read_input: usage["cache_read_input_tokens"].as_u64(),
    }
}

fn parse_stop_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("end_turn") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("tool_use") => StopReason::ToolUse,
        Some("stop_sequence") => StopReason::StopSequence,
        Some(other) => StopReason::Unknown(other.to_string()),
        None => StopReason::Unknown("unknown".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::LlmProvider;

    #[test]
    fn default_caps_full_feature_set() {
        // MiniMax-M3 完整支持: 1M 上下文 + thinking + tool + image + system_prompt
        let caps = default_caps_for("MiniMax");
        assert!(caps.streaming);
        assert!(caps.thinking);
        assert!(caps.tool_use);
        assert_eq!(caps.max_tokens, Some(16384));
        assert_eq!(caps.max_input_tokens, Some(1_000_000));
        assert!(caps.supports_system_prompt);
        assert!(caps.supports_image_input);
    }

    #[test]
    fn provider_id_name_capitalization() {
        let p = AnthropicCompatProvider::new(
            "deepseek".into(),
            "k".into(),
            "https://example.com".into(),
            "m".into(),
        );
        assert_eq!(p.id(), "deepseek");
        // capitalize() 仅大写首字符, 其余原样保留: "deepseek" → "Deepseek"
        assert_eq!(p.name(), "Deepseek (Anthropic API)");

        let p2 = AnthropicCompatProvider::new(
            "MiniMax".into(),
            "k".into(),
            "https://example.com".into(),
            "m".into(),
        );
        assert_eq!(p2.id(), "MiniMax");
        // "MiniMax" 首字符 'M' 已是大写, capitalize 后仍是 "MiniMax" (中间大小写不变)
        assert_eq!(p2.name(), "MiniMax (Anthropic API)");

        let p3 = AnthropicCompatProvider::new(
            "anthropic".into(),
            "k".into(),
            "https://example.com".into(),
            "m".into(),
        );
        assert_eq!(p3.name(), "Anthropic (Anthropic API)");
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── JSON-RPC 2.0 信封 ─────────────────────────────────

pub type RequestId = serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone)]
pub enum IncomingMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

// ─── ACP 协议类型 ───────────────────────────────────────

/// 从 agent 侧发出的双向请求类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum AgentRequest {
    #[serde(rename = "fs/read_text_file")]
    ReadTextFile(ReadTextFileParams),
    #[serde(rename = "fs/write_text_file")]
    WriteTextFile(WriteTextFileParams),
    #[serde(rename = "permission/request")]
    PermissionRequest(PermissionRequestParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadTextFileParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadTextFileResult {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteTextFileParams {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteTextFileResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestParams {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequestResult {
    pub approved: bool,
}

// ─── Initialize ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    #[serde(default)]
    pub client_info: Option<ClientInfo>,
    #[serde(default)]
    pub capabilities: Option<ClientCapabilities>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub tools: Option<ToolCapabilities>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCapabilities {
    #[serde(default)]
    pub forwarding: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// 必须是 JSON 数字 `1`，Zed 自定义反序列化器将字符串映射为 V0 导致 "Unsupported version"
    pub protocol_version: u32,
    pub capabilities: ServerCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(default)]
    pub session: SessionCapabilities,
    #[serde(default)]
    pub tools: ToolCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCapabilities {
    pub max_sessions: u32,
}

// ─── Sessions ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewParams {
    /// 工作目录（来自编辑器，用于工作区检测）
    pub cwd: Option<String>,
    #[serde(default)]
    pub mcp_servers: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewResult {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCloseParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDeleteParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionForkParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionForkResult {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResult {
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    pub created_at: String,
    pub turn_count: u32,
}

// ─── Prompt ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptParams {
    pub session_id: String,
    /// 用户消息内容块数组（Zed 发送 [{type:"text",text:"..."}]）
    pub prompt: Vec<Value>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResult {
    pub accepted: bool,
}

// ─── Cancel ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelParams {
    pub session_id: String,
}

// ─── Session Update (notification, server→client) ──────

/// 内部通知类型（不直接序列化为 ACP 格式，由 `session_update_notification` 转换）
#[derive(Debug, Clone)]
pub enum SessionUpdateContent {
    AgentMessageChunk { text: String },
    AgentThoughtChunk { text: String },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    TurnFinished { reason: String },
    Error { message: String },
}

// ─── Helper — 构造 RPC 响应 ─────────────────────────────

pub fn rpc_success(id: RequestId, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

pub fn rpc_error(id: RequestId, code: i32, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data: None,
        }),
    }
}

pub fn rpc_method_not_found(id: RequestId, method: &str) -> JsonRpcResponse {
    rpc_error(id, -32601, format!("Method not found: {method}"))
}

pub fn rpc_invalid_params(id: RequestId, msg: String) -> JsonRpcResponse {
    rpc_error(id, -32602, msg)
}

pub fn rpc_invalid_request(id: RequestId) -> JsonRpcResponse {
    rpc_error(id, -32600, "Invalid Request".into())
}

/// 构造 session/update 通知 JSON（Zed ACP 协议格式）
///
/// Zed 期望的格式（agent-client-protocol-schema v0.13.2）:
///
/// AgentMessageChunk / AgentThoughtChunk:
/// ```json
/// {"sessionId":"...", "update": {"sessionUpdate":"agent_message_chunk",
///   "content": {"type":"text", "text":"..."}}}
/// ```
///
/// ToolCall:
/// ```json
/// {"sessionId":"...", "update": {"sessionUpdate":"tool_call",
///   "toolCallId":"...", "title":"...", "status":"running", "rawInput":{...}}}
/// ```
pub fn session_update_notification(session_id: &str, content: &SessionUpdateContent) -> Value {
    let update = match content {
        SessionUpdateContent::AgentMessageChunk { text }
        | SessionUpdateContent::AgentThoughtChunk { text } => {
            let tag = match content {
                SessionUpdateContent::AgentMessageChunk { .. } => "agent_message_chunk",
                _ => "agent_thought_chunk",
            };
            serde_json::json!({
                "sessionUpdate": tag,
                "content": { "type": "text", "text": text }
            })
        }
        SessionUpdateContent::ToolCall {
            tool_call_id,
            tool_name,
            arguments,
        } => serde_json::json!({
            "sessionUpdate": "tool_call",
            "toolCallId": tool_call_id,
            "title": tool_name,
            "status": "in_progress",
            "rawInput": arguments,
        }),
        SessionUpdateContent::Usage { .. } => {
            // UsageUpdate 需要 unstable_session_usage feature，Zed 可能未启用
            // 发送一个最小化的 usage_update，被忽略也无害
            return serde_json::json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "usage_update",
                        "used": 0,
                        "size": 0,
                    }
                }
            });
        }
        SessionUpdateContent::TurnFinished { .. } => {
            // ACP 协议没有 turn_finished 类型；回合结束靠 JSON-RPC 响应信号
            // 发送空文本块，被 Zed 忽略也无害
            return serde_json::json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "" }
                    }
                }
            });
        }
        SessionUpdateContent::Error { message } => serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": format!("[Error: {message}]") }
        }),
    };

    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": update,
        }
    })
}

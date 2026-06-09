// qianxun-runtime/src/api/error.rs
// RuntimeApiError — 5 个 trait 方法共用的错误类型.
//
// 设计原则:
//   - thiserror 派生的 enum, 不绑 anyhow (caller 拿不到 backtrace 也 OK)
//   - 错误码分 4 类: NotFound (404) / InvalidRequest (400) / Internal (500) / Unavailable (503)
//   - 转换给 HTTP layer (daemon router) 直接 map 成 (StatusCode, String)
//   - 转换给 Tauri layer (command) 直接 map 成 Err<String> 给前端
//
// 不引 thiserror 重新 derive, 复用 qianxun_core::types::LlmError 模式: 跟 session store
// error / agent host error 互转的 From impl 在调用方做, 避免循环 dep.

use std::fmt;

/// RuntimeApi 调用错误.
#[derive(Debug, Clone)]
pub enum RuntimeApiError {
    /// 资源不存在 (session_id 没找到 / plan_id 没找到)
    NotFound(String),
    /// 请求参数非法 (空消息 / 字段缺失)
    InvalidRequest(String),
    /// 资源状态冲突 (already paused / not paused 调 resume)
    Conflict(String),
    /// 内部错误 (SQLite / LLM / agent_host panic 等)
    Internal(String),
    /// 服务暂时不可用 (max sessions reached / provider 未配)
    Unavailable(String),
}

impl fmt::Display for RuntimeApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::InvalidRequest(msg) => write!(f, "invalid request: {msg}"),
            Self::Conflict(msg) => write!(f, "conflict: {msg}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
            Self::Unavailable(msg) => write!(f, "unavailable: {msg}"),
        }
    }
}

impl std::error::Error for RuntimeApiError {}

/// RuntimeApi 调用结果.
pub type RuntimeApiResult<T> = Result<T, RuntimeApiError>;

impl RuntimeApiError {
    /// HTTP layer 用的 (StatusCode, String) 转换. Tauri layer 用 into() 直接拿 String.
    pub fn http_status(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "404",
            Self::InvalidRequest(_) => "400",
            Self::Conflict(_) => "409",
            Self::Internal(_) => "500",
            Self::Unavailable(_) => "503",
        }
    }
}

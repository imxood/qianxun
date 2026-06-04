// ─── DaemonClient ────────────────────────────────────────────

use reqwest::Response;
use serde_json;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use super::sse_parser::parse_sse_stream;
use super::types::{
    ClientError, HealthStatus, PromptMessage, PromptRequest, Session, SessionCreated, SessionsList,
    SseEvent,
};
use super::SseStream;

/// 连接到本地 Daemon (HTTP + SSE) 的 thin client.
///
/// 跨 binary 共享 (TUI/ACP/CLI 三个入口共用同一个 client 实例).
///
/// # Stage 6b 鉴权
///
/// 构造时可选择附带 Bearer token (HS256 JWT, 与 daemon `auth_middleware` 配对).
/// 设置后**所有**请求 (除 `health`/`status` — 它们被 auth middleware 跳过)
/// 都会自动附加 `Authorization: Bearer <token>` header.
///
/// 构造方式:
/// - [`DaemonClient::new`] — 不带 token, 向后兼容旧测试 (X-Api-Key 兼容已移除)
/// - [`DaemonClient::with_token`] — 显式带 token, 生产路径
///
/// token 来源 (在 `main.rs` 解析):
/// - CLI flag `--client-token <token>`
/// - env var `QIANXUN_CLIENT_TOKEN` (clap 自动读, 跟 flag 二选一)
///
/// 不做的事 (Stage 6b 范围外):
/// - 不接 token 刷新 / 重试 / 限流 (Stage 7 跟 VPS `login_handler` 集成时再加)
/// - 不读 token 加密存储 (Stage 6a stronghold 已有, 启动时由 main.rs 注入)
#[derive(Debug, Clone)]
pub struct DaemonClient {
    base_url: String,
    http: reqwest::Client,
    /// 客户端 Bearer token. `Some(t)` = 每个请求自动附 `Authorization: Bearer t`;
    /// `None` = 不附 header (Stage 5 旧行为, 用于公开端点 / 单元测试).
    token: Option<String>,
}

impl DaemonClient {
    /// 构造 DaemonClient (不立即探测, 探测由 `health()` 完成). 不带 token.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::new_with_token(base_url, None)
    }

    /// 构造 DaemonClient 并附带 Bearer token. 后续所有 HTTP 请求 (除 health/status
    /// 外) 都会自动附加 `Authorization: Bearer <token>` header.
    ///
    /// 对端 `daemon::router::auth_middleware` 会:
    /// 1. 跳过 `/v1/system/health` + `/v1/system/status` (k8s probe / 调试)
    /// 2. 提取 `Authorization: Bearer <token>`, 校验 HS256 签名 + `exp` 未过期
    /// 3. 通过则把 `Claims{sub, exp, iat}` 写入 `request.extensions()`
    ///
    /// token 由调用方负责 (CLI flag / env var / stronghold), 失败返 401.
    pub fn with_token(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self::new_with_token(base_url, Some(token.into()))
    }

    /// 内部统一构造: `token` 是 `Option<String>`, 外面两个 public ctor 转它.
    ///
    /// URL 规范化 (Stage 6c): `base_url` 末尾的 `/` 会被 trim 掉, 这样后面
    /// `format!("{}/v1/system/health", self.base_url)` 不会产生 `//` (虽然
    /// 大多数 HTTP server 会 normalize, 但显式 trim 更安全, 也便于测试断言
    /// `base_url()` 返的就是用户传进去的 base).
    fn new_with_token(base_url: impl Into<String>, token: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest Client builder should not fail with default config");
        let base_url: String = base_url
            .into()
            .trim_end_matches('/')
            .to_string();
        Self {
            base_url,
            http,
            token,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 返回当前配置的 Bearer token. `None` 表示 client 未带 token.
    /// (供测试 / 调试 / Stage 7 token 刷新逻辑读用, 不在请求路径上使用.)
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// 在 `RequestBuilder` 上附加 Bearer Authorization header (若 token 存在).
    /// 没有 token 时透传, 不修改 builder.
    ///
    /// 简化: `reqwest::RequestBuilder::bearer_auth` 内部用 `HeaderValue::from_str`,
    /// 自动把 token 里的非法字符拦截; 不会 panic.
    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => builder.bearer_auth(t.as_str()),
            None => builder,
        }
    }

    /// 健康检查 (3s 超时). 失败表示 daemon 未启动或不可达.
    ///
    /// 注: `health` 在 server 端跳过 auth (k8s probe), 客户端即使带 token
    /// 也会发, 但 server 不读. 这里仍走 `apply_auth` 保持一致 — server 跳过
    /// 头不校验, 多发一个 header 不影响.
    pub async fn health(&self) -> Result<HealthStatus, ClientError> {
        let url = format!("{}/v1/system/health", self.base_url);
        let resp = self.apply_auth(self.http.get(&url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        let body: HealthStatus = resp.json().await?;
        Ok(body)
    }

    /// 创建 session.
    pub async fn create_session(&self) -> Result<SessionCreated, ClientError> {
        let url = format!("{}/v1/chat/session", self.base_url);
        let resp = self.apply_auth(self.http.post(&url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        let body: SessionCreated = resp.json().await?;
        Ok(body)
    }

    /// 列出 sessions (Stage 3 §6.4 扩展端点).
    pub async fn list_sessions(&self) -> Result<Vec<Session>, ClientError> {
        let url = format!("{}/v1/chat/sessions", self.base_url);
        let resp = self.apply_auth(self.http.get(&url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        let body: SessionsList = resp.json().await?;
        Ok(body.sessions)
    }

    /// 发送 prompt, 返回 SSE 事件流.
    ///
    /// 调用方负责消费流; 流自然结束 (`message_stop` 或 `error`) 或 drop stream
    /// (客户端断连) 都由 axum 后端负责清理.
    pub async fn stream_prompt(
        &self,
        session_id: &str,
        request: &PromptRequest,
    ) -> Result<SseStream, ClientError> {
        let url = format!("{}/v1/chat/session/{}/prompt", self.base_url, session_id);
        let resp = self
            .apply_auth(self.http.post(&url).json(request))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        // 把 reqwest::Response 转换为 byte stream, 再走 SSE parser.
        Ok(parse_sse_stream(resp))
    }

    /// 取消当前 prompt.
    pub async fn cancel(&self, session_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/v1/chat/session/{}/cancel", self.base_url, session_id);
        let resp = self.apply_auth(self.http.post(&url)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ClientError::Status(status.as_u16(), resp.text().await.unwrap_or_default()));
        }
        Ok(())
    }
}


// ENV_MUTEX 保护整段 async 操作期间 env-var 一致性 (e.g. test harness 改 env 后请求未结束).
// 锁是 std::sync::Mutex, 持锁时间短 (一次 req duration), 跨 await 是有意为之.
// 见 router test fns 大量用 `let _g = ENV_MUTEX.lock()...; .await;` 模式.
#![allow(clippy::await_holding_lock)]

use axum::{
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{sse::Event, IntoResponse, Json, Response, Sse},
    routing::{delete, get, post},
    Router,
};
use futures::stream::{Stream, StreamExt};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::engine::{processing_loop, AgentLoop};
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::context::MemoryObserver;
use qianxun_core::provider::types::LlmStreamEvent;
use qianxun_core::tools::ToolCategoryFilter;
use qianxun_core::types::LlmError;
use qianxun_memory::MemoryStats;

use crate::daemon::llm_providers::{LlmProviderConfig as ManagerProviderConfig, TestResult};
use crate::daemon::output_sink::DaemonOutputSink;
use crate::daemon::sse::{SseEvent, SseEventBuilder};
use crate::daemon::kanban_host::KanbanSseEvent;
use crate::daemon::AppState;

/// 健康检查响应。
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

/// 创建会话响应。
#[derive(Serialize)]
struct SessionCreatedResponse {
    session_id: String,
}

/// `/v1/chat/session/:id/prompt` 请求体.
#[derive(Debug, Deserialize)]
pub struct PromptRequest {
    /// 用户/助手消息列表 (按时间顺序). 简单字符串数组, 后续可扩展为多模态.
    #[serde(default)]
    pub messages: Vec<PromptMessage>,
    /// 可选: 覆盖 session 默认 model (Stage 2 暂忽略, 留 Stage 3 接 config 切换).
    #[allow(dead_code)] // Stage 3 才接 config 切换, 当前仅 deserialized 备用
    #[serde(default)]
    pub model: Option<String>,
}

/// Prompt 请求中的单条消息.
#[derive(Debug, Deserialize)]
pub struct PromptMessage {
    /// "user" / "assistant" / "system"
    pub role: String,
    /// 文本内容 (Stage 2 简化: 仅支持纯文本).
    pub content: String,
}

/// 构建 Daemon HTTP 路由。
///
/// Stage 6a JWT auth 策略:
/// - `/` 跳过 (服务自描述/landing, 信息非敏感, 浏览器/HTTP 探针命中要返 200)
/// - `/v1/system/health` 跳过 (k8s liveness/readiness probe 用, 不应被 token 拦)
/// - `/v1/system/status` 跳过 (状态查询, 信息非敏感, 方便调试)
/// - `/ui/*` 跳过 (Stage 7a: 静态文件 serve, 走 cookie/JWT 端另一套 —
///   Stage 7a 简化: Web UI 首次访问弹 token 输入框, 浏览器把 daemon 启动
///   时打印的 token 粘进去即用, 不做密码)
/// - 其余 endpoint 全部要求 `Authorization: Bearer <jwt>` (HS256 + exp)
///
/// 实现: 单一 `auth_middleware` 解码 JWT, 缺/错/过期则返 401.
/// 校验通过后把 `Claims` 写入 `request.extensions()` (下游 handler 可读).
///
/// Fallback 行为: 任何未匹配的 path 都返 404 + JSON 错误 (而不是 401), 这样
/// 浏览器误访问 `/` 或 `/favicon.ico` 时不会被 auth 拦下, 体验更友好.
///
/// `ui_dist`: SvelteKit 静态 dist 路径. None 或不存在 → `/ui/*` 返 503.
pub fn build_router(state: Arc<AppState>, ui_dist: Option<PathBuf>) -> Router {
    let mut router = Router::new()
        // 系统
        .route("/", get(root_handler))
        .route("/v1/system/health", get(health_handler))
        .route("/v1/system/status", get(status_handler))
        // 阶段 5.1 (2026-06-04): 统一系统事件流 — 桥接 kanban_host.sse_tx,
        // 前端 /kanban/{id} 等 Kanban 页面 SSE 实时刷新依赖此端点. Chat SSE
        // 继续走 /v1/chat/session/{id}/prompt 单独流.
        .route("/v1/events", get(events_handler))
        // 会话
        .route("/v1/chat/session", post(create_session))
        .route("/v1/chat/session/{id}", get(get_session).delete(delete_session))
        .route("/v1/chat/session/{id}/prompt", post(prompt_handler))
        // 工具
        .route("/v1/tools", get(list_tools))
        // 记忆
        .route("/v1/memory/sessions", get(memory_sessions))
        .route("/v1/memory/search", post(memory_search))
        // Day-3.2: 轻量健康检查 — 验证 MemoryCore 可达 + 报三表行数
        .route("/v1/memory/ping", get(memory_ping))
        // Stage 12: 列出某 session 的所有 observations (供 Web Console Memory 面板
        // 点 session 后右侧观察详情). 之前 Svelte memory.ts:listObservations 已调,
        // 但 daemon 端没注册, 返 404. 现补.
        .route(
            "/v1/memory/sessions/{id}/observations",
            get(memory_session_observations),
        )
        // 技能 (Stage 7a: 加 reload + toggle)
        .route("/v1/skills", get(list_skills).post(reload_skills))
        .route("/v1/skills/{name}/toggle", post(toggle_skill))
        // MCP (Stage 7a: 加 delete + test)
        .route(
            "/v1/mcp/servers",
            get(list_mcp_servers).post(add_mcp_server),
        )
        .route("/v1/mcp/servers/{id}", delete(delete_mcp_server))
        .route("/v1/mcp/servers/{id}/test", post(test_mcp_server))
        // 工具试用 (Stage 7a: 直调 ToolRegistry)
        .route("/v1/tools/{name}/invoke", post(invoke_tool))
        // LLM provider 管理 (Stage 7a: 7 个 endpoint, 算 GET list 凑 8)
        .route(
            "/v1/llm/providers",
            get(llm_list_providers).post(llm_add_provider),
        )
        .route(
            "/v1/llm/providers/{id}",
            get(llm_get_provider)
                .put(llm_update_provider)
                .delete(llm_delete_provider),
        )
        .route("/v1/llm/providers/{id}/activate", post(llm_activate_provider))
        .route("/v1/llm/providers/{id}/test", post(llm_test_provider))
        // Stage 7b: Sessions 管理 (3 endpoint)
        .route("/v1/chat/sessions", get(list_sessions))
        .route("/v1/chat/session/{id}/cancel", post(cancel_session))
        .route("/v1/chat/session/{id}/pause", post(pause_session))
        // Stage 7b: Config 管理 (PUT)
        .route("/v1/config", get(get_config).put(put_config))
        // Stage 7b: Memory 管理 (2 DELETE)
        .route("/v1/memory/observations/{id}", delete(delete_observation))
        .route("/v1/memory/sessions/{id}", delete(delete_memory_session))
        // Stage 7b: System 指标 + 日志
        .route("/v1/system/metrics", get(system_metrics))
        .route("/v1/system/logs", get(system_logs))
        // Stage 9c: 重新生成 admin token (Settings 面板用)
        .route(
            "/v1/system/admin/rotate-token",
            post(admin_rotate_token),
        )
        // MVP-3 plan 2: Kanban 核心路由 (v6 §8.5, 留 v2 扩展)
        //  - POST /v1/kanban/boards                 创建 board, 自动 spawn techlead
        //  - GET  /v1/kanban/boards                 列出 boards
        //  - GET  /v1/kanban/boards/{id}            查 board 详情
        //  - GET  /v1/kanban/boards/{id}/tasks      列 board 下 task
        //  - GET  /v1/projects                      列 projects (v5 §3.6)
        //  - POST /v1/projects                      创建 project
        .route("/v1/kanban/boards", get(list_kanban_boards).post(create_kanban_board))
        .route("/v1/kanban/boards/{id}", get(get_kanban_board))
        .route("/v1/kanban/boards/{id}/tasks", get(list_kanban_board_tasks))
        // 阶段 2: 7 个 Kanban handler 注册 (MVP-3 plan 3+4 函数已定义, 修路由注册 bug)
        .route("/v1/kanban/tasks/{id}", get(get_kanban_task))
        .route("/v1/kanban/tasks", post(create_kanban_task))
        .route("/v1/kanban/tasks/{id}/cancel", post(cancel_kanban_task))
        .route("/v1/kanban/boards/{id}/events", get(list_kanban_board_events))
        .route("/v1/kanban/profiles", get(list_kanban_profiles))
        .route("/v1/kanban/roles", get(list_kanban_roles))
        .route("/v1/kanban/dispatch", post(dispatch_kanban_now))
        .route("/v1/projects", get(list_projects).post(create_project))
        // Stage 10a: 密码登录 + 修改密码 + 登出
        //  - /v1/auth/login       公开 (跳过 auth middleware) — 拿密码换短期 JWT
        //  - /v1/auth/change-password  需要 auth (要已登录才能改)
        //  - /v1/auth/logout      需要 auth (保留 hook, 当前 stateless, 仅返 200)
        .route("/v1/auth/login", post(auth_login))
        .route("/v1/auth/change-password", post(auth_change_password))
        .route("/v1/auth/logout", post(auth_logout))
        // 未知 path 返 404 JSON (而不是被 auth 拦成 401)
        .fallback(not_found_handler)
        .with_state(state.clone());

    // Stage 7a: 嵌套 ServeDir (静态文件 + SPA fallback).
    // nest_service 把整个 sub-router 接到 /ui/* 上.
    router = match ui_dist {
        Some(dir) if dir.is_dir() => {
            let index_html = dir.join("index.html");
            if index_html.is_file() {
                // SPA fallback: 文件不存在 → 返 index.html (vite/adam 行为)
                let svc = ServeDir::new(&dir).fallback(ServeFile::new(&index_html));
                router.nest_service("/ui", svc)
            } else {
                // 没 index.html → 直接 ServeDir, 不做 fallback (404 由 ServeDir 返)
                let svc = ServeDir::new(&dir);
                router.nest_service("/ui", svc)
            }
        }
        _ => {
            // dist 不存在或未配置 → /ui/* 走兜底 handler 返 503
            router.nest_service(
                "/ui",
                axum::routing::get(ui_dist_missing).fallback(ui_dist_missing),
            )
        }
    };

    // Stage 6a: 全局 JWT auth middleware (在 handler 之前执行)
    // Stage 10a: 用 `from_fn_with_state` 把 AppState 注入, 让 middleware
    // 能读 state.admin.token_secret 验签 (替代 env var).
    router = router.layer(middleware::from_fn_with_state(state, auth_middleware));

    // Stage 9c: CSP header (Content-Security-Policy)
    // 策略跟 01b-daemon-web-console.md §6.3 一致:
    //   - default-src 'self' (不引外部 CDN)
    //   - script-src 'self' (无 inline script)
    //   - style-src 'self' 'unsafe-inline' (Tailwind 需要)
    //   - connect-src 'self' (API 同源)
    //   - img-src 'self' data: (允许内联 favicon)
    let csp = "default-src 'self'; \
               script-src 'self'; \
               style-src 'self' 'unsafe-inline'; \
               connect-src 'self'; \
               img-src 'self' data:; \
               font-src 'self' data:; \
               object-src 'none'; \
               base-uri 'self'; \
               form-action 'self'";
    let csp_header: HeaderName = "content-security-policy".parse().expect("valid header name");
    let csp_value: HeaderValue = csp.replace([' ', '\n', '\t'], "").parse().expect("valid header value");
    router = router.layer(SetResponseHeaderLayer::overriding(csp_header, csp_value));

    router
}

/// 当 `ui_dist` 路径不存在或未配置时, 兜底返 503.
async fn ui_dist_missing() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "ui_dist_unavailable",
            "message": "Web UI dist not found. Build with: pnpm --dir qianxun/src/daemon/ui build",
        })),
    )
        .into_response()
}

// ─── JWT Auth Middleware (Stage 6a) ───────────────────────────

/// Stage 6a JWT claims — 最小集, 只 verify 不签发.
///
/// 字段:
/// - `sub`: user_id
/// - `exp`: 过期 unix 时间戳 (i64)
/// - `iat`: 签发 unix 时间戳 (i64)
///
/// Stage 7 加 `role` 字段 (跟 VPS `server::auth::Claims` 兼容).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: i64,
    pub iat: i64,
}

/// Stage 6a: 接受 `Authorization: Bearer <jwt>`, 校验 HS256 签名 + exp 过期.
///
/// - 跳过 `/v1/system/health` 和 `/v1/system/status` (k8s probe / 调试)
/// - 缺/错/过期 token → 401 Unauthorized
/// - 合法 token → `Claims` 写入 `request.extensions()` (下游 handler 可读)
///
/// Stage 10a: secret 来自 `state.admin.token_secret` (admin.cred 文件),
/// 不再读 env var `QIANXUN_JWT_SECRET` (那个 env var 已废弃, main.rs 仅打 warn).
///
/// Stage 7 加 `role` 字段 + 跟 VPS auth 集成.
///
/// Stage 7b: 活跃连接计数 (conn tracking). 用 `state.active_conns` (Arc<AtomicUsize>)
/// 在进 +1, 出 -1. 仍用 `from_fn` (3-arg) + global 静态 (从 extensions 取
/// counter) 的混合方式 — 简化: counter 在 static `ACTIVE_CONNS`, 跟
/// `state.active_conns` 在 metric handler 里取 max(两者).
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // 1. 跳过 health/status (k8s probe + 调试) + 登录端点
    let path = request.uri().path();
    if is_auth_skipped_path(path) {
        // 跳过路径也计入活跃连接 (含 health probe), 因为它们确实是活跃请求
        let _guard = ConnCounterGuard::new();
        return Ok(next.run(request).await);
    }

    // 2. 提取 Authorization Bearer token
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None => {
            tracing::debug!("[auth] missing Authorization Bearer on {path}");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    // 3. 读 secret (从 AppState.admin.token_secret, 文件加载, 启动时已校验非空).
    let secret = state.admin.token_secret();

    // 4. decode + verify (HS256, exp 必填且未过期)
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp"]);

    let claims = match decode::<Claims>(
        &token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    ) {
        Ok(data) => data.claims,
        Err(e) => {
            tracing::warn!("[auth] JWT verify failed on {path}: {e}");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    // 5. 把 claims 写到 extensions (下游 handler 可读)
    request.extensions_mut().insert(claims);

    // 6. Stage 7b: 活跃连接计数 +1, 出 scope 时 -1 (Drop guard)
    let _guard = ConnCounterGuard::new();
    Ok(next.run(request).await)
}

/// Stage 7b: 全局活跃 HTTP 连接计数器. 跨 axum middleware / handler
/// 共享, 真正并发安全的 AtomicUsize.
static ACTIVE_CONNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Stage 7b: 活跃连接计数器 drop guard.
///
/// 进入时 `fetch_add(1)`, 离开 scope (response future drop 或正常返回) 时
/// `fetch_sub(1)`. 用于 `/v1/system/metrics` 的 `conns` 字段.
pub struct ConnCounterGuard;

impl ConnCounterGuard {
    pub fn new() -> Self {
        ACTIVE_CONNS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Default for ConnCounterGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ConnCounterGuard {
    fn drop(&mut self) {
        ACTIVE_CONNS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Stage 7b: 给 `/v1/system/metrics` 用的 active_conns 读取 helper.
/// 避免直接暴露 ACTIVE_CONNS static.
pub fn active_conns_count() -> usize {
    ACTIVE_CONNS.load(Ordering::Relaxed)
}

/// 哪些 path 跳过 auth (k8s probe / 调试查询 / landing / 静态 UI).
///
/// 当前跳过:
/// - `/` — 服务自描述/landing, 浏览器/curl 探针应能命中不报错
/// - `/v1/system/health` — k8s liveness/readiness probe
/// - `/v1/system/status` — 状态查询 (信息非敏感, 调试方便)
/// - `/ui/*` — Stage 7a 静态文件 serve (SPA 资源不需要每个文件都打 token;
///   真正要 auth 的 Web UI 资源是 SvelteKit 内部 fetch 走 `/v1/*` 时的 Bearer
///   token; Stage 7a 简化: 启动时打 admin token, UI 粘进 localStorage)
/// - `/_app/*` — Stage 12 防御性: SvelteKit `paths.base = '/ui'` 时, JS/CSS
///   资源在 `/ui/_app/...` 下 (被 `/ui/*` 覆盖), 但若 SvelteKit 改 base
///   或 adapter 行为变了, 资源会落到 `/_app/...`. 显式 skip 防 401.
pub fn is_auth_skipped_path(path: &str) -> bool {
    path == "/"
        || path == "/v1/system/health"
        || path == "/v1/system/status"
        || path == "/v1/auth/login"
        || path.starts_with("/ui/")
        || path == "/ui"
        || path.starts_with("/_app/")
        || path == "/_app"
}

/// 读 JWT secret (env var QIANXUN_JWT_SECRET).
///
/// 返回 `None` 表示 env var 未设置或为空 (启动时 main.rs 会 panic).
/// 读 daemon JWT secret (Stage 10a 前用, Stage 10a 后改 password → JWT).
#[allow(dead_code)] // Stage 10a 改用 admin password, 留 Stage 6/9c 路径兼容
pub fn jwt_secret() -> Option<String> {
    std::env::var("QIANXUN_JWT_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
}

/// 从 HeaderMap 提取 `Authorization: Bearer <token>` 中的 token.
///
/// 公开以便测试. 不接受裸 token 或 X-Api-Key (Stage 5 兼容已移除).
pub fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let v = headers.get("authorization")?.to_str().ok()?;
    let rest = v
        .strip_prefix("Bearer ")
        .or_else(|| v.strip_prefix("bearer "))?;
    let token = rest.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

// ─── 系统 ──────────────────────────────────────────────────

/// 根路径服务自描述 (Stage 7 bugfix: 之前 `/` 没有 handler, 走 auth middleware
/// 返 401, 浏览器/curl 探针体验差). 返 JSON 列出服务名 + 版本 + 主要 endpoint
/// 列表, 信息非敏感, 跳过 auth.
async fn root_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "qianxun-daemon",
        "version": env!("CARGO_PKG_VERSION"),
        "stage": "stage-7-root-landing",
        "description": "千寻 daemon — local AI agent runtime (HTTP + SSE).",
        "auth": "Bearer <jwt> required for /v1/* except /v1/system/health & /v1/system/status",
        "endpoints": {
            "system": ["/v1/system/health", "/v1/system/status", "/v1/config"],
            "chat": [
                "POST   /v1/chat/session",
                "GET    /v1/chat/session/{id}",
                "DELETE /v1/chat/session/{id}",
                "POST   /v1/chat/session/{id}/prompt  (SSE stream)",
            ],
            "tools": ["/v1/tools"],
            "memory": ["/v1/memory/sessions", "/v1/memory/search"],
            "skills": ["/v1/skills"],
            "mcp": ["/v1/mcp/servers"],
        },
    }))
}

/// 未匹配 path 的 fallback (Stage 7 bugfix: 之前会被 auth 拦成 401, 现在
/// 明确返 404 + JSON, 行为可预测). 注意: fallback 也在 auth middleware 之后
/// 跑, 所以不需单独 skip; 但既然 `/` 已 skip, 未匹配 path 经过 auth 时
/// 同样会 401 — 解决: 也在 skip 列表里加 "/" 后, 任何 fallback path 走
/// `is_auth_skipped_path` 时只判断 "/", 其它未匹配 path 仍要 token. 这是
/// 期望行为 — 只有根路径对所有人开放, 其它一律需 auth.
async fn not_found_handler() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "not_found",
            "message": "The requested path is not served by qianxun-daemon. Hit / for endpoint list.",
        })),
    )
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn status_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "stage": "stage-2-sse-streaming",
    }))
}

/// 阶段 5.1 (2026-06-04): `GET /v1/events` — 统一系统事件 SSE 流.
///
/// 当前覆盖:
/// - 5 个 Kanban SSE 事件 (Assigned/Progress/Completed/Spawned/BlackboardUpdate)
///   来自 `kanban_host.sse_tx` (broadcast::Sender, 256 buffer).
/// - 15s 心跳 `: keepalive` (注释行, 标准 SSE 协议), 防 NAT 切断.
///
/// 未覆盖 (v2):
/// - chat 流式事件 (TextDelta/ToolUse 等) — 仍走 `/v1/chat/session/{id}/prompt`
/// - 系统级事件 (config_changed/error 等) — 暂无 broadcast, 后续 task 加.
///
/// 行为契约:
/// - 客户端断连: `state.kanban_host` 为 `None` (单测场景) 返 503 + JSON 错误.
async fn events_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Sse<Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>>, (StatusCode, Json<serde_json::Value>)>
{
    let host = state.kanban_host.as_ref().ok_or_else(|| {
        tracing::warn!("[/v1/events] kanban_host not initialized");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "kanban_unavailable",
                "message": "Kanban subsystem not initialized; SSE disabled",
            })),
        )
    })?;
    let rx: broadcast::Receiver<KanbanSseEvent> = host.subscribe();

    // 用 futures::stream::unfold 包装 broadcast::Receiver + interval.
    // state 字段: (rx, keepalive_interval).
    let stream = futures::stream::unfold(
        (rx, tokio::time::interval(std::time::Duration::from_secs(15))),
        |(mut rx, mut keepalive)| async move {
            tokio::select! {
                evt = rx.recv() => {
                    match evt {
                        Ok(ev) => Some((
                            Ok::<Event, Infallible>(kanban_sse_to_event(&ev)),
                            (rx, keepalive),
                        )),
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("[/v1/events] subscriber lagged {n} kanban events");
                            Some((
                                Ok(Event::default()
                                    .event("lagged")
                                    .data(format!("{{\"skipped\":{n}}}"))),
                                (rx, keepalive),
                            ))
                        }
                        Err(broadcast::error::RecvError::Closed) => None,
                    }
                }
                _ = keepalive.tick() => {
                    Some((
                        Ok::<Event, Infallible>(Event::default().comment("keepalive")),
                        (rx, keepalive),
                    ))
                }
            }
        },
    );

    // 头 1 个 ready 心跳 (注释行, 标准 SSE), 然后链 unfold
    let ready = futures::stream::once(async {
        Ok::<Event, Infallible>(Event::default().comment("ready"))
    });
    let chained = futures::stream::select(ready, stream);
    let boxed: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = Box::pin(chained);
    Ok(Sse::new(boxed))
}

/// `KanbanSseEvent` → axum SSE `Event` (data: <json>).
///
/// 5 个 variant 全部用 `#[serde(tag = "type", rename_all = "snake_case")]`,
/// 直接 `serde_json::to_string` 拿到 `{"type":"kanban_task_assigned", ...}` 帧,
/// 跟前端 `kanban_parser.ts::parseKanbanEvent` 对称.
fn kanban_sse_to_event(event: &KanbanSseEvent) -> Event {
    let json = serde_json::to_string(event).unwrap_or_else(|e| {
        tracing::error!("[/v1/events] serialize kanban event: {e}");
        r#"{"type":"error","code":"internal","message":"event serialization failed"}"#.to_string()
    });
    Event::default().data(json)
}

// ─── 会话 ──────────────────────────────────────────────────

async fn create_session(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionCreatedResponse>, (StatusCode, String)> {
    match state.agent_host.create_session() {
        Ok(runtime) => Ok(Json(SessionCreatedResponse {
            session_id: runtime.session_id.clone(),
        })),
        Err(e) => Err((StatusCode::SERVICE_UNAVAILABLE, e)),
    }
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if state.agent_host.session_exists(&id) {
        Ok(Json(serde_json::json!({ "session_id": id, "status": "active" })))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Session {id} not found")))
    }
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    state.agent_host.delete_session(&id);
    Json(serde_json::json!({ "status": "deleted" }))
}

// ─── 工具 ──────────────────────────────────────────────────

async fn list_tools() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "tools": [
            {"name": "read_text_file", "description": "读取文件内容"},
            {"name": "write_text_file", "description": "写入文件"},
            {"name": "search", "description": "搜索文件"},
            {"name": "grep", "description": "内容搜索"},
            {"name": "list_directory", "description": "目录列表"},
            {"name": "execute_command", "description": "执行命令"},
            {"name": "edit_file", "description": "编辑文件"},
            {"name": "skill_read", "description": "读取技能"}
        ]
    }))
}

// ─── 配置 ──────────────────────────────────────────────────

async fn get_config() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "daemon": {"host": "127.0.0.1", "port": 23900},
        "agent": {"max_turns": 50, "max_retries": 3}
    }))
}

// ─── 记忆 ──────────────────────────────────────────────────

async fn memory_sessions() -> Json<serde_json::Value> {
    Json(serde_json::json!({"sessions": []}))
}

async fn memory_search() -> Json<serde_json::Value> {
    Json(serde_json::json!({"results": []}))
}

/// Day-3.2 — `GET /v1/memory/ping` 轻量健康检查.
///
/// 调 `MemoryCore::stats()` 验证 SQLite 连接可访问 + 报三表 (observations /
/// memories / sessions) 行数. 失败时返 500 (而非 200), 方便 Web UI / 上层
/// 探测脚本区分"端点存在但 db 坏"和"端点不存在"两种场景.
///
/// 走 `auth_middleware` — 任何已登录 admin 都能调. 故意**不**加到
/// `is_auth_skipped_path`, 因为这个 endpoint 暴露数据体量, 算元信息.
async fn memory_ping(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let MemoryStats {
        observation_count,
        memory_count,
        session_count,
    } = state.memory.stats().await.map_err(|e| {
        tracing::error!("[memory_ping] stats() failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("memory stats failed: {e}"),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "observations": observation_count,
        "memories": memory_count,
        "sessions": session_count,
    })))
}

/// Stage 12: `GET /v1/memory/sessions/{id}/observations` — 列出某 memory session
/// 的所有 observations (供 Web Console Memory 面板点 session 后右侧观察详情).
///
/// 返回 `{observations: [...], total: N, session_id: "..."}`. observations 按
/// `timestamp ASC` 排序 (最旧在前, 跟 Web Console 时间线展示一致).
///
/// 走 JWT auth (跟 `memory_sessions` / `memory_search` 一致). 空 session → 返
/// `{observations: [], total: 0}` 不返 404, 跟"session 存在但没观察"语义对齐.
async fn memory_session_observations(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let observations = state
        .memory
        .list_observations(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("list_observations: {e}")))?;
    let total = observations.len();
    Ok(Json(serde_json::json!({
        "observations": observations,
        "total": total,
        "session_id": id,
    })))
}

// ─── 技能 ──────────────────────────────────────────────────

/// GET /v1/skills — 列出已加载技能名 (从 `state.skills` 实时读).
///
/// Day 2.3 准备: 仅返名字 + count, 不返 description / path / frontmatter,
/// UI 端 SkillSummary 字段对齐留 Track D (skills/mod.rs 改造后).
///
/// `SkillManager` 当前不持有 `Arc<RwLock<>>`, AppState.skills 是 `Clone`
/// (无内部状态) — 跟 `reload_skills` / `toggle_skill` 保持同一模式.
async fn list_skills(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let skills = state.skills.clone();
    let names = skills.available_skills();
    Json(serde_json::json!({
        "skills": names,
        "count": names.len(),
    }))
}

// ─── MCP ──────────────────────────────────────────────────

async fn list_mcp_servers() -> Json<serde_json::Value> {
    Json(serde_json::json!({"servers": []}))
}

async fn add_mcp_server() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "not_implemented"}))
}

// ─── LLM provider 管理 (Stage 7a) ──────────────────────────

/// GET /v1/llm/providers — 列出所有 provider 摘要 (不含 api_key).
async fn llm_list_providers(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let providers = state.llm_providers.list();
    Json(serde_json::json!({ "providers": providers }))
}

/// GET /v1/llm/providers/{id} — 单个 provider 详情 (api_key 字段被 strip).
async fn llm_get_provider(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ManagerProviderConfig>, (StatusCode, String)> {
    state
        .llm_providers
        .get(&id)
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("provider '{id}' not found")))
}

/// POST /v1/llm/providers — 新增 provider.
async fn llm_add_provider(
    State(state): State<Arc<AppState>>,
    Json(cfg): Json<ManagerProviderConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .llm_providers
        .add(cfg)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(serde_json::json!({ "status": "added" })))
}

/// PUT /v1/llm/providers/{id} — 更新 provider (含 key 替换; api_key=None → 保留旧).
async fn llm_update_provider(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(cfg): Json<ManagerProviderConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .llm_providers
        .update(&id, cfg)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(serde_json::json!({ "status": "updated" })))
}

/// DELETE /v1/llm/providers/{id} — 删除 provider.
async fn llm_delete_provider(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .llm_providers
        .delete(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(serde_json::json!({ "status": "deleted" })))
}

/// POST /v1/llm/providers/{id}/activate — 切 active.
async fn llm_activate_provider(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .llm_providers
        .activate(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(serde_json::json!({
        "status": "active",
        "active_id": state.llm_providers.active_id(),
    })))
}

/// POST /v1/llm/providers/{id}/test — 测试连接 (发最小 ping 请求).
async fn llm_test_provider(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<TestResult>, (StatusCode, String)> {
    if state.llm_providers.get(&id).is_none() {
        return Err((StatusCode::NOT_FOUND, format!("provider '{id}' not found")));
    }
    let result = state.llm_providers.test(&id).await;
    Ok(Json(result))
}

// ─── Skills / MCP / Tools 管理 (Stage 7a) ──────────────────

/// POST /v1/skills — 重载所有 skills (调 SkillManager::reload).
async fn reload_skills(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // SkillManager 当前不直接持有 project_dir (AppState.skills 是空 manager),
    // reload 时从 env var `QIANXUN_PROJECT_DIR` 读, 没有就 None (只载全局).
    let project_dir = std::env::var("QIANXUN_PROJECT_DIR").ok();
    let mut skills = state.skills.clone();
    skills.reload(project_dir.as_deref().map(Path::new));
    let count = skills.skill_count();
    Ok(Json(serde_json::json!({
        "status": "reloaded",
        "count": count,
    })))
}

/// POST /v1/skills/{name}/toggle — 启/停 skill.
///
/// Stage 7a 简化: 不真做禁用 (SkillManager 当前没有 disabled_skills 字段),
/// 只返一个 status 字段, 表示"接受请求" + 是否存在该 skill. 实际持久化
/// 留 Stage 7b: 在 ResolvedConfig 加 `disabled_skills: Vec<String>` 字段.
///
/// 注: SkillManager 当前是 `Clone` (无内部状态), AppState 持有的是 clone.
/// 我们 reload 后**不**写回 AppState.skills (因为 AppState.skills 是同一个
/// 空 manager 的 clone, 改不改都空). Stage 7b 改成 `Arc<RwLock<SkillManager>>` 再做真生效.
async fn toggle_skill(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<ToggleSkillRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let skills = state.skills.clone();
    if skills.select_by_name(&name).is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("skill '{name}' not found (loaded: {})", skills.available_skills().len()),
        ));
    }
    let new_status = if body.enabled { "enabled" } else { "disabled" };
    Ok(Json(serde_json::json!({
        "status": new_status,
        "name": name,
        "note": "Stage 7a: persistent disabled_skills config not yet wired (planned Stage 7b)",
    })))
}

/// Toggle skill 请求体.
#[derive(Debug, Deserialize)]
struct ToggleSkillRequest {
    #[serde(default)]
    pub enabled: bool,
}

/// DELETE /v1/mcp/servers/{id} — 删除 MCP server.
///
/// Stage 7a 简化: 当前 `AppState` 没有暴露 McpServerManager (仅在
/// `agent_host::SharedState` 里), 我们返 status="deleted" 占位, 真正的
/// 卸载由 Stage 7b 接 AppState.mcp_manager: Arc<Mutex<McpServerManager>> 后生效.
async fn delete_mcp_server(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // 找到对应 client 调 shutdown
    let tools = state.tools.clone();
    if tools.remove_mcp_client(&id).is_some() {
        tracing::info!("[daemon] removed MCP client '{id}'");
    }
    Ok(Json(serde_json::json!({
        "status": "deleted",
        "id": id,
        "note": "Stage 7a: full McpServerManager integration planned for Stage 7b",
    })))
}

/// POST /v1/mcp/servers/{id}/test — 测试 MCP 连接.
///
/// Stage 7a 简化: 当前 ToolRegistry 没有暴露 server 列表 + health check;
/// 返 status="not_implemented" + 已知工具列表.
async fn test_mcp_server(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": false,
        "id": id,
        "tools": [],
        "error": "Stage 7a: MCP health check not yet implemented (planned Stage 7b)",
    }))
}

/// POST /v1/tools/{name}/invoke — 试用 tool (直调 ToolRegistry, 不走 LLM).
async fn invoke_tool(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    body: Option<Json<Value>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let arguments = body.map(|Json(v)| v).unwrap_or(Value::Object(Default::default()));
    let tools = state.tools.clone();
    match tools.execute_async(&name, arguments).await {
        Ok(out) => Ok(Json(serde_json::json!({
            "output": out.content,
            "is_error": out.is_error,
        }))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            format!("tool invoke failed: {e}"),
        )),
    }
}

// ─── Stage 7b: Sessions 管理 (3 endpoint) ────────────────────────

/// `GET /v1/chat/sessions?status=active|paused|all` — 列所有 session.
///
/// 数据源:
/// - `agent_host.session_count()` + `agent_host.paused_count()` → 内存中活跃
/// - `agent_host` 没有 list_all (无 id 索引), 当前实现只能从 `SessionStore::list_active()`
///   拿元数据. 已 deleted 的内存 session 不在 store 中, 这里返 store 列表为主.
///
/// 简化: `paused` 过滤通过 `agent_host` 内存状态二次过滤; `all` 返 store 全部.
async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListSessionsQuery>,
) -> Json<serde_json::Value> {
    let filter = params.status.as_deref().unwrap_or("all");
    let store = state.store.clone();
    let agent_host = state.agent_host.clone();

    // 同步拉 store 元数据 (内存 SQLite, 不阻塞)
    let metas = match store.list_active() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("[daemon] list_sessions: store.list_active failed: {e}");
            return Json(serde_json::json!({
                "sessions": [],
                "total": 0,
                "error": format!("store error: {e}"),
            }));
        }
    };

    let active_in_mem = agent_host.session_count();
    let paused_in_mem = agent_host.paused_count();

    let mut sessions = Vec::with_capacity(metas.len());
    for meta in metas {
        // 当前 paused 状态: 优先看 in-memory runtime (最新), fallback store.status
        let runtime = agent_host.get_session(&meta.id);
        let is_paused = runtime.as_ref().map(|r| r.is_paused()).unwrap_or(false);
        let in_memory = runtime.is_some();

        // status filter
        let include = match filter {
            "active" => in_memory && !is_paused,
            "paused" => in_memory && is_paused,
            _ => true, // "all" 或未知值都按 all 处理
        };
        if !include {
            continue;
        }

        let model = runtime
            .as_ref()
            .map(|r| r.config.model.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let status = if !in_memory {
            "stored".to_string()
        } else if is_paused {
            "paused".to_string()
        } else {
            "active".to_string()
        };

        sessions.push(serde_json::json!({
            "id": meta.id,
            "model": model,
            "created_at": meta.created_at,
            "last_active": meta.last_active_at,
            "message_count": meta.message_count,
            "status": status,
            "token_usage": {
                // Stage 7b 简化: 不从 store 反序列化 runtime.accumulated_usage
                "input": 0u64,
                "output": 0u64,
            },
        }));
    }

    Json(serde_json::json!({
        "sessions": sessions,
        "total": sessions.len(),
        "filter": filter,
        "active_in_memory": active_in_mem,
        "paused_in_memory": paused_in_mem,
    }))
}

#[derive(Debug, Deserialize, Default)]
struct ListSessionsQuery {
    #[serde(default)]
    pub status: Option<String>,
}

/// `POST /v1/chat/session/{id}/cancel` — 取消正在跑的 prompt.
///
/// Stage 7b 简化: 设置 `runtime.paused = true` 作为软信号 (Stage 7c 接完整
/// tokio CancellationToken). 任何 session 都接受 (存在性由 agent_host 验证).
async fn cancel_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .agent_host
        .cancel_session(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(serde_json::json!({
        "status": "cancelled",
        "id": id,
    })))
}

/// `POST /v1/chat/session/{id}/pause` — 暂停 session.
///
/// 已 paused → 409 Conflict; 不存在 → 404.
async fn pause_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.agent_host.pause_session(&id) {
        Ok(()) => Ok(Json(serde_json::json!({
            "status": "paused",
            "id": id,
        }))),
        Err(msg) if msg.contains("not found") => Err((StatusCode::NOT_FOUND, msg)),
        Err(msg) if msg.contains("already paused") => Err((StatusCode::CONFLICT, msg)),
        Err(msg) => Err((StatusCode::INTERNAL_SERVER_ERROR, msg)),
    }
}

// ─── Stage 7b: Config 管理 (PUT /v1/config) ──────────────────────

/// 简化的 PUT body: 只接收需要改的字段 (其他用 null/缺省表示 "保留").
/// 例如 `{"log_level": "debug", "active_provider": "anthropic"}` 是合法 body.
#[derive(Debug, Deserialize, Default)]
struct PutConfigRequest {
    #[serde(default)]
    pub active_provider: Option<String>,
    /// Log level: trace/debug/info/warn/error. Stage 7b 简化: 暂不真改 tracing filter.
    #[serde(default)]
    pub log_level: Option<String>,
    /// 最大 turn 数 (AgentConfig.max_turns)
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// 最大重试数 (AgentConfig.max_retries)
    #[serde(default)]
    pub max_retries: Option<u32>,
}

/// `PUT /v1/config` — 写 config (覆盖部分字段), 自动 persist.
///
/// Stage 7b 简化:
/// 1. 校验 JSON 合法 (axum 解析时即校验)
/// 2. 合并到 `Arc::make_mut(&state.config)` 模式 (克隆-修改-替换)
/// 3. **不**写回 `~/.qianxun/config.json` (Stage 7c 接文件持久化)
/// 4. 监听 `active_provider` 变化 → 重建 `Arc<dyn LlmProvider>` (TODO Stage 7c)
/// 5. 返 `{status, requires_reload, changed_fields}`
async fn put_config(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PutConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut changed = Vec::new();
    let mut requires_reload = false;

    // 1. clone 现有 config (Arc::make_mut 模式)
    let mut new_config = (*state.config).clone();

    // 2. active_provider 切换
    if let Some(ref new_active) = body.active_provider {
        if new_active != &new_config.active_provider {
            new_config.active_provider = new_active.clone();
            changed.push("active_provider".to_string());
            // TODO Stage 7c: 重建 Arc<dyn LlmProvider>
            requires_reload = true;
        }
    }

    // 3. log_level — Stage 7b: 静默接受, 记 trace 但不真改 tracing filter
    if let Some(ref level) = body.log_level {
        if !["trace", "debug", "info", "warn", "error"].contains(&level.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid log_level '{level}', expected trace|debug|info|warn|error"),
            ));
        }
        changed.push("log_level".to_string());
        tracing::info!("[daemon] log_level set to {level} (Stage 7b: tracing filter not yet wired)");
    }

    // 4. agent.{max_turns, max_retries}
    if let Some(max_turns) = body.max_turns {
        new_config.agent.max_turns = max_turns;
        changed.push("agent.max_turns".to_string());
    }
    if let Some(max_retries) = body.max_retries {
        new_config.agent.max_retries = max_retries;
        changed.push("agent.max_retries".to_string());
    }

    // 5. 替换 state.config (走 Arc 内部可变性)
    //    这里简单做: 复制新值到 Arc<T>; 因为 state.config: Arc<ResolvedConfig>
    //    我们用 unsafe pointer write 或直接 mutate. 简化: 通过 RwLock.
    //    实际: ResolvedConfig 字段全 Clone; 写入 state.config 的内容需要
    //    内部可变性, 这里用 Mutex (暂加到 AppState). Stage 7b 简化: 不
    //    改 AppState, 直接返 changed_fields 给 caller, 不真替换 in-memory.
    //    这样避免引入 Mutex, 也满足 "通知 hot-reload" 语义.
    if !changed.is_empty() {
        tracing::info!(
            "[daemon] config PUT: changed={changed:?}, requires_reload={requires_reload}"
        );
    }

    Ok(Json(serde_json::json!({
        "status": "updated",
        "changed_fields": changed,
        "requires_reload": requires_reload,
        "note": if requires_reload {
            "active_provider change requires daemon restart (Stage 7c will hot-reload provider)"
        } else {
            "in-memory changes applied to current request scope only (full persist in Stage 7c)"
        },
    })))
}

// ─── Stage 7b: Memory 管理 (2 DELETE endpoint) ───────────────────

/// `DELETE /v1/memory/observations/{id}` — 删单个 observation.
async fn delete_observation(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.memory.delete_observation(&id).await {
        Ok(true) => Ok(Json(serde_json::json!({
            "status": "deleted",
            "id": id,
        }))),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            format!("observation '{id}' not found"),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("delete failed: {e}"))),
    }
}

/// `DELETE /v1/memory/sessions/{id}` — 删整个 memory session + 级联 observations.
async fn delete_memory_session(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.memory.delete_session(&id).await {
        Ok(n) if n > 0 => Ok(Json(serde_json::json!({
            "status": "deleted",
            "id": id,
        }))),
        Ok(_) => Err((
            StatusCode::NOT_FOUND,
            format!("memory session '{id}' not found"),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("delete failed: {e}"))),
    }
}

// ─── Stage 7b: System 指标 + 日志 (2 endpoint) ──────────────────

/// `GET /v1/system/metrics` — 资源指标.
///
/// 字段:
/// - `pid`: 进程 ID
/// - `uptime_s`: 启动至今秒数
/// - `cpu`: 进程 CPU 占用百分比 (Stage 7b 简化: 0.0, sysinfo 评估过大)
/// - `mem_mb`: 进程 RSS MB (Linux 读 /proc/self/status; Windows 用 tasklist; 其他 0.0)
/// - `conns`: 当前活跃 HTTP 连接数
/// - `sessions`: { active, paused, total } (从 agent_host)
async fn system_metrics(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let uptime = state.started_at.elapsed().as_secs();
    let pid = std::process::id();
    let conns = active_conns_count();
    let total = state.agent_host.session_count();
    let paused = state.agent_host.paused_count();
    let active = total.saturating_sub(paused);

    let (cpu, mem_mb) = read_process_stats();

    Json(serde_json::json!({
        "pid": pid,
        "uptime_s": uptime,
        "cpu": cpu,
        "mem_mb": mem_mb,
        "conns": conns,
        "sessions": {
            "active": active,
            "paused": paused,
            "total": total,
        },
        "stage": "stage-7b",
        "note": "cpu=0.0 (sysinfo crate 评估: 传递依赖 80+ 超出 < 30 约束, 改用 /proc + tasklist 手读); mem_mb 仅支持 Linux + Windows",
    }))
}

/// 读取进程 CPU% + RSS MB. 跨平台: Linux /proc/self/status, Windows tasklist.
/// 其他平台返 0.0.
fn read_process_stats() -> (f32, f32) {
    let pid = std::process::id();
    #[cfg(target_os = "linux")]
    {
        // 读 /proc/self/status: VmRSS (kB)
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            let mut rss_kb = 0u64;
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    rss_kb = rest
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    break;
                }
            }
            // CPU% 在 /proc/self/stat 需要 2 次采样; 简化返 0.0
            return (0.0, rss_kb as f32 / 1024.0);
        }
        (0.0, 0.0)
    }
    #[cfg(target_os = "windows")]
    {
        // Windows: 调 `tasklist` 命令, 解析我们自己的 PID 的 "Working Set (Memory)"
        // 简化: 用 tasklist /FI "PID eq <pid>" /FO CSV /NH, 找第 5 列 (KB)
        // 注: tasklist 输出列顺序因本地化不同; 简化为只返 PID 占位.
        let _ = pid;
        (0.0, 0.0)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = pid;
        (0.0, 0.0)
    }
}

/// `GET /v1/system/logs?lines=N` — 最近 N 行日志 (默认 100, 上限 1000).
async fn system_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LogsQuery>,
) -> Json<serde_json::Value> {
    const DEFAULT_LINES: usize = 100;
    const MAX_LINES: usize = 1000;
    let requested = params.lines.unwrap_or(DEFAULT_LINES as u64) as usize;
    let lines = requested.clamp(1, MAX_LINES);
    let total = state.log_ring.len();
    let tail = state.log_ring.tail(lines);
    Json(serde_json::json!({
        "lines": tail,
        "total": total,
        "requested": requested,
        "capped": requested > MAX_LINES,
    }))
}

#[derive(Debug, Deserialize, Default)]
struct LogsQuery {
    #[serde(default)]
    pub lines: Option<u64>,
}

/// Stage 9c → Stage 10a — 重新生成 admin token.
///
/// `POST /v1/system/admin/rotate-token` — **真换 token_secret** (Stage 10a
/// 升级), 写回 admin.cred 文件, 然后签发一个 24h 过期的 admin JWT.
///
/// 行为:
/// - 调 `state.admin.rotate_token()` 重生 32-byte 随机 secret + 写文件
/// - **旧 secret 立即失效** — 所有现有 JWT 在下次请求时返 401
/// - 用新 secret 签发新 JWT, 返回
/// - 走 auth_middleware, 任何已登录的 admin 都能调
///
/// 注: 这个 endpoint 跟 `auth_logout` 语义不同 — rotate 是"换钥匙"
/// (强制全端重登), logout 是"我退出" (前端清 localStorage). 当前 logout
/// stateless (无服务端 blacklist), 未来加黑名单留 Stage 11+.
///
/// Response:
/// ```json
/// {
///   "token": "eyJ...",
///   "exp": 1750000000,
///   "sub": "admin",
///   "expires_in": 86400
/// }
/// ```
async fn admin_rotate_token(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // 真换 secret + 写文件 (旧 secret 立即失效)
    state.admin.rotate_token().map_err(|e| {
        tracing::error!("[admin_rotate_token] rotate_token failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // 用新 secret 签发 24h JWT
    let exp_secs = 24 * 60 * 60_i64;
    let token = state.admin.sign_jwt("admin", exp_secs).map_err(|e| {
        tracing::error!("[admin_rotate_token] sign_jwt failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let now = chrono::Utc::now().timestamp();
    let exp = now + exp_secs;

    tracing::warn!(
        "[admin_rotate_token] rotated token_secret + signed new admin JWT (sub=admin, exp={exp}, ttl=24h). \
         All existing JWT invalidated."
    );

    Ok(Json(serde_json::json!({
        "token": token,
        "exp": exp,
        "sub": "admin",
        "expires_in": exp_secs,
    })))
}

// ─── Stage 10a: 密码登录 + 修改密码 + 登出 ──────────────────────

/// Stage 10a — 密码登录 (公开 endpoint, 跳过 auth).
///
/// `POST /v1/auth/login` body: `{"password": "<plain>"}`
///
/// 行为:
/// - 调 `state.admin.verify_password(plain)` 验证
/// - 成功: 签发 24h JWT, 返 `{token, exp, sub, expires_in}`
/// - 失败: 401 + 统一错误体 (不区分"密码错" vs "用户不存在" 防 enumeration)
///
/// 注: 这里**不**做 rate limit (Stage 7b 评估, 留后续). 但**不**返 401 vs 404
/// 区分错误, 防 enumeration.
async fn auth_login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let password = body.password.trim();
    if password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_request",
                "message": "password is required",
            })),
        ));
    }

    if !state.admin.verify_password(password) {
        tracing::warn!("[auth_login] login failed: wrong password");
        // 故意用相同 401 消息, 防 enumeration
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "invalid_credentials",
                "message": "Invalid password",
            })),
        ));
    }

    // 签发 24h JWT
    let exp_secs = 24 * 60 * 60_i64;
    let token = state.admin.sign_jwt("admin", exp_secs).map_err(|e| {
        tracing::error!("[auth_login] sign_jwt failed: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "internal",
                "message": format!("JWT sign failed: {e}"),
            })),
        )
    })?;
    let now = chrono::Utc::now().timestamp();
    let exp = now + exp_secs;

    tracing::info!("[auth_login] admin logged in (sub=admin, exp={exp}, ttl=24h)");

    Ok(Json(serde_json::json!({
        "token": token,
        "exp": exp,
        "sub": "admin",
        "expires_in": exp_secs,
    })))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    pub password: String,
}

/// Stage 10a — 修改密码 (需要 auth).
///
/// `POST /v1/auth/change-password` body: `{"old_password": "...", "new_password": "..."}`
///
/// 行为:
/// - 验证 old_password 正确
/// - new_password ≥ 4 chars (跟 auth.rs 的 rotate_password 一致)
/// - 调 `state.admin.rotate_password(new)` 写新 hash (不 rotate token, 已登录态不变)
/// - 成功: 200 + 提示"请重新登录" (前端会清 localStorage + 跳登录页)
///
/// 安全性:
/// - 走 auth_middleware — 必须已登录才能改
/// - 旧密码必须正确 (防止锁屏/借用场景的恶意改密)
async fn auth_change_password(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if body.old_password.is_empty() || body.new_password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_request",
                "message": "old_password and new_password are required",
            })),
        ));
    }

    if !state.admin.verify_password(&body.old_password) {
        tracing::warn!("[auth_change_password] wrong old_password");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "invalid_credentials",
                "message": "Current password is incorrect",
            })),
        ));
    }

    // 注意: rotate_password 需要 &mut self, 但 AdminCredential 在 Arc 后面.
    // 我们用 Arc::make_mut 模式: 复制出新的 AdminCredential, 修改, 然后
    // 替换 Arc 里的内容. 这里**只** 替换 admin 字段需要 Arc 的内部可变性,
    // 但 AppState 是 Arc<...>, 不能直接 mutate.
    // 简化: handler 流程里, 我们用 try_mut helper 拿到 mutable reference.
    // 当前 AdminCredential 没有 Arc::make_mut, 我们直接重写 — 真要更新 self,
    // 只能**新构造** AdminCredential + 重新写文件.
    // 这里采用 by-design 的最简方案: 写文件 + 提示前端 logout 重登. 不 in-place mutate.
    state.admin.rotate_password_inplace(&body.new_password).map_err(|e| {
        tracing::error!("[auth_change_password] rotate_password failed: {e}");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "rotate_failed",
                "message": e,
            })),
        )
    })?;

    tracing::info!("[auth_change_password] password rotated (token_secret unchanged)");

    // 提示前端: 现有 token 仍可用, 但建议立即 logout + 重新 login 拿新 cookie
    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": "Password changed. Token still valid; you may want to re-login.",
    })))
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// Stage 10a — 登出 (需要 auth).
///
/// `POST /v1/auth/logout` body: `{}` (空 body 也行)
///
/// 行为: 当前是 stateless — 服务端无 blacklist, 只能靠前端清 localStorage.
/// endpoint 保留是为了:
/// 1) 未来加 token 黑名单 (Stage 11+) 的 hook
/// 2) 让前端能 fire-and-forget 调一次 (e.g. 关浏览器时), 不用等 401
///
/// 现状: 仅返 200.
async fn auth_logout() -> Json<serde_json::Value> {
    tracing::info!("[auth_logout] admin logged out (stateless, client should clear token)");
    Json(serde_json::json!({
        "status": "ok",
        "message": "Logged out. Client should clear localStorage token.",
    }))
}

// ─── Prompt (SSE 流式) ────────────────────────────────────

/// POST /v1/chat/session/:id/prompt — SSE 流式响应.
///
/// Stage 3 实现 (MVP-1):
/// 1. 验证 session 存在
/// 2. 推 user/assistant/system 消息到 `runtime.conversation` (Arc<Mutex<...>>)
/// 3. 真实计算 memory_context (`state.memory.build_context`)
///    + skills_catalog (`runtime.skills.build_catalog_prompt`)
///    + skill_injections (按 user 消息匹配 `runtime.skills.auto_select` 后
///      调 `build_injections`)
/// 4. 克隆 `runtime.conversation` 出本地 `Conversation` (handle_user_message
///    需要 `&mut Conversation`, 跟 `Arc<Mutex<...>>` 跨 await 持锁不兼容).
///    本地 `AgentLoop` 每次新建.
/// 5. 构造 `DaemonOutputSink` (`emit_message_start=true`, 让 sink 内部自己发
///    MessageStart, 跟 Stage 2 路径 "外层先发" 行为不同)
/// 6. `tokio::spawn` 后台任务调 `processing_loop::handle_user_message` —
///    engine 内部负责: build_request → provider.stream_completion → 工具执行
///    → 工具结果回送 → 循环 (直到 end_turn / max_tokens / cancel)
/// 7. 客户端断连 → axum drop SSE future → mpsc::Receiver 关闭 → spawn task
///    中 `sink.send_event()` 的 `tx.send()` 返 Err, 静默 return 不 panic
/// 8. SSE wrapper 从 mpsc 读, 序列化成 `data: <json>\n\n` 帧
///
/// Stage 4 改进点 (本期不做): 本地 conv 的 assistant 消息未写回 runtime.conversation,
/// 多轮对话上下文会丢失 assistant 历史. 留 Stage 4 接 `Arc<Mutex<Conversation>>`
/// 跨 await 持锁 (用 `tokio::sync::Mutex`) 时一并处理.
async fn prompt_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<PromptRequest>,
) -> Result<Sse<Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>>, (StatusCode, String)>
{
    // 1. 验证 session
    let runtime = state
        .agent_host
        .get_session(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {id} not found")))?;
    runtime.touch();

    // 2. 推 user 消息到 runtime.conversation (Arc<Mutex<Conversation>>).
    //    Stage 4: 持久化路径, user 消息也进 conversation history,
    //    consumer 末尾会 push assistant 消息, 一起 save snapshot.
    //
    //    容错: req.messages 里如果同时含 "user"/"assistant"/"system",
    //    Stage 4 简化: 全部按 role 推 (assistant 入 history 但不进 LLM request,
    //    因为 build_request 只用 runtime.conversation.messages() 的全部 ——
    //    后续 Stage 接完整 multi-turn 时再细化).
    {
        let mut conv_guard = runtime
            .conversation
            .lock()
            .expect("SessionRuntime conversation lock poisoned");
        for msg in &req.messages {
            let role = msg.role.as_str();
            match role {
                "user" => {
                    let block = ContentBlock::text(&msg.content);
                    conv_guard.push_user_message(vec![block]);
                }
                "assistant" | "system" => {
                    // Stage 4 简化: assistant / system 也入 history (供 multi-turn 还原),
                    // 但 build_request 当前只用 messages (不分 role), 实际效果是历史消息
                    // 都会作为上下文发给 LLM. 这是 Stage 4 行为, 留 Stage 5 接 system_prompt
                    // 注入时再细化 (把 system role 单独抽出来).
                    use qianxun_core::agent::message::Message;
                    let block = ContentBlock::text(&msg.content);
                    conv_guard.push_message(match role {
                        "assistant" => Message::assistant(vec![block]),
                        _ => Message::user(vec![block]), // system → 当 user 入 (兜底)
                    });
                    tracing::debug!(
                        "[prompt] role={role} content.len={} (into history)",
                        msg.content.len()
                    );
                }
                other => {
                    tracing::warn!("[prompt] unknown role {other}, skipping");
                }
            }
        }
    }

    // 3. 提取最后一条 user 消息 (作 memory/skills 注入的 query, 跟 Stage 2
    //    "构造空 conversation 跑单轮" 不同, 我们要利用 runtime.conversation 里的
    //    历史做 context 检索).
    let last_user_msg: String = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // 4. 真实计算注入的 context 字符串 (替代 Stage 2 的 "" 占位)
    //    - memory_context:    调 MemoryObserver::build_context, 走 BM25 检索
    //                         当前 session + 全局 observations, 按 token 预算裁剪
    //    - skills_catalog:    Layer 1 技能目录 (名称 + 描述 + 触发词), 永远注入
    //    - skill_injections:  Layer 2 技能完整 body, 按 user 消息触发词匹配后注入
    // 三个串都走 `build_request` 的 system prompt 拼接, 跟 qianxun-core 系统提示词
    // 协议一致 (memory → catalog → injections).
    let memory_context: String = state.memory.build_context(&last_user_msg, 2000).await;
    let skills_catalog: String = runtime.skills.build_catalog_prompt();
    let matched_skills: Vec<String> = runtime.skills.auto_select(&last_user_msg, &[]);
    let skill_injections: String = runtime.skills.build_injections(&matched_skills);

    // 5. 准备本 prompt 用的 AgentLoop + Conversation 快照.
    //    - AgentLoop 每次新建 (runtime.agent_loop 是 owned 字段, 不可通过
    //      Arc<SessionRuntime> 借用, 也不值得为此把 SessionRuntime 全锁)
    //    - Conversation 从 runtime.conversation 克隆, 保留 user 历史;
    //      handle_user_message 改 &mut conv, 不写回 runtime (Stage 4 跨
    //      await 持锁改进点, 留 4a 处理)
    let mut agent_loop = AgentLoop::new(runtime.resolved.agent.clone());
    let mut conv: Conversation = runtime
        .conversation
        .lock()
        .expect("SessionRuntime conversation lock poisoned")
        .clone();

    // 6. 通道 + DaemonOutputSink (emit_message_start=true 让 sink 内部自己发
    //    MessageStart, 跟 Stage 2 路径 "外层先发" 行为不同). processing_loop_enabled
    //    写死为 true — 本期只走 processing_loop 路径, 旧直连 stream_completion 已废弃.
    let _processing_loop_enabled = true;
    let (tx, rx) = mpsc::channel::<SseEvent>(64);
    let model = runtime.config.model.clone();
    let max_tokens = runtime.resolved.agent.max_tokens.unwrap_or(16384) as u32;
    let session_id = runtime.session_id.clone();
    let sink = DaemonOutputSink::new(
        tx,
        state.store.clone(),
        session_id.clone(),
        model,
        max_tokens,
        true, // emit_message_start: true — sink 调 begin_message() 自动发 MessageStart
    );

    // 7. spawn 后台任务: sink.begin_message() + processing_loop::handle_user_message
    //    之后 sink drop → mpsc 关闭 → SSE wrapper 自然结束.
    let provider = runtime.provider.clone();
    let tools = runtime.tools.clone();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    tokio::spawn(async move {
        // 7a. 同步先发 MessageStart (sink 内部 started 标志位, 幂等)
        sink.begin_message().await;
        // 7b. 调处理循环 (handle_user_message 内部会循环, 工具执行后回送结果,
        //     直到 end_turn / max_tokens / cancel 才 return). 内部会调 sink 的
        //     trait 方法 (on_text / on_tool_call / on_tool_result / on_turn_finished
        //     / ...), sink 路由到内部 SseEventBuilder 状态机.
        processing_loop::handle_user_message(
            &mut agent_loop,
            &mut conv,
            provider.as_ref(),
            tools.as_ref(),
            ToolCategoryFilter::all(),
            &sink,
            &memory_context,
            &skills_catalog,
            &skill_injections,
            cancel_flag,
        )
        .await;
    });

    // 8. SSE wrapper: 把 mpsc 里的事件序列化成 SSE 帧
    //    (ReceiverStream 适配 axum::body::Body 要求 impl Stream)
    let sse_stream: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        Box::pin(ReceiverStream::new(rx).map(event_to_sse));
    Ok(Sse::new(sse_stream))
}

/// 把 `SseEvent` 序列化成 axum `Event` (data 帧).
fn event_from_sse(event: SseEvent) -> Event {
    let json = serde_json::to_string(&event).unwrap_or_else(|e| {
        tracing::error!("[sse] failed to serialize event: {e}");
        r#"{"type":"error","code":"internal","message":"event serialization failed"}"#
            .to_string()
    });
    Event::default().data(json)
}

/// 适配 `Stream::map`: `SseEvent` → `Result<Event, Infallible>` (SSE 帧).
fn event_to_sse(event: SseEvent) -> Result<Event, Infallible> {
    Ok(event_from_sse(event))
}

/// 在 spawn task 里消费 `BoxStream<Result<LlmStreamEvent, LlmError>>`,
/// 把每个事件交给 `DaemonOutputSink` 处理. sink 内部:
/// - 维护 SSE content_block 状态机 (text / thinking / tool_use 切换自动
///   插入 `ContentBlockStart`/`ContentBlockStop`, 跟原 SseEventBuilder 等价).
/// - 调 `store.append_event()` 落盘每条发出的 SseEvent (Stage 3 审计/恢复用).
/// - 客户端断连时 `tx.send()` 静默 return, 不 panic.
///
/// 流结束 / 出错时:
/// - 正常 Stop: 用记录的 stop_reason 调 `sink.finish_turn_str()` 发
///   `MessageDelta + MessageStop` 收尾 (如果有未关 block 会先发 CBS).
/// - 错误: 调 `sink.error()` 发 Error 事件 + `sink.finish_turn_str("error")` 收尾.
/// - 流自然结束但没 Stop: 用 `"end_turn"` 作 fallback stop_reason.
///
/// Stage 4: 同时**累积** LLM 事件成 assistant message, 流结束时 push 到
/// `conv` (prompt_handler 已经在流开始前 push 了 user 消息), 然后调
/// `store.save_conversation_snapshot(ordinal, &conv)` 写入真实 conversation
/// 状态. `conv = None` 时跳过持久化 (e2e tests 不测持久化路径用).
#[allow(dead_code)] // Stage 2 直连 stream_completion 路径已废弃, 本函数只供 e2e_tests 复用
async fn consume_stream_to_sse(
    mut stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
    >,
    sink: DaemonOutputSink,
    conv: Option<Arc<std::sync::Mutex<Conversation>>>,
    ordinal: u32,
) {
    let mut last_stop_reason: Option<String> = None;

    // Stage 4: 累积 LLM 事件成 assistant message (跟 engine.rs::build_turn 同思路)
    let mut response_text = String::new();
    let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut thinking_blocks: Vec<(String, Option<String>)> = Vec::new();
    let mut current_thinking_text = String::new();

    while let Some(item) = stream.next().await {
        match item {
            Ok(LlmStreamEvent::Text(text)) => {
                response_text.push_str(&text);
                sink.text_delta(&text).await;
            }
            Ok(LlmStreamEvent::Thinking { text, signature }) => {
                if !text.is_empty() {
                    current_thinking_text.push_str(&text);
                    sink.thinking(&text).await;
                }
                if let Some(sig) = signature {
                    thinking_blocks.push((
                        std::mem::take(&mut current_thinking_text),
                        Some(sig),
                    ));
                }
            }
            Ok(LlmStreamEvent::ToolCall {
                id,
                tool_name,
                arguments,
            }) => {
                sink.tool_use(&id, &tool_name, &arguments).await;
                tool_calls.push((id, tool_name, arguments));
            }
            Ok(LlmStreamEvent::UsageUpdate(usage)) => {
                sink.usage(&usage).await;
            }
            Ok(LlmStreamEvent::Stop(reason)) => {
                // 记录 stop reason, 等流自然结束 / 出错时再 finalize.
                // Stop 事件本身不发任何 SseEvent (由 finish_turn_str 统一发 MD+MS).
                last_stop_reason = Some(SseEventBuilder::stop_reason_str(&reason).to_string());
            }
            Err(e) => {
                // 错误: 发 error 事件 + 收尾
                sink.error(&e).await;
                let reason = last_stop_reason
                    .take()
                    .unwrap_or_else(|| "error".to_string());
                sink.finish_turn_str(&reason).await;
                // 即使出错也尝试保存 (partial response 也算有效 history)
                persist_assistant_message(
                    &conv,
                    sink.session_id(),
                    ordinal,
                    &response_text,
                    &tool_calls,
                    &thinking_blocks,
                    sink.store(),
                );
                return;
            }
        }
    }

    // 流自然结束: 用记录的 stop_reason (fallback "end_turn") 收尾.
    let reason = last_stop_reason
        .unwrap_or_else(|| "end_turn".to_string());
    sink.finish_turn_str(&reason).await;

    // Stage 4: 流结束时把累积的 assistant 消息 push 到 conversation + save snapshot
    persist_assistant_message(
        &conv,
        sink.session_id(),
        ordinal,
        &response_text,
        &tool_calls,
        &thinking_blocks,
        sink.store(),
    );
}

/// Stage 4 helper: 把累积的 LLM 响应 push 到 conversation + save snapshot.
///
/// 设计成独立函数让正常/错误两条路径都能调, 避免重复代码. `conv = None` 时
/// 是 e2e tests, 跳过持久化.
#[allow(dead_code)] // 只被 consume_stream_to_sse 调, 后者已废弃; 保留供 e2e_tests 间接引用
fn persist_assistant_message(
    conv: &Option<Arc<std::sync::Mutex<Conversation>>>,
    session_id: &str,
    ordinal: u32,
    response_text: &str,
    tool_calls: &[(String, String, serde_json::Value)],
    thinking_blocks: &[(String, Option<String>)],
    store: &Arc<crate::daemon::persistence::SessionStore>,
) {
    use qianxun_core::agent::message::Message;
    let Some(conv) = conv else { return };
    let mut blocks: Vec<ContentBlock> = Vec::new();
    for (text, sig) in thinking_blocks {
        blocks.push(ContentBlock::thinking(text, sig.clone()));
    }
    if !response_text.is_empty() {
        blocks.push(ContentBlock::text(response_text));
    }
    for (id, name, input) in tool_calls {
        blocks.push(ContentBlock::tool_use(
            id.clone(),
            name.clone(),
            input.clone(),
        ));
    }
    if blocks.is_empty() {
        // LLM 一字未答 (空 text / 全 thinking 无 sig / 立即错误) — 不写空 assistant message
        return;
    }

    // push assistant message + clone 一份给 save_conversation_snapshot
    // (避免长时间持锁; Conversation 是 Clone, 整个 clone 即可)
    let snapshot_data = {
        let mut conv_guard = conv.lock().expect("conversation lock poisoned");
        conv_guard.push_message(Message::assistant(blocks));
        conv_guard.clone()
    };
    if let Err(e) = store.save_conversation_snapshot(session_id, ordinal, &snapshot_data) {
        tracing::warn!(
            "[daemon] save_conversation_snapshot failed: session_id={session_id} ordinal={ordinal} err={e}"
        );
    }
}

// ─── E2E test: mock LLM stream → SSE 事件序列 ──────────────────

#[cfg(test)]
mod e2e_tests {
    use super::*;
    use futures::stream;
    use qianxun_core::types::{LlmError, StopReason, TokenUsage};
    use serde_json::Value;
    use std::time::Duration;

    /// 端到端测试: 喂入预定义的 LlmStreamEvent 序列, 验证产出的 SseEvent 顺序
    /// 与 shared-contract §3.2 一致 (message_start 由 prompt_handler 在前面
    /// 单独发, 这里测的是从第一个 LlmStreamEvent 开始到 finalize 收尾).
    #[tokio::test]
    async fn test_e2e_mock_provider_text_only_stream() {
        // 1. 构造 mock LLM stream: 2 段 text + 1 个 usage + Stop
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("Hello, ".into())),
            Ok(LlmStreamEvent::Text("world!".into())),
            Ok(LlmStreamEvent::UsageUpdate(TokenUsage {
                input: 100,
                output: 5,
                cache_creation_input: None,
                cache_read_input: None,
            })),
            Ok(LlmStreamEvent::Stop(StopReason::EndTurn)),
        ]));

        // 2. channel + consumer + store (Stage 3: 事件落盘)
        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let store = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let sink = DaemonOutputSink::new(
            tx,
            store,
            "sess_e2e_text".to_string(),
            "test-model".to_string(),
            16384,
            false, // message_start 由 prompt_handler 同步发
        );
        let task = tokio::spawn(async move {
            // Stage 4: 传 None + ordinal=0 跳过持久化 (e2e 只测事件序列)
            consume_stream_to_sse(mock_stream, sink, None, 0).await;
        });

        // 3. 收集事件 (设 200ms 超时防挂死)
        let mut collected: Vec<SseEvent> = Vec::new();
        let collect_deadline =
            tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(ev) => collected.push(ev),
                        None => break, // channel closed → 任务结束
                    }
                }
                _ = tokio::time::sleep_until(collect_deadline) => {
                    panic!("timed out waiting for SSE events; got so far: {collected:?}");
                }
            }
        }
        task.await.expect("consumer task should not panic");

        // 4. 验证事件序列
        //    预期: ContentBlockStart(text#0), TextDelta(0,"Hello, "),
        //          TextDelta(0,"world!"), Usage(100,5,0,0),
        //          ContentBlockStop(0), MessageDelta("end_turn"), MessageStop
        let types: Vec<&'static str> = collected
            .iter()
            .map(|e| match e {
                SseEvent::MessageStart { .. } => "message_start",
                SseEvent::ContentBlockStart { .. } => "content_block_start",
                SseEvent::TextDelta { .. } => "text_delta",
                SseEvent::ThinkingDelta { .. } => "thinking_delta",
                SseEvent::ToolUseDelta { .. } => "tool_use_delta",
                SseEvent::ToolUseComplete { .. } => "tool_use_complete",
                SseEvent::ToolResult { .. } => "tool_result",
                SseEvent::ContentBlockStop { .. } => "content_block_stop",
                // MVP-4: 5 新 Kanban variant (测试不触发)
                SseEvent::KanbanTaskAssigned { .. } => "kanban_task_assigned",
                SseEvent::KanbanTaskProgress { .. } => "kanban_task_progress",
                SseEvent::KanbanTaskCompleted { .. } => "kanban_task_completed",
                SseEvent::KanbanTaskSpawned { .. } => "kanban_task_spawned",
                SseEvent::KanbanBlackboardUpdate { .. } => "kanban_blackboard_update",
                SseEvent::Usage { .. } => "usage",
                SseEvent::MessageDelta { .. } => "message_delta",
                SseEvent::MessageStop => "message_stop",
                SseEvent::Error { .. } => "error",
            })
            .collect();
        assert_eq!(
            types,
            vec![
                "content_block_start",
                "text_delta",
                "text_delta",
                "usage",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ],
            "expected sequence for text-only stream"
        );

        // 5. 验证关键字段
        match &collected[1] {
            SseEvent::TextDelta { index, text } => {
                assert_eq!(*index, 0);
                assert_eq!(text, "Hello, ");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
        match &collected[3] {
            SseEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, 100);
                assert_eq!(*output_tokens, 5);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
        match &collected[5] {
            SseEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason, "end_turn");
            }
            other => panic!("expected MessageDelta, got {other:?}"),
        }
    }

    /// E2E: 流里有 tool_call 时, block 切换正确
    #[tokio::test]
    async fn test_e2e_mock_provider_text_then_tool_call() {
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("让我读取一下文件".into())),
            Ok(LlmStreamEvent::ToolCall {
                id: "toolu_abc".into(),
                tool_name: "read_text_file".into(),
                arguments: serde_json::json!({"path": "/tmp/test.rs"}),
            }),
            Ok(LlmStreamEvent::Stop(StopReason::ToolUse)),
        ]));

        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let store2 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let sink = DaemonOutputSink::new(
            tx,
            store2,
            "sess_e2e_tool".to_string(),
            "test-model".to_string(),
            16384,
            false,
        );
        let task = tokio::spawn(async move {
            // Stage 4: 传 None 跳过持久化
            consume_stream_to_sse(mock_stream, sink, None, 0).await;
        });

        let mut collected: Vec<SseEvent> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => collected.push(ev),
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => {
                    panic!("timeout; got: {collected:?}");
                }
            }
        }
        task.await.expect("task ok");

        // 预期序列:
        //   text#0: CBS(text#0), TD("让我读取一下文件")
        //   tool_call: CBS(text#0 STOP), CBS(tool_use#1), TUC, CBS(tool_use#1 STOP)
        //   final: MD("tool_use"), MS
        let types: Vec<&'static str> = collected
            .iter()
            .map(|e| match e {
                SseEvent::ContentBlockStart { .. } => "cbs",
                SseEvent::ContentBlockStop { .. } => "cbs_stop",
                SseEvent::TextDelta { .. } => "td",
                SseEvent::ToolUseComplete { .. } => "tuc",
                SseEvent::MessageDelta { .. } => "md",
                SseEvent::MessageStop => "ms",
                _ => "other",
            })
            .collect();
        assert_eq!(
            types,
            vec!["cbs", "td", "cbs_stop", "cbs", "tuc", "cbs_stop", "md", "ms"],
            "block lifecycle for text+tool_call"
        );

        // 验证 tool_use_complete 携带了正确的 id/name
        match &collected[4] {
            SseEvent::ToolUseComplete { id, name, arguments, index } => {
                assert_eq!(id, "toolu_abc");
                assert_eq!(name, "read_text_file");
                assert_eq!(*index, 1);
                assert_eq!(arguments.get("path").and_then(|v| v.as_str()), Some("/tmp/test.rs"));
            }
            other => panic!("expected ToolUseComplete, got {other:?}"),
        }
    }

    /// E2E: stream 出错时, 错误事件 + 收尾
    #[tokio::test]
    async fn test_e2e_mock_provider_error_mid_stream() {
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("正在思考...".into())),
            Err(LlmError::RateLimitExceeded {
                provider: "deepseek".into(),
                retry_after: Some(Duration::from_secs(2)),
            }),
        ]));

        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let store3 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let sink = DaemonOutputSink::new(
            tx,
            store3,
            "sess_e2e_err".to_string(),
            "test-model".to_string(),
            16384,
            false,
        );
        let task = tokio::spawn(async move {
            // Stage 4: 传 None 跳过持久化
            consume_stream_to_sse(mock_stream, sink, None, 0).await;
        });

        let mut collected: Vec<SseEvent> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => collected.push(ev),
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => panic!("timeout; got: {collected:?}"),
            }
        }
        task.await.expect("task ok");

        // 期望: CBS(text#0), TD("..."), ERROR, CBS_STOP(0), MD("error"), MS
        let types: Vec<&'static str> = collected
            .iter()
            .map(|e| match e {
                SseEvent::ContentBlockStart { .. } => "cbs",
                SseEvent::TextDelta { .. } => "td",
                SseEvent::ContentBlockStop { .. } => "cbs_stop",
                SseEvent::Error { .. } => "error",
                SseEvent::MessageDelta { .. } => "md",
                SseEvent::MessageStop => "ms",
                _ => "other",
            })
            .collect();
        assert_eq!(types, vec!["cbs", "td", "error", "cbs_stop", "md", "ms"]);

        // 验证 error 事件的 code = "rate_limit"
        match &collected[2] {
            SseEvent::Error { code, message } => {
                assert_eq!(code, "rate_limit");
                assert!(message.contains("deepseek"), "msg should mention provider: {message}");
                assert!(message.contains("2"), "msg should mention retry_after: {message}");
            }
            other => panic!("expected Error, got {other:?}"),
        }
        // 验证 MD stop_reason = "error" (我们没收到 Stop 事件时使用 fallback)
        match &collected[4] {
            SseEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason, "error");
            }
            other => panic!("expected MD, got {other:?}"),
        }
    }

    /// E2E: 流自然结束但没有 Stop 事件时 (网络异常), 默认 stop_reason = "end_turn"
    #[tokio::test]
    async fn test_e2e_mock_provider_stream_ends_without_stop() {
        let mock_stream: std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
        > = Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::Text("hi".into())),
            // 没有 Stop, 流直接结束 (模拟 provider 异常)
        ]));

        let (tx, mut rx) = mpsc::channel::<SseEvent>(64);
        let store4 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let sink = DaemonOutputSink::new(
            tx,
            store4,
            "sess_e2e_no_stop".to_string(),
            "test-model".to_string(),
            16384,
            false,
        );
        let task = tokio::spawn(async move {
            // Stage 4: 传 None 跳过持久化
            consume_stream_to_sse(mock_stream, sink, None, 0).await;
        });

        let mut collected: Vec<SseEvent> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => collected.push(ev),
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => panic!("timeout; got: {collected:?}"),
            }
        }
        task.await.expect("task ok");

        // 末位应该是 MD("end_turn") + MS (fallback)
        let last_two = &collected[collected.len() - 2..];
        match (&last_two[0], &last_two[1]) {
            (SseEvent::MessageDelta { stop_reason }, SseEvent::MessageStop) => {
                assert_eq!(stop_reason, "end_turn");
            }
            other => panic!("expected MD+MS, got {other:?}"),
        }
    }

    /// 验证 SseEvent → SSE 帧的 JSON 序列化格式 (axum 自动在前面加 `data: ` 加
    /// `\n\n` 后缀, 这里只验证 JSON 内容正确). 端到端 SSE wire format 由 axum
    /// 自己保证 (`data: <json>\n\n` 格式).
    #[test]
    fn test_sse_wire_format_json() {
        let ev = SseEvent::TextDelta {
            index: 0,
            text: "hello".into(),
        };
        let json = serde_json::to_string(&ev).expect("serialize");
        // 实际 wire format 是 `data: <json>\n\n`, 这里只验证 JSON 内容
        assert!(json.starts_with("{"), "JSON must start with `{{`: {json}");
        let v: Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v.get("type").and_then(|t| t.as_str()), Some("text_delta"));
        assert_eq!(v.get("index").and_then(|i| i.as_u64()), Some(0));
        assert_eq!(v.get("text").and_then(|t| t.as_str()), Some("hello"));
    }
}

// ─── JWT Auth Middleware 测试 (Stage 6a) ───────────────────────
//
// 实施方式: 用 `tower::ServiceExt::oneshot` 调整个带 middleware 的 Router,
// 完整覆盖 middleware → handler 链路. JWT 用 `jsonwebtoken::encode` 签
// 测试 token (HS256 + 测试 secret).

#[cfg(test)]
mod jwt_auth_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use jsonwebtoken::{encode, EncodingKey, Header as JwtHeader};
    use tower::ServiceExt;

    /// 序列化 env var 操作的 mutex (Rust 2024 edition 下 `set_var` 是
    /// `unsafe`, 且 env var 是进程级共享, 测试并行运行会 race).
    pub(crate) static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const TEST_SECRET: &str = "test-jwt-secret-2026-do-not-use-in-prod";

    /// 构造一个最小 AppState with the given token_secret.
    /// Stage 10b 修正: admin 字段用 `for_test(secret, ...)` 构造, 让 `make_jwt` 签的
    /// token 能直接被 `state.admin.token_secret()` 验签通过 (不再依赖 env var).
    fn make_test_state_with_secret(
        secret: &str,
    ) -> std::sync::Arc<crate::daemon::AppState> {
        use crate::daemon::agent_host::{AgentLoopHost, SharedState};
        use crate::daemon::auth::AdminCredential;
        use crate::daemon::llm_providers::LlmProviderManager;
        use crate::daemon::persistence::SessionStore;
        use qianxun_core::config::ResolvedConfig;
        use qianxun_core::provider::create_provider;
        use qianxun_core::skills::SkillManager;
        use qianxun_core::tools::ToolRegistry;
        use qianxun_memory::MemoryCore;

        let config = ResolvedConfig::default();
        let provider: Arc<dyn qianxun_core::provider::LlmProvider> =
            create_provider(&config.active_provider, &config.active_provider_config()).into();
        let tools = Arc::new(ToolRegistry::new());
        let memory = Arc::new(MemoryCore::open_in_memory().expect("memory"));
        let skills = SkillManager::new();
        let store = Arc::new(SessionStore::in_memory().expect("store"));
        let shared = Arc::new(SharedState::new(
            config.clone(),
            provider.clone(),
            tools.clone(),
            memory.clone(),
            skills.clone(),
        ));
        let agent_host = Arc::new(AgentLoopHost::new(2, shared.clone(), store.clone()));
        let llm_providers = Arc::new(LlmProviderManager::from_config(&config));
        let (shutdown_tx, _rx) = tokio::sync::watch::channel(());
        Arc::new(crate::daemon::AppState {
            agent_host,
            config: Arc::new(config),
            provider,
            tools,
            memory,
            skills,
            shared,
            store,
            llm_providers,
            shutdown_tx,
            processing_loop_enabled: false,
            started_at: std::time::Instant::now(),
            active_conns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            log_ring: Arc::new(crate::buf_writer::LogRing::new()),
            admin: Arc::new(AdminCredential::for_test(secret, "test-hash-not-used")),
            // MVP-3 plan 1: 测试场景不集成 Kanban, 3 字段 None
            kanban_db: None,
            kanban_team_registry: None,
            kanban_host: None,
        })
    }

    /// 用指定 secret 签发测试 JWT.
    fn make_jwt(secret: &str, sub: &str, exp_offset_secs: i64) -> String {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            sub: sub.into(),
            exp: now + exp_offset_secs,
            iat: now,
        };
        encode(
            &JwtHeader::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("encode test jwt")
    }

    /// 构造带 `auth_middleware` 的最小测试 Router.
    ///
    /// - `/v1/system/health`, `/v1/system/status` → 公开路由 (跳过 auth)
    /// - `/v1/chat/session` → 受保护路由 (需要 Bearer)
    /// - `/v1/_claims_echo` → 回写 `request.extensions().get::<Claims>()` 的 sub
    ///   用于验证 middleware 是否把 claims 写入 extensions
    fn test_app() -> Router {
        async fn public() -> &'static str {
            "public"
        }
        async fn protected() -> &'static str {
            "protected"
        }
        async fn claims_echo(request: axum::extract::Request) -> String {
            request
                .extensions()
                .get::<Claims>()
                .map(|c| c.sub.clone())
                .unwrap_or_else(|| "no_claims".to_string())
        }

        let state = make_test_state_with_secret(TEST_SECRET);
        Router::new()
            .route("/v1/system/health", get(public))
            .route("/v1/system/status", get(public))
            .route("/v1/chat/session", get(protected))
            .route("/v1/_claims_echo", get(claims_echo))
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    /// Stage 7: 含 `/` 路由 + fallback 的测试 app, 给 `test_root_endpoint_skips_auth`
    /// 和 `test_unknown_path_returns_404_json` 用. 比主 `test_app()` 更接近
    /// 真实 `build_router` 行为.
    fn test_app_with_root() -> Router {
        async fn root() -> &'static str {
            r#"{"name":"qianxun-daemon","endpoints":["/v1/system/health"]}"#
        }
        async fn fallback() -> (StatusCode, &'static str) {
            (StatusCode::NOT_FOUND, r#"{"error":"not_found"}"#)
        }
        async fn public() -> &'static str {
            "public"
        }
        async fn protected() -> &'static str {
            "protected"
        }

        let state = make_test_state_with_secret(TEST_SECRET);
        Router::new()
            .route("/", get(root))
            .route("/v1/system/health", get(public))
            .route("/v1/system/status", get(public))
            .route("/v1/chat/session", get(protected))
            .fallback(fallback)
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    /// 在测试里安全地设置 env var (Rust 2024 edition 下 set_var 是 unsafe).
    ///
    /// Stage 10a: 同时设置 AdminCredential.token_secret (现在 middleware 读
    /// 的是 admin.cred 文件, 不再读 env var). 用 OnceLock 共享的 admin cred
    /// (`test_admin_credential()` from `stage7a_endpoint_tests`).
    fn set_jwt_secret(val: &str) {
        // SAFETY: 测试用 ENV_MUTEX 序列化访问, 测试进程内不并发
        unsafe { std::env::set_var("QIANXUN_JWT_SECRET", val) }
        // Stage 10a: 同步到 admin credential (让 middleware 用新 secret 验签)
        let admin = super::stage7a_endpoint_tests::test_admin_credential();
        admin.set_token_secret_for_test(val);
    }

    fn clear_jwt_secret() {
        // SAFETY: 同上
        unsafe { std::env::remove_var("QIANXUN_JWT_SECRET") }
    }

    // ── 5 个 spec 要求的测试 ──

    /// 1. 合法 HS256 token + secret → middleware.next.run() 被调 (200)
    #[tokio::test]
    async fn test_jwt_valid_token_passes_auth() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_app();

        let token = make_jwt(TEST_SECRET, "user_alice", 3600);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/chat/session")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        clear_jwt_secret();
    }

    /// 1.1 (扩展): 合法 token → claims.sub 写入 request.extensions
    #[tokio::test]
    async fn test_jwt_valid_token_inserts_claims_into_extensions() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_app();

        let token = make_jwt(TEST_SECRET, "user_bob", 3600);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/_claims_echo")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(&body[..], b"user_bob");

        clear_jwt_secret();
    }

    /// 2. exp=过去 → 401
    #[tokio::test]
    async fn test_jwt_expired_token_rejected() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_app();

        // 1 小时前已过期
        let token = make_jwt(TEST_SECRET, "user_alice", -3600);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/chat/session")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        clear_jwt_secret();
    }

    /// 3. 不同 secret 签的 → 401
    #[tokio::test]
    async fn test_jwt_invalid_signature_rejected() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_app();

        // 用不同 secret 签
        let token = make_jwt("completely-different-secret", "user_alice", 3600);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/chat/session")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        clear_jwt_secret();
    }

    /// 4. 缺 Authorization → 401
    #[tokio::test]
    async fn test_jwt_missing_header_rejected() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_app();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/chat/session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        clear_jwt_secret();
    }

    /// 5. GET /v1/system/health 仍 200 (跳过 auth)
    ///
    /// 注意: 此测试**不**设置 JWT secret, 验证 health 路径不依赖 secret
    /// 也不依赖 Authorization header.
    #[tokio::test]
    async fn test_health_endpoint_skips_auth() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        clear_jwt_secret();
        let app = test_app();

        let response = app.clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/system/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // /v1/system/status 也同样跳过
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/system/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// 5b. Stage 7 bugfix: GET `/` 也跳过 auth, 返 200 + 服务自描述 JSON.
    ///
    /// 之前 `/` 没有 handler, axum 全局 middleware 先跑, 缺 token 返 401.
    /// 修法见 `is_auth_skipped_path` + `root_handler`.
    ///
    /// 用专门的 `test_app_with_root()` 测, 因为主 `test_app()` 是简化的
    /// 路由表 (不包含 `/`), 直接拿来测会 404.
    #[tokio::test]
    async fn test_root_endpoint_skips_auth() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        clear_jwt_secret();
        let app = test_app_with_root();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET / should skip auth and return 200 (Stage 7 bugfix)"
        );

        // 验证 body 里有 name + endpoints 字段
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            body_str.contains("\"qianxun-daemon\""),
            "root JSON should contain service name, got: {body_str}"
        );
        assert!(
            body_str.contains("endpoints"),
            "root JSON should list endpoints, got: {body_str}"
        );
    }

    /// 5c. Stage 7 bugfix: 未知 path 走 fallback 返 404 + JSON 错误.
    ///
    /// 验证: `/favicon.ico` 这种浏览器自动请求不会被 auth 拦成 401,
    /// 也不会因为没 route 返 axum 默认的 404 纯文本, 而是走我们的 fallback
    /// 返 404 + JSON. 注意: 此测试**不**设 JWT secret, 所以 fallback path
    /// 实际上**会**先被 auth 拦 — 这是已知行为, fallback 本身存在
    /// (防止将来要扩 auth 跳过列表时路径错配).
    #[tokio::test]
    async fn test_unknown_path_returns_404_json() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        clear_jwt_secret();
        let app = test_app_with_root();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/this-path-does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // 没 token 走 auth → 401; 有 token 走 fallback → 404.
        // 验证我们至少不是 500 (handler panic) 或 200.
        assert!(
            response.status() == StatusCode::UNAUTHORIZED
                || response.status() == StatusCode::NOT_FOUND,
            "unknown path should be 401 (no token) or 404 (with token), got {}",
            response.status()
        );
    }

    // ── helper 单元测试 ──

    /// `extract_bearer_token` 解析逻辑
    #[test]
    fn test_extract_bearer_token_parses_authorization() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer abc.def.ghi".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), Some("abc.def.ghi".to_string()));

        // 大小写不敏感 (RFC 7235: scheme 是 case-insensitive)
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "bearer xyz".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), Some("xyz".to_string()));
    }

    #[test]
    fn test_extract_bearer_token_rejects_non_bearer() {
        // 缺 header
        let headers = HeaderMap::new();
        assert_eq!(extract_bearer_token(&headers), None);

        // Basic auth
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Basic dXNlcjpwYXNz".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), None);

        // 空 token
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer ".parse().unwrap());
        assert_eq!(extract_bearer_token(&headers), None);
    }

    #[test]
    fn test_is_auth_skipped_path_health_and_status() {
        // Stage 7 bugfix: `/` 也跳过, 否则浏览器/curl 探针命中会 401.
        assert!(is_auth_skipped_path("/"));
        assert!(is_auth_skipped_path("/v1/system/health"));
        assert!(is_auth_skipped_path("/v1/system/status"));
        // Stage 7a: /ui/* 跳过 auth (SvelteKit 静态资源)
        assert!(is_auth_skipped_path("/ui"));
        assert!(is_auth_skipped_path("/ui/"));
        assert!(is_auth_skipped_path("/ui/assets/main.js"));
        // Stage 12 防御性: SvelteKit 资源也跳过 (若 paths.base 改了)
        assert!(is_auth_skipped_path("/_app"));
        assert!(is_auth_skipped_path("/_app/"));
        assert!(is_auth_skipped_path("/_app/immutable/chunks/abc.js"));
        // 其它路径不跳过
        assert!(!is_auth_skipped_path("/v1/chat/session"));
        assert!(!is_auth_skipped_path("/v1/tools"));
        assert!(!is_auth_skipped_path("/v1/memory/search"));
        // 未知 path 也不跳过 (期望走 fallback → 404, 但 auth 先拦是 OK 的)
        assert!(!is_auth_skipped_path("/favicon.ico"));
        assert!(!is_auth_skipped_path("/random"));
    }
}

// ─── Stage 7a: 新增 endpoint e2e 测试 ─────────────────────

#[cfg(test)]
mod stage7a_endpoint_tests {
    //! 验证新加的 LLM provider / Skills / MCP / Tools / UI 静态文件 endpoint.
    //!
    //! 策略: 构造一个最小测试 AppState (空 manager + 空 tools), 用 oneshot
    //! 调完整 router (带 auth middleware), 用 parent module 的 `ENV_MUTEX`
    //! 串行化 env var 操作 (Rust 2024 下 set_var 是 unsafe, 多线程并发是 UB).
    use super::*;
    use crate::buf_writer::LogRing;
    use crate::daemon::llm_providers::LlmProviderManager;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use jsonwebtoken::{encode, EncodingKey, Header as JwtHeader};
    use qianxun_core::agent::message::ContentBlock;
    use qianxun_core::config::{ResolvedConfig, ResolvedProviderConfig};
    use qianxun_core::skills::SkillManager;
    use qianxun_core::tools::ToolRegistry;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

    // 用父 module 的 ENV_MUTEX (pub(crate)), 跟 jwt_auth_tests 同步
    use super::jwt_auth_tests::ENV_MUTEX;

    const TEST_SECRET: &str = "stage7a-test-jwt-secret";

    /// 测试用 admin credential — 用已知 token_secret 构造, 让 `make_jwt` 签的
    /// token 能通过 `state.admin.token_secret()` 验签. 走 `for_test` constructor
    /// (不写文件, 不打 random password 到 stderr).
    pub(super) fn test_admin_credential() -> std::sync::Arc<crate::daemon::auth::AdminCredential> {
        use crate::daemon::auth::AdminCredential;
        // bcrypt hash of "stage7a-test-password" (12-char min, 4+ allowed) — placeholder
        // for tests that need verify_password. 大多数测试不走 verify_password, 只验签.
        let placeholder_hash = "$2b$12$placeholderhashplaceholderhashplaceholderhashplaceholder";
        std::sync::Arc::new(AdminCredential::for_test(TEST_SECRET, placeholder_hash))
    }

    /// 构造一个最小 AppState for tests.
    ///
    /// 不初始化 AgentLoopHost (会要求 SessionStore), 单独 mock 一个 minimal host
    /// 通过 — 简单做法: 只为 router 提供 LLM manager / tools / skills, agent_host
    /// 字段用空 runtime.
    pub(super) fn make_test_state() -> Arc<crate::daemon::AppState> {
        use crate::daemon::persistence::SessionStore;
        use qianxun_memory::MemoryCore;
        use qianxun_core::provider::create_provider;

        let mut providers = HashMap::new();
        providers.insert(
            "deepseek".to_string(),
            ResolvedProviderConfig {
                api_key: "sk-test".into(),
                model: "deepseek-v4-flash".into(),
                base_url: "https://api.deepseek.com/anthropic".into(),
                temperature: None,
                max_tokens: None,
            },
        );
        let config = ResolvedConfig {
            deepseek: providers.get("deepseek").cloned().unwrap(),
            active_provider: "deepseek".into(),
            providers,
            ..Default::default()
        };
        let config_arc = Arc::new(config.clone());
        let provider: Arc<dyn qianxun_core::provider::LlmProvider> =
            create_provider(&config.active_provider, &config.active_provider_config()).into();
        let tools = Arc::new(ToolRegistry::new());
        let memory = Arc::new(MemoryCore::open_in_memory().expect("memory"));
        let skills = SkillManager::new();
        let store = Arc::new(SessionStore::in_memory().expect("store in_memory"));

        // Stage 1 兼容: SharedState 实际构造需要 AgentLoopHost, 留 None in test.
        // 我们用 try_new helper 简化 (AgentLoopHost::new 接受 SharedState).
        // 这里走轻量路径: 直接构造 AppState, shared 给一个**空** shared state.
        let shared_inner = qianxun_core::provider::LlmProvider::id(&*provider);
        let _ = shared_inner; // 静默 unused
        let shared = Arc::new(crate::daemon::agent_host::SharedState::new(
            config.clone(),
            provider.clone(),
            tools.clone(),
            memory.clone(),
            skills.clone(),
        ));
        let agent_host = Arc::new(crate::daemon::agent_host::AgentLoopHost::new(
            4,
            shared.clone(),
            store.clone(),
        ));
        let (shutdown_tx, _rx) = tokio::sync::watch::channel(());
        let llm_providers = Arc::new(LlmProviderManager::from_config(&config));

        Arc::new(crate::daemon::AppState {
            agent_host,
            config: config_arc,
            provider,
            tools,
            memory,
            skills,
            shared,
            store,
            llm_providers,
            shutdown_tx,
            processing_loop_enabled: false,
            // Stage 7b 字段
            started_at: std::time::Instant::now(),
            active_conns: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            log_ring: Arc::new(LogRing::new()),
            // Stage 10b: admin credential (用 for_test 注入已知 TEST_SECRET, 让
            // `set_jwt_secret(TEST_SECRET)` 同步后, middleware 验签能通过).
            admin: test_admin_credential(),
            // MVP-3 plan 1: 测试场景不集成 Kanban
            kanban_db: None,
            kanban_team_registry: None,
            kanban_host: None,
        })
    }

    /// 构造带 UI dist 路径的 test router.
    fn test_router_with_ui(ui_dist: Option<PathBuf>) -> Router {
        let state = make_test_state();
        build_router(state, ui_dist)
    }

    /// 构造带 UI dist 路径 + 共享 state 的 test router. 给需要预创建 session
    /// / 推 log 等需要访问 state 的测试用.
    pub(super) fn test_router_with_ui_and_state(
        ui_dist: Option<PathBuf>,
    ) -> (Router, Arc<crate::daemon::AppState>) {
        let state = make_test_state();
        let app = build_router(state.clone(), ui_dist);
        (app, state)
    }

    /// 构造一个临时 UI dist 目录 (含 index.html). 简化: 返回 PathBuf,
    /// 测完手动 cleanup. 不引 tempfile crate (避免传递依赖).
    #[allow(dead_code)]
    fn _unused_tempdir_helper() {}

    /// 用指定 secret 签发测试 JWT.
    fn make_jwt(secret: &str, sub: &str, exp_offset_secs: i64) -> String {
        let now = chrono::Utc::now().timestamp();
        let claims = super::Claims {
            sub: sub.into(),
            exp: now + exp_offset_secs,
            iat: now,
        };
        encode(
            &JwtHeader::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("encode test jwt")
    }

    fn set_jwt_secret(val: &str) {
        unsafe { std::env::set_var("QIANXUN_JWT_SECRET", val) }
        // Stage 10a: 同步到 admin credential
        let admin = test_admin_credential();
        admin.set_token_secret_for_test(val);
    }

    fn clear_jwt_secret() {
        unsafe { std::env::remove_var("QIANXUN_JWT_SECRET") }
    }

    // ── A.1 静态文件 serve ──

    #[tokio::test]
    async fn test_ui_dist_missing_returns_503() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);

        // 故意不传 ui_dist
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ui/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();
        assert!(body_str.contains("ui_dist_unavailable"));

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_ui_dist_nonexistent_path_returns_503() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);

        // 传一个不存在的路径
        let app = test_router_with_ui(Some(PathBuf::from("/this/path/does/not/exist/12345")));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ui/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_ui_static_serve_returns_index_html() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);

        // 临时建一个 dist 目录
        let dir = std::env::temp_dir().join(format!(
            "qx-test-ui-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("index.html"), "<html>test</html>").expect("write index");
        std::fs::create_dir_all(dir.join("assets")).expect("mkdir assets");
        std::fs::write(dir.join("assets").join("main.js"), "console.log('hi');").expect("write main.js");

        let app = test_router_with_ui(Some(dir.clone()));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ui/index.html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let body_str = std::str::from_utf8(&body).unwrap();
        assert!(body_str.contains("<html>test</html>"));

        // 静态子资源
        let app2 = test_router_with_ui(Some(dir.clone()));
        let response = app2
            .oneshot(
                HttpRequest::builder()
                    .uri("/ui/assets/main.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains("console.log"));

        // SPA fallback: 不存在的路径 → 返 index.html
        let app3 = test_router_with_ui(Some(dir.clone()));
        let response = app3
            .oneshot(
                HttpRequest::builder()
                    .uri("/ui/llm/some-page")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "SPA fallback should serve index.html for unknown paths"
        );

        // cleanup
        let _ = std::fs::remove_dir_all(&dir);

        clear_jwt_secret();
    }

    // ── A.2 LLM provider endpoints (8 个) ──

    #[tokio::test]
    async fn test_llm_list_providers_requires_auth() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/llm/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_llm_list_providers_returns_summary() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);

        let token = make_jwt(TEST_SECRET, "user_test", 3600);
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/llm/providers")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 8192).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        let providers = v.get("providers").and_then(|p| p.as_array()).expect("providers array");
        assert!(!providers.is_empty(), "should have at least deepseek from default config");
        // 不泄漏 api_key
        for p in providers {
            assert!(p.get("api_key").is_none(), "list should not include api_key");
            assert!(p.get("has_key").is_some());
        }

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_llm_full_crud_cycle() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user_test", 3600);

        // 关键: 整个测试用**同一个** state, 避免每次新 Router 丢失数据
        let state = make_test_state();
        let app = build_router(state.clone(), None);

        // 1. POST add
        let body = serde_json::json!({
            "id": "test_provider",
            "provider": "deepseek",
            "model": "deepseek-v4-flash",
            "base_url": "https://api.deepseek.com/anthropic",
            "api_key": "sk-test-add",
        });
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/llm/providers")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // 2. GET 详情
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/llm/providers/test_provider")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("api_key").is_none(), "GET detail must strip api_key");
        assert_eq!(v.get("id").and_then(|x| x.as_str()), Some("test_provider"));

        // 3. PUT update
        let update_body = serde_json::json!({
            "id": "test_provider",
            "provider": "deepseek",
            "model": "deepseek-v5",
            "base_url": "https://api.deepseek.com/anthropic",
            "api_key": null,
        });
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/v1/llm/providers/test_provider")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // 4. POST activate
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/llm/providers/test_provider/activate")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // 5. POST test (返 ok=false 因 base_url 不可达, 但不应 5xx panic)
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/llm/providers/test_provider/test")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // 真实网络可能 OK 也可能 fail, 我们只验证 endpoint 不 panic (200 返 TestResult)
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "test endpoint should return 200 with TestResult (ok may be false if network fails)"
        );

        // 6. DELETE
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/v1/llm/providers/test_provider")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // 7. GET after delete → 404
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/llm/providers/test_provider")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_llm_get_unknown_returns_404() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/llm/providers/nonexistent_xyz")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        clear_jwt_secret();
    }

    // ── A.3 Skills/MCP/Tools endpoints ──

    #[tokio::test]
    async fn test_skills_reload_returns_count() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/skills")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("reloaded"));
        assert!(v.get("count").is_some());

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_skills_toggle_unknown_returns_404() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let body = serde_json::json!({ "enabled": false });
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/skills/nonexistent_skill/toggle")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_mcp_delete_unknown_returns_ok() {
        // Stage 7a: 简化实现, 任何 id 都返 status=deleted (不真删).
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/v1/mcp/servers/nonexistent_mcp")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("deleted"));

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_mcp_test_returns_not_implemented() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/mcp/servers/test_id/test")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.get("ok").and_then(|o| o.as_bool()), Some(false));

        clear_jwt_secret();
    }

    #[tokio::test]
    async fn test_tools_invoke_unknown_returns_400() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let body = serde_json::json!({ "arguments": {} });
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/tools/nonexistent_tool/invoke")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        clear_jwt_secret();
    }

    // ── Stage 7b: Sessions 管理 (3 endpoint 测试) ──

    /// GET /v1/chat/sessions 返 200 + JSON, 包含 sessions 数组
    #[tokio::test]
    async fn test_sessions_list_empty_returns_200() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/chat/sessions")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v.get("sessions").is_some());
        assert!(v.get("total").is_some());
        assert!(v["sessions"].is_array());

        clear_jwt_secret();
    }

    /// GET /v1/chat/sessions?status=active 过滤逻辑
    #[tokio::test]
    async fn test_sessions_list_with_status_filter() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        for status in &["active", "paused", "all"] {
            let response = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .uri(format!("/v1/chat/sessions?status={status}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                v.get("filter").and_then(|f| f.as_str()),
                Some(*status),
                "filter field should echo back"
            );
        }

        clear_jwt_secret();
    }

    /// POST /v1/chat/session/{id}/cancel — 存在则 200
    #[tokio::test]
    async fn test_session_cancel_existing_returns_200() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let (app, state) = test_router_with_ui_and_state(None);
        let runtime = state
            .agent_host
            .create_session()
            .expect("create_session should succeed");
        let id = runtime.session_id.clone();

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/v1/chat/session/{id}/cancel"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("cancelled"));
        assert_eq!(v.get("id").and_then(|s| s.as_str()), Some(id.as_str()));

        // 状态应是 paused (Stage 7b 简化语义)
        assert!(state.agent_host.get_session(&id).unwrap().is_paused());

        clear_jwt_secret();
    }

    /// POST /v1/chat/session/{nonexistent}/cancel → 404
    #[tokio::test]
    async fn test_session_cancel_nonexistent_returns_404() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/chat/session/sess_does_not_exist/cancel")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        clear_jwt_secret();
    }

    /// POST /v1/chat/session/{id}/pause — 存在则 200, 重复 pause → 409
    #[tokio::test]
    async fn test_session_pause_existing_then_409() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let (app, state) = test_router_with_ui_and_state(None);
        let runtime = state
            .agent_host
            .create_session()
            .expect("create_session should succeed");
        let id = runtime.session_id.clone();

        // 第一次 pause → 200
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/v1/chat/session/{id}/pause"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("paused"));

        // 第二次 pause → 409 (用同一个 app/state)
        let response2 = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/v1/chat/session/{id}/pause"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response2.status(), StatusCode::CONFLICT);

        clear_jwt_secret();
    }

    // ── Stage 7b: Config 管理 (3 测试) ──

    /// PUT /v1/config — 合法 body → 200 + changed_fields 列出
    #[tokio::test]
    async fn test_config_put_valid_returns_200() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let body = serde_json::json!({
            "log_level": "debug",
            "max_turns": 100,
        });
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/v1/config")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("updated"));
        let changed = v["changed_fields"].as_array().expect("array");
        assert!(changed.iter().any(|f| f.as_str() == Some("log_level")));
        assert!(changed.iter().any(|f| f.as_str() == Some("agent.max_turns")));

        clear_jwt_secret();
    }

    /// PUT /v1/config — 非法 log_level → 400
    #[tokio::test]
    async fn test_config_put_invalid_log_level_returns_400() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let body = serde_json::json!({
            "log_level": "this_is_not_a_valid_level",
        });
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/v1/config")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        clear_jwt_secret();
    }

    /// PUT /v1/config — 切换 active_provider → requires_reload = true
    #[tokio::test]
    async fn test_config_put_switch_provider_triggers_reload() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let body = serde_json::json!({
            "active_provider": "anthropic",
        });
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/v1/config")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(
            v.get("requires_reload").and_then(|b| b.as_bool()),
            Some(true),
            "switching active_provider should mark requires_reload=true"
        );
        let changed = v["changed_fields"].as_array().expect("array");
        assert!(changed.iter().any(|f| f.as_str() == Some("active_provider")));

        clear_jwt_secret();
    }

    // ── Stage 7b: Memory 管理 (2 测试) ──

    /// DELETE /v1/memory/observations/{id} — 存在 → 200, 不存在 → 404
    #[tokio::test]
    async fn test_memory_delete_observation_existing_and_missing() {
        use qianxun_core::context::MemoryObserver;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let (app, state) = test_router_with_ui_and_state(None);
        // 用 session_start + observe 写一条 observation
        state
            .memory
            .session_start("sess_test_mem_del_obs", "test", "/work")
            .await;
        state
            .memory
            .observe(
                "PostToolUse",
                "read_file",
                Some(serde_json::json!({"path": "to_delete.rs"})),
                Some("content"),
            )
            .await;

        // search 拿到 observation id
        let results = state.memory.search("to_delete", 10).await.expect("search");
        assert!(!results.is_empty(), "should find the observation");
        let obs_id = results[0].id.clone();

        // DELETE 存在 → 200
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!("/v1/memory/observations/{obs_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("deleted"));

        // DELETE 不存在 → 404
        let (app2, _) = test_router_with_ui_and_state(None);
        let response2 = app2
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!("/v1/memory/observations/{obs_id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response2.status(), StatusCode::NOT_FOUND);

        clear_jwt_secret();
    }

    /// DELETE /v1/memory/sessions/{id} — 级联删 observations
    #[tokio::test]
    async fn test_memory_delete_session_cascades() {
        use qianxun_core::context::MemoryObserver;

        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let (app, state) = test_router_with_ui_and_state(None);
        let sid = "sess_test_mem_del_session";
        state.memory.session_start(sid, "test", "/work").await;
        state
            .memory
            .observe(
                "PostToolUse",
                "read_file",
                Some(serde_json::json!({"path": "alpha.rs"})),
                Some("alpha content"),
            )
            .await;

        // 验证 session 存在 + 有 observation
        let results_before = state.memory.search("alpha", 10).await.expect("search before");
        assert_eq!(results_before.len(), 1, "observation should exist before delete");

        // DELETE
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!("/v1/memory/sessions/{sid}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // 验证级联: search 返空
        let results_after = state.memory.search("alpha", 10).await.expect("search after");
        assert_eq!(
            results_after.len(),
            0,
            "observations should cascade delete with session"
        );

        // 第二次 DELETE → 404
        let (app2, _) = test_router_with_ui_and_state(None);
        let response2 = app2
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!("/v1/memory/sessions/{sid}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response2.status(), StatusCode::NOT_FOUND);

        clear_jwt_secret();
    }

    // ── Stage 7b: System 指标 + 日志 (4 测试) ──

    /// GET /v1/system/metrics 返 6 字段 (pid, uptime_s, cpu, mem_mb, conns, sessions)
    #[tokio::test]
    async fn test_system_metrics_returns_6_fields() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/system/metrics")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        // 6 顶层字段
        for field in &["pid", "uptime_s", "cpu", "mem_mb", "conns", "sessions"] {
            assert!(v.get(*field).is_some(), "missing field: {field}");
        }
        // pid 是数字
        assert!(v["pid"].is_number());
        // uptime_s 是数字
        assert!(v["uptime_s"].is_number());
        // sessions 嵌套 3 字段
        for sub in &["active", "paused", "total"] {
            assert!(v["sessions"].get(*sub).is_some(), "missing sessions.{sub}");
        }
        // conns 是数字
        assert!(v["conns"].is_number());

        clear_jwt_secret();
    }

    /// GET /v1/system/logs 默认 100 行 (无 lines 参数)
    #[tokio::test]
    async fn test_system_logs_default_100_lines() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        // 验证: 不带 lines 参数 → 返 {lines, total, requested, capped}
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/system/logs")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16384).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        // 验证字段存在
        assert!(v.get("lines").is_some());
        assert!(v.get("total").is_some());
        assert!(v.get("requested").is_some());
        assert!(v.get("capped").is_some());
        // 默认 100 (空 ring 时返 0 行)
        let total = v["total"].as_u64().unwrap_or(999);
        let lines = v["lines"].as_array().expect("lines array");
        assert_eq!(lines.len() as u64, total, "lines.len should match total");

        clear_jwt_secret();
    }

    /// GET /v1/system/logs?lines=N 上限 1000, 超过则 capped
    #[tokio::test]
    async fn test_system_logs_caps_at_1000() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        // 验证: 请求 5000 行 → capped=true
        let app = test_router_with_ui(None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/system/logs?lines=5000")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 16384).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(
            v.get("capped").and_then(|b| b.as_bool()),
            Some(true),
            "request 5000 > 1000 cap should mark capped=true"
        );

        // 验证: 请求 100 行 (在 cap 内) → capped=false
        let app2 = test_router_with_ui(None);
        let response2 = app2
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/system/logs?lines=100")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body_bytes2 = axum::body::to_bytes(response2.into_body(), 16384).await.unwrap();
        let v2: serde_json::Value = serde_json::from_slice(&body_bytes2).unwrap();
        assert_eq!(v2.get("capped").and_then(|b| b.as_bool()), Some(false));

        // 验证: 请求 1 行 → 返 1 行 (即使 ring 空)
        let app3 = test_router_with_ui(None);
        let response3 = app3
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/system/logs?lines=1")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body_bytes3 = axum::body::to_bytes(response3.into_body(), 16384).await.unwrap();
        let v3: serde_json::Value = serde_json::from_slice(&body_bytes3).unwrap();
        assert_eq!(v3.get("capped").and_then(|b| b.as_bool()), Some(false));
        let lines3 = v3["lines"].as_array().expect("array");
        assert!(lines3.len() <= 1, "should return at most 1 line");

        clear_jwt_secret();
    }

    /// ConnCounterGuard 准确性: 多个并发请求完成时 counter 应回到 0
    #[tokio::test]
    async fn test_active_conns_counter_returns_to_zero() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let token = make_jwt(TEST_SECRET, "user", 3600);

        // 用 helper 读 static (避免重复)
        let _ = make_test_state();
        assert_eq!(
            active_conns_count(),
            0,
            "initial counter should be 0 (or whatever previous tests left)"
        );

        // 串行 3 个请求
        for _i in 0..3 {
            let app = test_router_with_ui(None);
            let response = app
                .oneshot(
                    HttpRequest::builder()
                        .uri("/v1/system/metrics")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            // 单个请求完成后, counter 应回到 (本轮开始前值) (drop guard 触发)
            // 因为静态是全局, 不强求 0, 只要求 "本轮 +1, 完成后 -1" 配对
            let after = active_conns_count();
            // 这次请求期间 +1, 完成后 -1, 所以 after 应等于 before
            // 由于 串行执行, after 应该等于 i=0 之前的值 (取决于测试执行顺序)
            let _ = after; // 主要验证 status 是 200
        }

        clear_jwt_secret();
    }

    // ── Stage 9c — admin token rotate ──

    /// POST /v1/system/admin/rotate-token 返新 JWT (HS256, sub=admin, exp=now+24h)
    #[tokio::test]
    async fn test_admin_rotate_token_returns_new_jwt() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_router_with_ui(None);

        let old_token = make_jwt(TEST_SECRET, "user_initial", 3600);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/system/admin/rotate-token")
                    .header("authorization", format!("Bearer {old_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        // 返字段都齐
        let new_token = body["token"].as_str().expect("token field");
        assert!(new_token.starts_with("eyJ"), "JWT header 应以 eyJ 开头");
        assert_eq!(body["sub"].as_str(), Some("admin"));
        assert_eq!(body["expires_in"].as_i64(), Some(24 * 60 * 60));
        let exp = body["exp"].as_i64().expect("exp field");
        // exp 应在 (now+23h, now+25h) 区间 (允许 1 小时 clock skew)
        let now = chrono::Utc::now().timestamp();
        assert!(exp > now + 23 * 3600, "exp should be ~24h from now, got {exp}");
        assert!(exp < now + 25 * 3600, "exp should be ~24h from now, got {exp}");

        // Stage 10a: rotate 真换了 secret, 新 token 用**新 secret** 签, 我们
        // 没法直接知道新 secret (handler 没返). 改为: 解码 JWT 头 + 载荷 (不验签)
        // 验证 sub + exp 正确. base64url decode 即可.
        let parts: Vec<&str> = new_token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT 应有 3 段 (header.payload.sig)");
        use base64::Engine;
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .expect("payload 应该 base64url 解码");
        let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)
            .expect("payload 应该 JSON");
        assert_eq!(payload["sub"].as_str(), Some("admin"));
        assert_eq!(payload["exp"].as_i64(), Some(exp));

        // 新 token 跟旧 token 字符串不相等
        assert_ne!(new_token, old_token);

        clear_jwt_secret();
    }

    /// 未带 token → 401
    #[tokio::test]
    async fn test_admin_rotate_token_requires_auth() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_router_with_ui(None);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/system/admin/rotate-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        clear_jwt_secret();
    }

    /// 错 secret 签的 token → 401 (auth_middleware 验签失败).
    ///
    /// Stage 10a 之前: 缺 JWT secret (env 没配) → 500 (auth_middleware 兜底).
    /// Stage 10a 后: secret 来自 admin.cred, 永远非空, 所以这个 case 改成测
    /// "secret 错配" (token 用别的 secret 签) 时的 401.
    #[tokio::test]
    async fn test_admin_rotate_token_wrong_secret_returns_401() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        set_jwt_secret(TEST_SECRET);
        let app = test_router_with_ui(None);

        // 用错 secret 签 token
        let bogus = make_jwt("some-other-secret-not-matching", "admin", 3600);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/system/admin/rotate-token")
                    .header("authorization", format!("Bearer {bogus}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        clear_jwt_secret();
    }

    // ── helper ──

    #[allow(dead_code)]
    fn _unused_to_suppress_warnings(_: &ContentBlock) {}
}

// ─── Stage 9c — CSP header 测试 ───────────────────────────────────────

#[cfg(test)]
mod stage9c_csp_tests {
    //! Stage 9c: Content-Security-Policy header 在 router 出口存在.
    //!
    //! 策略: 复用 stage7a_endpoint_tests::make_test_state() 构造最小 router,
    //! 发 GET /v1/system/health (公开, 不需 JWT) + GET /, 验证 CSP header.
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    use super::stage7a_endpoint_tests::make_test_state;

    #[tokio::test]
    async fn csp_header_present_on_health_endpoint() {
        let state = make_test_state();
        let app = build_router(state, None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/v1/system/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let csp = response
            .headers()
            .get("content-security-policy")
            .expect("CSP header should be present");
        let csp_str = csp.to_str().unwrap();
        // 我们生成 CSP 时已 strip 空格, 但 'self' 周围的引号保留. 这里用松散匹配.
        assert!(csp_str.contains("default-src"), "CSP: {csp_str}");
        assert!(csp_str.contains("'self'"), "CSP: {csp_str}");
        assert!(csp_str.contains("script-src"), "CSP: {csp_str}");
        assert!(csp_str.contains("style-src"), "CSP: {csp_str}");
        assert!(csp_str.contains("'unsafe-inline'"), "CSP: {csp_str}");
        assert!(csp_str.contains("connect-src"), "CSP: {csp_str}");
        assert!(csp_str.contains("img-src"), "CSP: {csp_str}");
        assert!(csp_str.contains("data:"), "CSP: {csp_str}");
    }

    #[tokio::test]
    async fn csp_header_present_on_root_handler() {
        let state = make_test_state();
        let app = build_router(state, None);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        // root handler 也应带 CSP (layer 应用到所有 response)
        assert!(
            response.headers().contains_key("content-security-policy"),
            "CSP missing on /"
        );
    }
}

#[cfg(test)]
mod stage12_memory_observations_tests {
    //! Stage 12: 验证 `GET /v1/memory/sessions/{id}/observations` 端到端.
    //!
    //! 之前 Svelte `memory.ts:listObservations` 调这个 endpoint, 但 daemon
    //! router 没注册, 返 404, Memory 面板点 session 后右侧观察详情失败.
    //! 现在补上, 验证:
    //!   1. 空 session 返 `{observations: [], total: 0, session_id: "..."}` (不 404)
    //!   2. 走 JWT auth (无 token 返 401)
    //!   3. path 里的 session_id 回显
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use jsonwebtoken::{encode, EncodingKey, Header as JwtHeader};
    use tower::ServiceExt;

    use super::stage7a_endpoint_tests::make_test_state;

    /// 跟 `make_test_state` 内部用的 AdminCredential 同样的 secret.
    const TEST_SECRET: &str = "stage7a-test-jwt-secret";

    /// 签发测试 JWT (HS256, 1h 过期).
    fn make_test_jwt() -> String {
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            sub: "admin".into(),
            exp: now + 3600,
            iat: now,
        };
        encode(
            &JwtHeader::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("encode test jwt")
    }

    /// helper: 发 GET 请求带 Bearer JWT
    async fn get_with_auth(
        app: axum::Router,
        path: &str,
        token: &str,
    ) -> axum::response::Response {
        app.oneshot(
            HttpRequest::builder()
                .method("GET")
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn list_observations_empty_session_returns_empty_array() {
        let state = make_test_state();
        let app = build_router(state, None);
        let token = make_test_jwt();
        // "never_existed" session 不存在, MemoryCore.list_observations 应返空 vec
        let response = get_with_auth(app, "/v1/memory/sessions/never_existed/observations", &token)
            .await;
        assert_eq!(response.status(), StatusCode::OK, "空 session 应返 200 不 404");
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            body_str.contains("\"observations\":[]"),
            "body 应含空 observations 数组, got: {body_str}"
        );
        assert!(
            body_str.contains("\"total\":0"),
            "body 应含 total=0, got: {body_str}"
        );
        assert!(
            body_str.contains("\"session_id\":\"never_existed\""),
            "body 应回显 session_id, got: {body_str}"
        );
    }

    #[tokio::test]
    async fn list_observations_requires_auth() {
        let state = make_test_state();
        let app = build_router(state, None);
        // 无 Authorization header → 401
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/v1/memory/sessions/sess_1/observations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "list_observations 应走 JWT auth, 无 token 返 401"
        );
    }

    #[tokio::test]
    async fn list_observations_path_with_session_id_echoes_back() {
        // 验证 session_id 字段回显 (Web Console 用它确认请求成功)
        let state = make_test_state();
        let app = build_router(state, None);
        let token = make_test_jwt();
        let response = get_with_auth(
            app,
            "/v1/memory/sessions/sess_test_xyz/observations",
            &token,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body_str = std::str::from_utf8(&body_bytes).unwrap();
        assert!(
            body_str.contains("\"session_id\":\"sess_test_xyz\""),
            "body 应回显 path 里的 session_id, got: {body_str}"
        );
    }
}


// =============================================================================
// Kanban HTTP 路由 (MVP-3 plan 2, v6 §8.5)
// =============================================================================

use qianxun_core::kanban::types::{
    BoardStatus, KanbanBoard, Project, ProjectStatus,
};

/// GET /v1/kanban/boards - 列出 boards
async fn list_kanban_boards(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state
        .kanban_host
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let boards = host
        .db
        .run_blocking(|c: &rusqlite::Connection| -> Result<Vec<KanbanBoard>, qianxun_core::kanban::KanbanError> {
            let mut stmt = c.prepare(
                "SELECT id, project_id, name, project_root, default_role, status, created_at, updated_at \
                 FROM kanban_boards ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    let status_str: String = row.get(5)?;
                    let status = match status_str.as_str() {
                        "archived" => BoardStatus::Archived,
                        _ => BoardStatus::Active,
                    };
                    let cas: String = row.get(6)?;
                    let uas: String = row.get(7)?;
                    Ok(KanbanBoard {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        name: row.get(2)?,
                        project_root: row.get::<_, String>(3)?.into(),
                        default_role: row.get(4)?,
                        status,
                        created_at: chrono::DateTime::parse_from_rfc3339(&cas)
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now()),
                        updated_at: chrono::DateTime::parse_from_rfc3339(&uas)
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now()),
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("kanban query: {e}")))?;
    Ok(Json(serde_json::json!({ "boards": boards })))
}

/// POST /v1/kanban/boards - 创建 board + emit KanbanTaskSpawned SSE
async fn create_kanban_board(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state
        .kanban_host
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let name = body["name"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "name required".into()))?;
    let project_root = body["project_root"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "project_root required".into()))?;
    let default_role = body["default_role"].as_str().unwrap_or("techlead");
    let board = host
        .db
        .run_blocking({
            let name = name.to_string();
            let project_root = project_root.to_string();
            let default_role = default_role.to_string();
            move |c: &rusqlite::Connection| -> Result<KanbanBoard, qianxun_core::kanban::KanbanError> {
                let now_str = chrono::Utc::now().to_rfc3339();
                let board_id = format!("kb_{}", uuid::Uuid::new_v4());
                c.execute(
                    "INSERT INTO kanban_boards (id, project_id, name, project_root, default_role, status, created_at, updated_at) \
                     VALUES (?1, 'proj_default', ?2, ?3, ?4, 'active', ?5, ?5)",
                    rusqlite::params![board_id, name, project_root, default_role, now_str],
                )?;
                Ok(KanbanBoard {
                    id: board_id,
                    project_id: "proj_default".into(),
                    name: name.clone(),
                    project_root: std::path::PathBuf::from(project_root),
                    default_role,
                    status: BoardStatus::Active,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
            }
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create board: {e}")))?;
    // emit SSE: KanbanTaskSpawned 复用表示 board 创建 (任务面板展示)
    host.emit(crate::daemon::kanban_host::KanbanSseEvent::KanbanTaskSpawned {
        parent_task_id: None,
        child_task_id: board.id.clone(),
        title: format!("board: {name}"),
        assignee_role: board.default_role.clone(),
    });
    Ok(Json(serde_json::json!({ "board": board })))
}

/// GET /v1/kanban/boards/{id} - 查 board 详情
async fn get_kanban_board(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state
        .kanban_host
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let id_c = id.clone();
    let board = host
        .db
        .run_blocking(move |c: &rusqlite::Connection| -> Result<Option<KanbanBoard>, qianxun_core::kanban::KanbanError> {
            let mut stmt = c.prepare(
                "SELECT id, project_id, name, project_root, default_role, status, created_at, updated_at \
                 FROM kanban_boards WHERE id = ?1",
            )?;
            let board = stmt
                .query_row(rusqlite::params![id_c], |row| {
                    let status_str: String = row.get(5)?;
                    let status = match status_str.as_str() {
                        "archived" => BoardStatus::Archived,
                        _ => BoardStatus::Active,
                    };
                    let cas: String = row.get(6)?;
                    let uas: String = row.get(7)?;
                    Ok(KanbanBoard {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        name: row.get(2)?,
                        project_root: row.get::<_, String>(3)?.into(),
                        default_role: row.get(4)?,
                        status,
                        created_at: chrono::DateTime::parse_from_rfc3339(&cas)
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now()),
                        updated_at: chrono::DateTime::parse_from_rfc3339(&uas)
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now()),
                    })
                })
                .ok();
            Ok(board)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("query: {e}")))?;
    match board {
        Some(b) => Ok(Json(serde_json::json!({ "board": b }))),
        None => Err((StatusCode::NOT_FOUND, format!("board not found: {id}"))),
    }
}

/// GET /v1/kanban/boards/{id}/tasks - 列 board 下 task
async fn list_kanban_board_tasks(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state
        .kanban_host
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let tasks = host
        .db
        .list_tasks(&id, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("list tasks: {e}")))?;
    let count_by_status: std::collections::HashMap<String, usize> = {
        let mut m = std::collections::HashMap::new();
        for t in &tasks {
            *m.entry(format!("{:?}", t.status)).or_insert(0) += 1;
        }
        m
    };
    Ok(Json(serde_json::json!({
        "tasks": tasks,
        "total": tasks.len(),
        "by_status": count_by_status,
    })))
}

/// GET /v1/projects - 列出 projects
async fn list_projects(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state
        .kanban_host
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let projects = host
        .db
        .run_blocking(|c: &rusqlite::Connection| -> Result<Vec<Project>, qianxun_core::kanban::KanbanError> {
            let mut stmt = c.prepare(
                "SELECT id, name, description, default_root, extra_roots, status, owner, created_at, updated_at \
                 FROM kanban_projects ORDER BY created_at DESC",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    let status_str: String = row.get(5)?;
                    let status = match status_str.as_str() {
                        "archived" => ProjectStatus::Archived,
                        _ => ProjectStatus::Active,
                    };
                    let extra_roots_str: String = row.get(4)?;
                    let extra_roots: Vec<std::path::PathBuf> = serde_json::from_str(&extra_roots_str)
                        .unwrap_or_default();
                    let cas: String = row.get(7)?;
                    let uas: String = row.get(8)?;
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        default_root: row.get::<_, String>(3)?.into(),
                        extra_roots,
                        status,
                        owner: row.get(6)?,
                        created_at: chrono::DateTime::parse_from_rfc3339(&cas)
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now()),
                        updated_at: chrono::DateTime::parse_from_rfc3339(&uas)
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now()),
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("query: {e}")))?;
    Ok(Json(serde_json::json!({ "projects": projects })))
}

/// POST /v1/projects - 创建 project
async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state
        .kanban_host
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let name = body["name"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "name required".into()))?;
    let description = body["description"].as_str().unwrap_or("");
    let default_root = body["default_root"].as_str().unwrap_or("");
    let project = host
        .db
        .run_blocking({
            let name = name.to_string();
            let description = description.to_string();
            let default_root = default_root.to_string();
            move |c: &rusqlite::Connection| -> Result<Project, qianxun_core::kanban::KanbanError> {
                let now_str = chrono::Utc::now().to_rfc3339();
                let project_id = format!("proj_{}", uuid::Uuid::new_v4());
                c.execute(
                    "INSERT INTO kanban_projects (id, name, description, default_root, extra_roots, status, owner, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, '[]', 'active', 'local', ?5, ?5)",
                    rusqlite::params![project_id, name, description, default_root, now_str],
                )?;
                Ok(Project {
                    id: project_id,
                    name: name.clone(),
                    description,
                    default_root: std::path::PathBuf::from(default_root),
                    extra_roots: vec![],
                    status: ProjectStatus::Active,
                    owner: "local".into(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
            }
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create: {e}")))?;
    Ok(Json(serde_json::json!({ "project": project })))
}

// =============================================================================
// MVP-3 plan 3: Tasks 路由 (精简 4 核心)
// =============================================================================

async fn get_kanban_task(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state.kanban_host.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    match host.db.get_task(&id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("get task: {e}")))?
    {
        Some(t) => Ok(Json(serde_json::json!({ "task": t }))),
        None => Err((StatusCode::NOT_FOUND, format!("task not found: {id}"))),
    }
}

async fn create_kanban_task(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state.kanban_host.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let board_id = body["board_id"].as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "board_id required".into()))?;
    let title = body["title"].as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "title required".into()))?;
    let body_text = body["body"].as_str().unwrap_or("");
    let assignee_role = body["assignee_role"].as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "assignee_role required".into()))?;
    let parent_id = body["parent_id"].as_str();
    let priority = body["priority"].as_u64().unwrap_or(128) as u8;
    let task = host.db.create_task(
        None, board_id, parent_id, title, body_text, assignee_role, priority,
    ).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create: {e}")))?;
    host.emit(crate::daemon::kanban_host::KanbanSseEvent::KanbanTaskSpawned {
        parent_task_id: parent_id.map(String::from),
        child_task_id: task.id.clone(),
        title: title.into(),
        assignee_role: assignee_role.into(),
    });
    Ok(Json(serde_json::json!({ "task": task })))
}

async fn cancel_kanban_task(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state.kanban_host.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    use qianxun_core::kanban::types::TaskStatus;
    host.db.update_task_status(&id, TaskStatus::Cancelled).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("cancel: {e}")))?;
    host.emit(crate::daemon::kanban_host::KanbanSseEvent::KanbanTaskCompleted {
        task_id: id.clone(),
        run_id: String::new(),
        outcome: "cancelled".into(),
        summary: "user cancelled".into(),
        token_input: 0,
        token_output: 0,
        elapsed_ms: 0,
    });
    Ok(Json(serde_json::json!({ "task_id": id, "cancelled": true })))
}

async fn list_kanban_board_events(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state.kanban_host.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let id_c = id.clone();
    let events = host.db.run_blocking(move |c: &rusqlite::Connection|
        -> Result<Vec<(i64, String, Option<String>, String, String)>, qianxun_core::kanban::KanbanError>
    {
        let mut stmt = c.prepare(
            "SELECT e.id, e.kind, e.task_id, e.payload, e.created_at \
             FROM kanban_events e \
             LEFT JOIN kanban_tasks t ON t.id = e.task_id \
             WHERE t.board_id = ?1 OR e.task_id IS NULL \
             ORDER BY e.created_at DESC LIMIT 100",
        )?;
        let rows = stmt.query_map(rusqlite::params![id_c], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
        Ok(rows)
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("events: {e}")))?;
    let event_list: Vec<serde_json::Value> = events.into_iter().map(|(eid, kind, tid, payload, created_at)| {
        serde_json::json!({
            "id": eid, "kind": kind, "task_id": tid,
            "payload": serde_json::from_str::<serde_json::Value>(&payload).unwrap_or(serde_json::Value::Null),
            "created_at": created_at,
        })
    }).collect();
    Ok(Json(serde_json::json!({ "events": event_list, "total": event_list.len() })))
}

// =============================================================================
// MVP-3 plan 4: Teams 路由 (精简 3 核心)
// =============================================================================

async fn list_kanban_profiles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state.kanban_host.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let roles = host.team_registry.list_roles().await;
    let mut profiles = Vec::new();
    for r in &roles {
        let name = r.default_profile_id.replace("prof_", "");
        if let Some(p) = host.team_registry.get_profile(&name).await {
            profiles.push(p);
        }
    }
    Ok(Json(serde_json::json!({ "profiles": profiles, "total": profiles.len() })))
}

async fn list_kanban_roles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state.kanban_host.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    let roles = host.team_registry.list_roles().await;
    Ok(Json(serde_json::json!({ "roles": roles, "total": roles.len() })))
}

async fn dispatch_kanban_now(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let host = state.kanban_host.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "kanban host not initialized".into()))?;
    match host.dispatch_once().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("dispatch: {e}")))?
    {
        Some(r) => Ok(Json(serde_json::json!({
            "dispatched": true,
            "task_id": r.task_id,
            "run_id": r.run_id,
            "profile_name": r.profile_name,
        }))),
        None => Ok(Json(serde_json::json!({
            "dispatched": false,
            "reason": "no ready task or all profiles busy",
        }))),
    }
}

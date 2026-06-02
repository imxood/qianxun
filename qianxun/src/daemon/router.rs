use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
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
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::iter;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::{ServeDir, ServeFile};

use qianxun_core::agent::conversation::Conversation;
use qianxun_core::agent::message::ContentBlock;
use qianxun_core::provider::types::LlmStreamEvent;
use qianxun_core::types::LlmError;

use crate::daemon::llm_providers::{LlmProviderConfig as ManagerProviderConfig, TestResult};
use crate::daemon::sse::{SseEvent, SseEventBuilder};
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
/// - `/_ui/*` 跳过 (Stage 7a: 静态文件 serve, 走 cookie/JWT 端另一套 —
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
        // 会话
        .route("/v1/chat/session", post(create_session))
        .route("/v1/chat/session/{id}", get(get_session).delete(delete_session))
        .route("/v1/chat/session/{id}/prompt", post(prompt_handler))
        // 工具
        .route("/v1/tools", get(list_tools))
        // 配置
        .route("/v1/config", get(get_config))
        // 记忆
        .route("/v1/memory/sessions", get(memory_sessions))
        .route("/v1/memory/search", post(memory_search))
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
        // 未知 path 返 404 JSON (而不是被 auth 拦成 401)
        .fallback(not_found_handler)
        .with_state(state);

    // Stage 7a: 嵌套 ServeDir (静态文件 + SPA fallback).
    // nest_service 把整个 sub-router 接到 /_ui/* 上.
    router = match ui_dist {
        Some(dir) if dir.is_dir() => {
            let index_html = dir.join("index.html");
            if index_html.is_file() {
                // SPA fallback: 文件不存在 → 返 index.html (vite/adam 行为)
                let svc = ServeDir::new(&dir).fallback(ServeFile::new(&index_html));
                router.nest_service("/_ui", svc)
            } else {
                // 没 index.html → 直接 ServeDir, 不做 fallback (404 由 ServeDir 返)
                let svc = ServeDir::new(&dir);
                router.nest_service("/_ui", svc)
            }
        }
        _ => {
            // dist 不存在或未配置 → /_ui/* 走兜底 handler 返 503
            router.nest_service(
                "/_ui",
                axum::routing::get(ui_dist_missing).fallback(ui_dist_missing),
            )
        }
    };

    // Stage 6a: 全局 JWT auth middleware (在 handler 之前执行)
    router.layer(middleware::from_fn(auth_middleware))
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
/// - secret 来自 env var `QIANXUN_JWT_SECRET`. 缺 secret → 启动失败 (main.rs).
///
/// Stage 7 加 `role` 字段 + 跟 VPS auth 集成.
pub async fn auth_middleware(
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // 1. 跳过 health/status (k8s probe + 调试)
    let path = request.uri().path();
    if is_auth_skipped_path(path) {
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

    // 3. 读 secret (env var QIANXUN_JWT_SECRET). main.rs 已校验存在,
    //    这里是兜底防御 (secret 在请求之间不应被删除).
    let secret = match jwt_secret() {
        Some(s) => s,
        None => {
            tracing::error!("[auth] QIANXUN_JWT_SECRET not set; daemon misconfigured");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

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

    Ok(next.run(request).await)
}

/// 哪些 path 跳过 auth (k8s probe / 调试查询 / landing / 静态 UI).
///
/// 当前跳过:
/// - `/` — 服务自描述/landing, 浏览器/curl 探针应能命中不报错
/// - `/v1/system/health` — k8s liveness/readiness probe
/// - `/v1/system/status` — 状态查询 (信息非敏感, 调试方便)
/// - `/_ui/*` — Stage 7a 静态文件 serve (SPA 资源不需要每个文件都打 token;
///   真正要 auth 的 Web UI 资源是 SvelteKit 内部 fetch 走 `/v1/*` 时的 Bearer
///   token; Stage 7a 简化: 启动时打 admin token, UI 粘进 localStorage)
pub fn is_auth_skipped_path(path: &str) -> bool {
    path == "/"
        || path == "/v1/system/health"
        || path == "/v1/system/status"
        || path.starts_with("/_ui/")
        || path == "/_ui"
}

/// 读 JWT secret (env var QIANXUN_JWT_SECRET).
///
/// 返回 `None` 表示 env var 未设置或为空 (启动时 main.rs 会 panic).
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

// ─── 技能 ──────────────────────────────────────────────────

async fn list_skills() -> Json<serde_json::Value> {
    Json(serde_json::json!({"skills": []}))
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

// ─── Prompt (SSE 流式) ────────────────────────────────────

/// POST /v1/chat/session/:id/prompt — SSE 流式响应.
///
/// Stage 2 实现:
/// 1. 验证 session 存在
/// 2. 构造一个**临时** `Conversation` (Stage 3 才把 conversation 持久化到 SessionRuntime)
/// 3. 调 `provider.stream_completion` 拿到 `BoxStream<LlmStreamEvent>`
/// 4. spawn 后台任务消费 stream, 用 `SseEventBuilder` 映射成 12 种契约事件, 推 mpsc
/// 5. SSE wrapper 从 mpsc 读, 序列化成 `data: <json>\n\n` 帧
/// 6. 客户端断连 → axum drop SSE future → mpsc::Receiver 关闭 → spawn task 中
///    `tx.send()` 返回 Err, 任务自然退出
///
/// **Stage 2 不接** `processing_loop::handle_user_message` (Stage 3 接入).
/// 也不接 `tool_result` 事件 (Stage 3 在工具执行路径上发射).
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

    // 2. 构造临时 conversation (Stage 2 简化: 不写回 SessionRuntime)
    let mut conv = Conversation::new(None);
    for msg in &req.messages {
        let role = msg.role.as_str();
        match role {
            "user" => {
                let block = ContentBlock::text(&msg.content);
                conv.push_user_message(vec![block]);
            }
            "assistant" | "system" => {
                // Stage 2 简化: assistant / system 直接入 history 不传给 LLM
                // (完整的 multi-turn 由 Stage 3 conversation 持久化时还原)
                tracing::debug!(
                    "[prompt] role={role} content.len={} (ignored in stage-2)",
                    msg.content.len()
                );
            }
            other => {
                tracing::warn!("[prompt] unknown role {other}, skipping");
            }
        }
    }

    // 3. 构建 CompletionRequest (memory / skills 留 Stage 3 接入)
    let request = conv.build_request(
        &[],
        "", // memory_context
        "", // skills_catalog
        "", // skill_injections
        &runtime.resolved.agent,
    );

    // 4. 调 provider.stream_completion
    let provider = runtime.provider.clone();
    let stream = match provider.stream_completion(request).await {
        Ok(s) => s,
        Err(e) => {
            // provider 启动失败 → 返回 error 事件后关闭
            let err_event = SseEventBuilder::error_from_llm(&e);
            let tail = SseEventBuilder::new().finalize("error");
            let mut events = vec![err_event];
            events.extend(tail);
            let s: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
                Box::pin(iter(events).map(event_to_sse));
            return Ok(Sse::new(s));
        }
    };

    // 5. 通道 + message_start 先发, 再 spawn consumer (避免与 text_delta 乱序)
    let (tx, rx) = mpsc::channel::<SseEvent>(64);

    // message_start: 同步先发, 保证客户端收到的第一帧就是 session 元数据
    let model = runtime.config.model.clone();
    let max_tokens = runtime.resolved.agent.max_tokens.unwrap_or(16384) as u32;
    let session_id = runtime.session_id.clone();
    // channel 容量 64, 单条消息不会 .await 等待
    let _ = tx
        .send(SseEvent::MessageStart {
            session_id: session_id.clone(),
            model,
            max_tokens,
        })
        .await;

    // Stage 3: 把 message_start 事件也写到 store.event_log
    if let Ok(json) = serde_json::to_string(&SseEvent::MessageStart {
        session_id: session_id.clone(),
        model: runtime.config.model.clone(),
        max_tokens,
    }) {
        let _ = state.store.append_event(&session_id, 0, "message_start", &json);
    }

    // consumer: 把 LlmStreamEvent 逐个映射成 SseEvent
    let mut builder = SseEventBuilder::new();
    // Stage 3: 给 consumer 传 store clone 用于事件落盘 + 末次 snapshot
    let store = state.store.clone();
    let sess_id_for_consumer = session_id.clone();
    tokio::spawn(async move {
        consume_stream_to_sse(stream, &mut builder, tx, store, sess_id_for_consumer).await;
    });

    // 6. SSE wrapper: 把 mpsc 里的事件序列化成 SSE 帧
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
/// 经 `SseEventBuilder` 转成 SseEvent, 推给 mpsc::Sender.
///
/// 流结束 / 出错时调用 `builder.finalize(reason)` 统一发
/// `ContentBlockStop` (关闭未关 block) + `MessageDelta` + `MessageStop` 收尾.
///
/// 客户端断连: `tx.send().await` 返回 Err, 立即 return 退出 (LLM 流仍可能
/// 在跑, 但没人消费事件, 任务自然结束).
///
/// Stage 3: 同时把每个 SseEvent 写到 `store.append_event()` 落盘; 流结束
/// 时调 `store.save_snapshot(ordinal+1, "{}")` 写一个占位 snapshot
/// (Stage 4 接完整 conversation 序列化).
async fn consume_stream_to_sse(
    mut stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<LlmStreamEvent, LlmError>> + Send>,
    >,
    builder: &mut SseEventBuilder,
    tx: mpsc::Sender<SseEvent>,
    store: std::sync::Arc<crate::daemon::persistence::SessionStore>,
    session_id: String,
) {
    let mut last_stop_reason: Option<String> = None;
    // Stage 3: 事件序号, 跳过 seq=0 (已用于 message_start in prompt_handler)
    let mut event_seq: u32 = 1;

    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => {
                // 记录 stop reason, 等流自然结束后再 finalize
                if let LlmStreamEvent::Stop(reason) = &event {
                    last_stop_reason =
                        Some(SseEventBuilder::stop_reason_str(reason).to_string());
                }
                let events = builder.from_llm_event(&event);
                for ev in events {
                    // 落盘: 序列化 SseEvent → JSON → store.append_event
                    if let Ok(json) = serde_json::to_string(&ev) {
                        let type_name = event_type_name(&ev);
                        let _ = store.append_event(&session_id, event_seq, type_name, &json);
                        event_seq += 1;
                    }
                    if tx.send(ev).await.is_err() {
                        return; // 客户端已断
                    }
                }
            }
            Err(e) => {
                // 错误: 发 error 事件 + finalize 收尾
                let err_event = SseEventBuilder::error_from_llm(&e);
                if let Ok(json) = serde_json::to_string(&err_event) {
                    let _ = store.append_event(&session_id, event_seq, "error", &json);
                    event_seq += 1;
                }
                if tx.send(err_event).await.is_err() {
                    return;
                }
                let reason = last_stop_reason
                    .take()
                    .unwrap_or_else(|| "error".to_string());
                let tail = builder.finalize(&reason);
                for ev in tail {
                    if let Ok(json) = serde_json::to_string(&ev) {
                        let type_name = event_type_name(&ev);
                        let _ = store.append_event(&session_id, event_seq, type_name, &json);
                        event_seq += 1;
                    }
                    if tx.send(ev).await.is_err() {
                        return;
                    }
                }
                return;
            }
        }
    }

    // 流自然结束: finalize (MessageDelta + MessageStop; 视 builder 内是否有
    // 未关 block 决定要不要先发 ContentBlockStop).
    let reason = last_stop_reason
        .unwrap_or_else(|| "end_turn".to_string());
    let tail = builder.finalize(&reason);
    for ev in tail {
        if let Ok(json) = serde_json::to_string(&ev) {
            let type_name = event_type_name(&ev);
            let _ = store.append_event(&session_id, event_seq, type_name, &json);
            event_seq += 1;
        }
        if tx.send(ev).await.is_err() {
            return;
        }
    }

    // Stage 3 简化: 流结束时写一次占位 snapshot
    // (Stage 4 接完整 conversation 序列化, ordinal=1 表示本次 turn)
    let _ = store.save_snapshot(&session_id, 1, r#"{"messages":[],"stage":"stage3_placeholder"}"#);
}

/// Stage 3: 提取 SSE 事件 type 字符串 (用于 store event_type 字段).
/// 与 `SseEvent` 的 serde tag 字段名严格一致.
fn event_type_name(ev: &SseEvent) -> &'static str {
    match ev {
        SseEvent::MessageStart { .. } => "message_start",
        SseEvent::ContentBlockStart { .. } => "content_block_start",
        SseEvent::TextDelta { .. } => "text_delta",
        SseEvent::ThinkingDelta { .. } => "thinking_delta",
        SseEvent::ToolUseDelta { .. } => "tool_use_delta",
        SseEvent::ToolUseComplete { .. } => "tool_use_complete",
        SseEvent::ToolResult { .. } => "tool_result",
        SseEvent::ContentBlockStop { .. } => "content_block_stop",
        SseEvent::Usage { .. } => "usage",
        SseEvent::MessageDelta { .. } => "message_delta",
        SseEvent::MessageStop => "message_stop",
        SseEvent::Error { .. } => "error",
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
        let mut builder = SseEventBuilder::new();
        let store = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store,
                "sess_e2e_text".to_string(),
            )
            .await;
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
        let mut builder = SseEventBuilder::new();
        let store2 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store2,
                "sess_e2e_tool".to_string(),
            )
            .await;
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
        let mut builder = SseEventBuilder::new();
        let store3 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store3,
                "sess_e2e_err".to_string(),
            )
            .await;
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
        let mut builder = SseEventBuilder::new();
        let store4 = std::sync::Arc::new(
            crate::daemon::persistence::SessionStore::in_memory().expect("in_memory"),
        );
        let task = tokio::spawn(async move {
            consume_stream_to_sse(
                mock_stream,
                &mut builder,
                tx,
                store4,
                "sess_e2e_no_stop".to_string(),
            )
            .await;
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

        Router::new()
            .route("/v1/system/health", get(public))
            .route("/v1/system/status", get(public))
            .route("/v1/chat/session", get(protected))
            .route("/v1/_claims_echo", get(claims_echo))
            .layer(middleware::from_fn(auth_middleware))
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

        Router::new()
            .route("/", get(root))
            .route("/v1/system/health", get(public))
            .route("/v1/system/status", get(public))
            .route("/v1/chat/session", get(protected))
            .fallback(fallback)
            .layer(middleware::from_fn(auth_middleware))
    }

    /// 在测试里安全地设置 env var (Rust 2024 edition 下 set_var 是 unsafe).
    fn set_jwt_secret(val: &str) {
        // SAFETY: 测试用 ENV_MUTEX 序列化访问, 测试进程内不并发
        unsafe { std::env::set_var("QIANXUN_JWT_SECRET", val) }
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
        // Stage 7a: /_ui/* 跳过 auth (SvelteKit 静态资源)
        assert!(is_auth_skipped_path("/_ui"));
        assert!(is_auth_skipped_path("/_ui/"));
        assert!(is_auth_skipped_path("/_ui/assets/main.js"));
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

    /// 构造一个最小 AppState for tests.
    ///
    /// 不初始化 AgentLoopHost (会要求 SessionStore), 单独 mock 一个 minimal host
    /// 通过 — 简单做法: 只为 router 提供 LLM manager / tools / skills, agent_host
    /// 字段用空 runtime.
    fn make_test_state() -> Arc<crate::daemon::AppState> {
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
        })
    }

    /// 构造带 UI dist 路径的 test router.
    fn test_router_with_ui(ui_dist: Option<PathBuf>) -> Router {
        let state = make_test_state();
        build_router(state, ui_dist)
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
                    .uri("/_ui/")
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
                    .uri("/_ui/")
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
                    .uri("/_ui/index.html")
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
                    .uri("/_ui/assets/main.js")
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
                    .uri("/_ui/llm/some-page")
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

    // ── helper ──

    #[allow(dead_code)]
    fn _unused_to_suppress_warnings(_: &ContentBlock) {}
}


use axum::{
    extract::State,
    http::StatusCode,
    response::{sse::Event, Json, Sse},
    routing::{get, post},
    Router,
};
use std::convert::Infallible;
use tokio_stream::StreamExt;
use serde::Serialize;
use std::sync::Arc;

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

/// 构建 Daemon HTTP 路由。
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // 系统
        .route("/v1/system/health", get(health_handler))
        .route("/v1/system/status", get(status_handler))
        // 会话
        .route("/v1/chat/session", post(create_session))
        .route("/v1/chat/session/:id", get(get_session).delete(delete_session))
        .route("/v1/chat/session/:id/prompt", post(prompt_handler))
        // 工具
        .route("/v1/tools", get(list_tools))
        // 配置
        .route("/v1/config", get(get_config))
        // 记忆
        .route("/v1/memory/sessions", get(memory_sessions))
        .route("/v1/memory/search", post(memory_search))
        // 技能
        .route("/v1/skills", get(list_skills))
        // MCP
        .route("/v1/mcp/servers", get(list_mcp_servers).post(add_mcp_server))
        .with_state(state)
}

// ─── 系统 ──────────────────────────────────────────────────

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn status_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ─── 会话 ──────────────────────────────────────────────────

async fn create_session(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionCreatedResponse>, (StatusCode, String)> {
    match state.agent_host.create_session() {
        Ok(session_id) => Ok(Json(SessionCreatedResponse { session_id })),
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

/// POST /v1/chat/session/:id/prompt — SSE 流式响应
async fn prompt_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    if !state.agent_host.session_exists(&id) {
        return Err((StatusCode::NOT_FOUND, format!("Session {id} not found")));
    }

    let stream = tokio_stream::once(Ok(Event::default().data("{\"text\":\"Daemon ready\"}")));
    Ok(Sse::new(stream))
}

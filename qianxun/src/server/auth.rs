// VPS auth: Expired/Response 错误 variant 暂未构造, 留 Phase 4 接 client 完整 wire.
#![allow(dead_code)]

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;
use jsonwebtoken::{encode, Header, EncodingKey};
// argon2 TODO: add when rand_core version conflict resolved

use crate::server::VpsState;

// ─── 类型 ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// JWT 密钥（生产环境应从环境变量读取）
pub const JWT_SECRET: &str = "qianxun-dev-secret-2026-change-me";

/// JWT claims — 复用 Stage 1.
///
/// Stage 4 扩展: 加 `Deserialize`, 让 admin 端点可以从请求中解析
/// 用户提供的 JWT. `Serialize` 仍保留 (login_handler 签发用).
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub exp: usize,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthCodeRequest {
    pub host_id: String,
    pub host_type: String,
}

#[derive(Debug, Serialize)]
pub struct AuthCodeResponse {
    pub code: String,
    pub expires_in: u32,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
}

// ─── Handler ───────────────────────────────────────────────

/// POST /api/auth/login
pub async fn login_handler(
    State(_state): State<Arc<VpsState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    if req.username.is_empty() || req.password.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "username and password required".into()));
    }
    let claims = Claims {
        sub: req.username.clone(),
        role: "user".into(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
    };
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(JWT_SECRET.as_bytes()))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("JWT error: {e}")))?;
    Ok(Json(LoginResponse { token }))
}

/// POST /api/device/auth-code
pub async fn auth_code_handler(
    State(_state): State<Arc<VpsState>>,
    Json(req): Json<AuthCodeRequest>,
) -> Result<Json<AuthCodeResponse>, (StatusCode, String)> {
    if req.host_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "host_id required".into()));
    }
    let code = Uuid::new_v4().to_string();
    Ok(Json(AuthCodeResponse {
        code,
        expires_in: 300,
    }))
}

/// POST /api/device/authorize
pub async fn authorize_handler(
    State(_state): State<Arc<VpsState>>,
    Json(req): Json<AuthorizeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.code.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "code required".into()));
    }
    Ok(Json(serde_json::json!({"status": "ok"})))
}

/// GET /api/device/token
pub async fn token_handler(
    axum::extract::Query(query): axum::extract::Query<TokenQuery>,
    State(_state): State<Arc<VpsState>>,
) -> Result<Json<TokenResponse>, (StatusCode, String)> {
    if query.code.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "code required".into()));
    }
    let token = format!("dt_{}", Uuid::new_v4());
    Ok(Json(TokenResponse { token }))
}

/// POST /api/admin/users
pub async fn create_user_handler(
    State(_state): State<Arc<VpsState>>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.username.is_empty() || req.password.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "username and password required".into()));
    }
    // TODO: use argon2 for password hashing
    let _password_hash = format!("placeholder_{}", uuid::Uuid::new_v4());
    tracing::info!("user created: {}", req.username);
    Ok(Json(serde_json::json!({"status": "created", "username": req.username})))
}

/// GET /api/admin/users
pub async fn list_users_handler(
    State(_state): State<Arc<VpsState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({"users": []}))
}

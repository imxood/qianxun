pub mod auth;

use axum::{
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::sync::Arc;

/// VPS Server 共享状态。
pub struct VpsState {
    pub db: std::sync::Mutex<rusqlite::Connection>,
}

/// 启动 VPS Server。
pub async fn run(port: u16) -> anyhow::Result<()> {
    tracing::info!("VPS Server starting on 0.0.0.0:{port}");

    let db = std::sync::Mutex::new(init_db()?);
    let state = Arc::new(VpsState { db });

    let app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/auth/login", post(auth::login_handler))
        .route("/api/device/auth-code", post(auth::auth_code_handler))
        .route("/api/device/authorize", post(auth::authorize_handler))
        .route("/api/device/token", get(auth::token_handler))
        .route("/api/admin/users", post(auth::create_user_handler).get(auth::list_users_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// 初始化 VPS 数据库。
fn init_db() -> anyhow::Result<rusqlite::Connection> {
    let data_dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let db_path = data_dir.join("qianxun").join("vps.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = rusqlite::Connection::open(&db_path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'user',
            created_at TEXT NOT NULL,
            disabled INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS devices (
            id TEXT PRIMARY KEY,
            host_id TEXT NOT NULL,
            user_id TEXT NOT NULL REFERENCES users(id),
            token_hash TEXT NOT NULL,
            host_type TEXT NOT NULL DEFAULT 'unknown',
            status TEXT NOT NULL DEFAULT 'offline',
            last_seen TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS auth_codes (
            code TEXT PRIMARY KEY,
            device_id TEXT NOT NULL,
            user_id TEXT,
            expires_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending'
        );",
    )?;
    Ok(conn)
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

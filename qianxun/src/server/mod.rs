//! VPS Server 入口 + 共享状态 + 路由.
//!
//! Stage 1 (本版本) 的范围:
//! - 5 个 REST 路由 (auth/device/admin) — 维持 v0.2 行为, 暂时不接完整 logic
//! - 1 个 WebSocket 端点 (`/api/ws`) — 接受升级, 立即 close
//! - WsHub 注册表 — 内存索引, 3 个 API (register / unregister / push_*)
//! - WsFrame enum — 12 variant, 仅定义 + serde, 暂不派发
//!
//! Stage 2-3 计划:
//! - auth: device_token 验证 + JWT 解析 → 决定 ConnectionType
//! - handler: WsFrame 派发到具体 action
//! - heartbeat manager: 30s ping, 90s timeout
//! - outbox: 重连缓冲

pub mod auth;
pub mod messages;
pub mod ws_hub;

use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::sync::Arc;

pub use ws_hub::{ConnectionType, HubStats, WsHub};

/// VPS Server 共享状态。
pub struct VpsState {
    pub db: std::sync::Mutex<rusqlite::Connection>,
    /// Stage 1 新增: WebSocket Hub (连接注册表 + 路由).
    pub ws_hub: Arc<WsHub>,
}

/// 启动 VPS Server。
pub async fn run(port: u16) -> anyhow::Result<()> {
    tracing::info!("VPS Server starting on 0.0.0.0:{port}");

    let db = std::sync::Mutex::new(init_db()?);
    let state = Arc::new(VpsState {
        db,
        ws_hub: Arc::new(WsHub::new()),
    });

    let app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/auth/login", post(auth::login_handler))
        .route("/api/device/auth-code", post(auth::auth_code_handler))
        .route("/api/device/authorize", post(auth::authorize_handler))
        .route("/api/device/token", get(auth::token_handler))
        .route("/api/admin/users", post(auth::create_user_handler).get(auth::list_users_handler))
        // Stage 1 WS 端点: 接受升级, 立即 close.
        // Stage 2 接 auth (token 区分 device vs app).
        .route("/api/ws", get(ws_upgrade))
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

// ─── WebSocket (Stage 1 雏形) ─────────────────────────────────────────

/// `GET /api/ws` — WS 升级入口.
///
/// Stage 1 行为: 接受 upgrade, 立刻 `Close(None)`. 验证 token (device_token 或 JWT)
/// 在 Stage 2 才接, 见 `docs/30_子项目规划/02-vps-server.md` §11.3.
async fn ws_upgrade(ws: WebSocketUpgrade) -> axum::response::Response {
    ws.on_upgrade(handle_socket)
}

/// Stage 1 雏形: 接受连接, 立刻关闭.
///
/// 真实实现要:
/// 1. 拿 query `?token=...`, 区分 device vs app
/// 2. `WsHub::register` 拿到 conn_id, 启动读循环 (把 `WsFrame` 解析后派发)
/// 3. 启动写循环 (从 `tx` 收 `Message` 写出)
/// 4. 任何一端 EOF → `WsHub::unregister`
async fn handle_socket(mut socket: WebSocket) {
    tracing::info!("ws connection accepted (stage 1: immediate close)");
    // 接受后立即关闭, 验证 Stage 1 WS 端点可达.
    // 真实实现见 `02-vps-server.md` §11.3: 拿 query token 区分 device vs app,
    // 调 `WsHub::register` 拿到 conn_id, 然后跑读循环 + 写循环.
    if let Err(e) = socket
        .send(axum::extract::ws::Message::Close(None))
        .await
    {
        tracing::debug!("ws close send failed: {e}");
    }
}

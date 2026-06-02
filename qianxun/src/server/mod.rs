//! VPS Server 入口 + 共享状态 + 路由.
//!
//! Stage 1 (本版本) 的范围:
//! - 5 个 REST 路由 (auth/device/admin) — 维持 v0.2 行为, 暂时不接完整 logic
//! - 1 个 WebSocket 端点 (`/api/ws`) — 接受升级, 立即 close
//! - WsHub 注册表 — 内存索引, 3 个 API (register / unregister / push_*)
//! - WsFrame enum — 12 variant, 仅定义 + serde, 暂不派发
//!
//! Stage 2 范围 (本版本增量):
//! - `auth_ws::validate_device_token` — 静态白名单 (Stage 3 换 SQLite)
//! - `WsHub::authenticate / handle_heartbeat / register_device` — 派发 WsFrame
//! - `handle_socket` — 完整事件分发: Auth → AuthOk/AuthError, Register → RegisterOk,
//!   Heartbeat → HeartbeatAck
//! - 后台 heartbeat monitor (30s tick) — 90s 无心跳 → 关闭连接 + unregister
//!
//! Stage 3 范围 (本版本增量):
//! - `team_db::TeamDb` — 4 张新表 (team_teams / team_members / team_projects /
//!   team_project_assignments) + devices 表持久化, 11 个 CRUD API
//! - `auth_ws::validate_device_token` — 改用 `TeamDb::lookup_device` 查 SQLite
//!   `devices` 表 (替换 Stage 2 静态白名单)
//! - 7 个新 REST endpoints: teams (3) + projects (3) + assign (1)
//!   见 `02-vps-server.md` §11.2.3 Team/Project 端点 (Stage 3 简化: 不接 admin auth)
//!
//! Stage 4 范围 (本版本增量):
//! - **`admin::require_admin` — JWT 解析 + role 检查 guard** (admin/owner 通过)
//! - **7 个 admin endpoint** (2 已有 + 5 新) 全部加 guard
//! - **`ws_hub::check_rbac` — Team RBAC helper** (同步查 TeamDb)
//! - **`handle_prompt_frame` — Prompt 帧路由 + RBAC 检查** (event_error on forbid)
//! - Dockerfile + docker-compose + .dockerignore (部署)
//!
//! Stage 5+ TODO:
//! - 完整 rate-limit (governor crate)
//! - Outbox + 完整 PendingCommand 跟踪
//! - NodeStatus 广播
//! - 完整 App JWT 验证 + refresh token 轮换

pub mod admin;
pub mod auth;
pub mod auth_ws;
pub mod messages;
pub mod team_db;
pub mod ws_hub;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use messages::WsFrame;
use team_db::{Project, Team, TeamDb, TeamMember};
pub use ws_hub::{check_rbac, ConnectionType, HubStats, RbacError, WsHub};

/// VPS Server 共享状态。
pub struct VpsState {
    pub db: std::sync::Mutex<rusqlite::Connection>,
    /// Stage 1 新增: WebSocket Hub (连接注册表 + 路由).
    pub ws_hub: Arc<WsHub>,
    /// Stage 3 新增: Team 持久化层 (4 张新表 + devices).
    pub team_db: Arc<TeamDb>,
}

/// 启动 VPS Server。
pub async fn run(port: u16) -> anyhow::Result<()> {
    tracing::info!("VPS Server starting on 0.0.0.0:{port}");

    let db = init_db()?;
    // Stage 3: 用 init_db 创建的 Connection 构造 TeamDb, 二者共享同一 SQLite 文件.
    let team_db = Arc::new(TeamDb::from_connection(db)?);
    let ws_hub = Arc::new(WsHub::new(team_db.clone()));
    // 把 db 字段保留 (Stage 1+ 旧 handler 还在用), TeamDb 持有独立 Connection.
    // Stage 4 评估是否合并到一个 Connection.
    let state = Arc::new(VpsState {
        db: std::sync::Mutex::new(team_db_open_standalone()?),
        ws_hub,
        team_db,
    });

    let app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/auth/login", post(auth::login_handler))
        .route("/api/device/auth-code", post(auth::auth_code_handler))
        .route("/api/device/authorize", post(auth::authorize_handler))
        .route("/api/device/token", get(auth::token_handler))
        // Stage 4: admin endpoint — 全部加 admin guard
        .route(
            "/api/admin/users",
            post(admin_create_user_handler).get(admin_list_users_handler),
        )
        .route("/api/admin/teams", get(admin_list_teams_handler))
        .route(
            "/api/admin/teams/:id/members",
            post(admin_add_team_member_handler).delete(admin_remove_team_member_handler),
        )
        .route("/api/admin/projects", get(admin_list_projects_handler))
        .route(
            "/api/admin/projects/:id",
            axum::routing::delete(admin_archive_project_handler),
        )
        // Stage 2: WS 端点接 auth (token 区分 device vs app).
        .route("/api/ws", get(ws_upgrade))
        // Stage 3: Team/Project REST endpoints (7 个, 不接 admin auth).
        .route("/api/teams", post(create_team_handler).get(list_teams_handler))
        .route("/api/teams/:id/members", get(list_members_handler).post(add_member_handler))
        .route("/api/projects", post(create_project_handler).get(list_projects_handler))
        .route("/api/projects/:id/assign", post(assign_project_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// 初始化 VPS 数据库 (Stage 1 已有, 维持原行为, Stage 3 扩展 TeamDb 在 from_connection 中).
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

/// 打开一个独立的 in-memory-style SQLite 连接给 VpsState.db (Stage 1 旧 handler 用).
///
/// Stage 3 临时做法: 直接复用文件路径打开第二个连接, 与 TeamDb 共享同一文件.
/// SQLite WAL 模式支持多读单写, 两个连接可共存 (Stage 4 评估合并).
fn team_db_open_standalone() -> anyhow::Result<rusqlite::Connection> {
    let data_dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let db_path = data_dir.join("qianxun").join("vps.db");
    let conn = rusqlite::Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

// ─── Stage 3: Team/Project REST handlers (无 admin auth) ─────

#[derive(Debug, Deserialize)]
struct CreateTeamRequest {
    name: String,
}

/// POST /api/teams — 创建 team. Stage 3 简化: 无 admin auth 检查.
async fn create_team_handler(
    State(state): State<Arc<VpsState>>,
    Json(req): Json<CreateTeamRequest>,
) -> impl IntoResponse {
    if req.name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name is required".to_string()));
    }
    let team_id = format!("team_{}", Uuid::new_v4());
    state
        .team_db
        .create_team(&team_id, &req.name)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create_team: {e}")))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "id": team_id,
        "name": req.name,
        "status": "created",
    }))))
}

/// GET /api/teams — 列出 teams. Stage 3 简化: 列出全部.
async fn list_teams_handler(
    State(state): State<Arc<VpsState>>,
) -> Result<Json<Vec<Team>>, (StatusCode, String)> {
    let teams = state
        .team_db
        .list_teams_for_user("anonymous")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("list_teams: {e}")))?;
    Ok(Json(teams))
}

#[derive(Debug, Serialize)]
struct MemberView {
    user_id: String,
    role: String,
    joined_at: String,
}

impl From<TeamMember> for MemberView {
    fn from(m: TeamMember) -> Self {
        Self {
            user_id: m.user_id,
            role: m.role,
            joined_at: m.joined_at,
        }
    }
}

/// GET /api/teams/:id/members — 列出 team 成员.
async fn list_members_handler(
    State(state): State<Arc<VpsState>>,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<MemberView>>, (StatusCode, String)> {
    let members = state
        .team_db
        .list_members(&team_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("list_members: {e}")))?;
    Ok(Json(members.into_iter().map(Into::into).collect()))
}

#[derive(Debug, Deserialize)]
struct AddMemberRequest {
    user_id: String,
    role: String,
}

/// POST /api/teams/:id/members — 添加成员. Stage 3 简化: 不验 user 存在.
async fn add_member_handler(
    State(state): State<Arc<VpsState>>,
    Path(team_id): Path<String>,
    Json(req): Json<AddMemberRequest>,
) -> impl IntoResponse {
    if req.user_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "user_id is required".to_string()));
    }
    state
        .team_db
        .add_member(&team_id, &req.user_id, &req.role)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("add_member: {e}")))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "team_id": team_id,
        "user_id": req.user_id,
        "role": req.role,
        "status": "added",
    }))))
}

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    team_id: String,
    name: String,
    path: String,
    /// Stage 3 简化: owner_id 必填, 不验 user 存在. 客户端从 JWT 提取 (Stage 4 接).
    owner_id: String,
}

/// POST /api/projects — 创建 project.
async fn create_project_handler(
    State(state): State<Arc<VpsState>>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    if req.name.is_empty() || req.path.is_empty() || req.team_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "team_id, name, path are required".to_string(),
        ));
    }
    let project_id = format!("proj_{}", Uuid::new_v4());
    state
        .team_db
        .create_project(&project_id, &req.team_id, &req.name, &req.path, &req.owner_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create_project: {e}")))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "id": project_id,
        "team_id": req.team_id,
        "name": req.name,
        "path": req.path,
        "owner_id": req.owner_id,
        "status": "created",
    }))))
}

/// GET /api/projects — 列出 projects. Stage 3 简化: 列出全部.
async fn list_projects_handler(
    State(state): State<Arc<VpsState>>,
) -> Result<Json<Vec<Project>>, (StatusCode, String)> {
    let projects = state
        .team_db
        .list_projects_for_user("anonymous")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("list_projects: {e}")))?;
    Ok(Json(projects))
}

#[derive(Debug, Deserialize)]
struct AssignProjectRequest {
    user_id: String,
}

/// POST /api/projects/:id/assign — 分配 project 给 user.
async fn assign_project_handler(
    State(state): State<Arc<VpsState>>,
    Path(project_id): Path<String>,
    Json(req): Json<AssignProjectRequest>,
) -> impl IntoResponse {
    if req.user_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "user_id is required".to_string()));
    }
    state
        .team_db
        .assign_project(&project_id, &req.user_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("assign_project: {e}")))?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "project_id": project_id,
        "user_id": req.user_id,
        "status": "assigned",
    }))))
}

// ─── Stage 4: Admin endpoints (7 个, 全部加 admin guard) ────
//
// 设计: 7 个 handler 共享一个 `require_admin_or_err` helper — 失败时
// 直接返回 `(StatusCode, String)`, 成功时把 claims 传给业务逻辑.
//
// 5 个新端点 (按 spec 顺序):
//   1. GET    /api/admin/teams                       — 列出所有 teams
//   2. POST   /api/admin/teams/:id/members           — 添加成员
//   3. DELETE /api/admin/teams/:id/members/:user_id  — 移除成员
//   4. GET    /api/admin/projects                    — 列出所有 team_projects
//   5. DELETE /api/admin/projects/:id               — 软删 (archived=1)
//
// 2 个已有端点 (Stage 1 在 auth.rs, 加 guard):
//   6. POST   /api/admin/users                       — 创建用户
//   7. GET    /api/admin/users                       — 列出用户

/// `require_admin` 错误 → HTTP 响应的统一映射.
fn admin_error_response(e: admin::AdminError) -> (StatusCode, String) {
    match e {
        admin::AdminError::Missing | admin::AdminError::Invalid => {
            (StatusCode::UNAUTHORIZED, "authentication required".to_string())
        }
        admin::AdminError::Forbidden => {
            (StatusCode::FORBIDDEN, "admin role required".to_string())
        }
    }
}

// 1. POST /api/admin/users — admin guard + 调 Stage 1 业务
async fn admin_create_user_handler(
    State(state): State<Arc<VpsState>>,
    headers: HeaderMap,
    Json(req): Json<auth::CreateUserRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Err(e) = admin::require_admin(&headers) {
        return Err(admin_error_response(e));
    }
    auth::create_user_handler(State(state), Json(req)).await
}

// 2. GET /api/admin/users — admin guard
async fn admin_list_users_handler(
    State(state): State<Arc<VpsState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Err(e) = admin::require_admin(&headers) {
        return Err(admin_error_response(e));
    }
    Ok(auth::list_users_handler(State(state)).await)
}

// 3. GET /api/admin/teams — admin guard
async fn admin_list_teams_handler(
    State(state): State<Arc<VpsState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Team>>, (StatusCode, String)> {
    if let Err(e) = admin::require_admin(&headers) {
        return Err(admin_error_response(e));
    }
    let teams = state
        .team_db
        .list_all_teams()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("list_teams: {e}")))?;
    Ok(Json(teams))
}

// 4. POST /api/admin/teams/:id/members — admin guard + add member
async fn admin_add_team_member_handler(
    State(state): State<Arc<VpsState>>,
    Path(team_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<AddMemberRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Err(e) = admin::require_admin(&headers) {
        return Err(admin_error_response(e));
    }
    if req.user_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "user_id is required".to_string()));
    }
    state
        .team_db
        .add_member(&team_id, &req.user_id, &req.role)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("add_member: {e}")))?;
    Ok(Json(serde_json::json!({
        "team_id": team_id,
        "user_id": req.user_id,
        "role": req.role,
        "status": "added",
    })))
}

// 5. DELETE /api/admin/teams/:id/members/:user_id — admin guard
async fn admin_remove_team_member_handler(
    State(state): State<Arc<VpsState>>,
    Path((team_id, user_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Err(e) = admin::require_admin(&headers) {
        return Err(admin_error_response(e));
    }
    remove_team_member(&state, &team_id, &user_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("remove_member: {e}")))?;
    Ok(Json(serde_json::json!({
        "team_id": team_id,
        "user_id": user_id,
        "status": "removed",
    })))
}

/// 直接走 SQL: 复用 TeamDb 内部锁, 避免再加一个公开方法.
fn remove_team_member(
    state: &Arc<VpsState>,
    team_id: &str,
    user_id: &str,
) -> rusqlite::Result<()> {
    let conn = state.team_db.conn.lock().unwrap();
    conn.execute(
        "DELETE FROM team_members WHERE team_id = ?1 AND user_id = ?2",
        rusqlite::params![team_id, user_id],
    )?;
    Ok(())
}

// 6. GET /api/admin/projects — admin guard
async fn admin_list_projects_handler(
    State(state): State<Arc<VpsState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Project>>, (StatusCode, String)> {
    if let Err(e) = admin::require_admin(&headers) {
        return Err(admin_error_response(e));
    }
    let conn = state.team_db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, team_id, name, path, description, owner_id, created_at, archived
             FROM team_projects ORDER BY created_at DESC",
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("prepare: {e}")))?;
    let projects: Vec<Project> = stmt
        .query_map([], |row| {
            let archived_int: i64 = row.get(7)?;
            Ok(Project {
                id: row.get(0)?,
                team_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                description: row.get(4)?,
                owner_id: row.get(5)?,
                created_at: row.get(6)?,
                archived: archived_int != 0,
            })
        })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("query_map: {e}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("collect: {e}")))?;
    Ok(Json(projects))
}

// 7. DELETE /api/admin/projects/:id — admin guard + 软删 (archived=1)
async fn admin_archive_project_handler(
    State(state): State<Arc<VpsState>>,
    Path(project_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Err(e) = admin::require_admin(&headers) {
        return Err(admin_error_response(e));
    }
    let conn = state.team_db.conn.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let n = conn
        .execute(
            "UPDATE team_projects SET archived = 1, archived_at = ?2 WHERE id = ?1",
            rusqlite::params![project_id, now],
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("archive_project: {e}")))?;
    if n == 0 {
        return Err((StatusCode::NOT_FOUND, format!("project {project_id} not found")));
    }
    Ok(Json(serde_json::json!({
        "project_id": project_id,
        "status": "archived",
    })))
}

// ─── WebSocket (Stage 2: 完整事件分发) ─────────────────────────

/// `GET /api/ws` — WS 升级入口.
///
/// Stage 2: 接受 upgrade, 启动 `handle_socket` 完整事件分发 (Auth / Register / Heartbeat).
/// 真正的 `?token=...` 校验见 `02-vps-server.md` §11.3 (Stage 3 接).
async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<VpsState>>,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Stage 2 事件分发主循环 (device-side 视角).
///
/// ## 协议顺序
/// 1. 客户端发 `WsFrame::Auth { device_token, machine_id }`
///    - 成功 → 回 `AuthOk { session_token, server_time, server_version, heartbeat_interval_ms }`
///    - 失败 → 回 `AuthError { code, message }` + 关闭
/// 2. 客户端发 `WsFrame::Register { device_id, name, host_type, host_id, tags, ... }`
///    - 成功 → 回 `RegisterOk { node_id }`
///    - 未认证 → 回 `RegisterError { code: "auth_required", ... }`
/// 3. 客户端持续发 `WsFrame::Heartbeat { ts }` (建议 30s 一次, 与 `heartbeat_interval_ms` 对齐)
///    - VPS 回 `HeartbeatAck { ts }`
///    - VPS 后台 monitor: 90s 没收到 Heartbeat → 主动关闭 + unregister
///
/// ## 实现细节
/// - 写循环: 独立 task, 监听 `rx` (`tx` 在读循环持有), 收到 frame → 发到 socket.
///   当读循环 drop 它的 `tx` clone 时, `rx.recv()` 返回 `None`, 写循环退出.
/// - 读循环: 当前 task, `tokio::select!` on `shutdown_rx` (watch) + `ws_rx.next()`.
/// - 关闭路径: monitor 发 shutdown 信号 → 读循环退出 → drop tx → 写循环退出 → unregister.
/// - 心跳 monitor: 30s tick, 仅对已认证 conn 生效.
async fn handle_socket(socket: WebSocket, state: Arc<VpsState>) {
    let hub = state.ws_hub.clone();

    // 1. 注册 conn, 拿到 conn_id + 写循环用的 tx.
    //    写循环在另一个 task 持有 rx, 读循环持有 tx clone, 写响应帧.
    let (tx, rx) = mpsc::unbounded_channel::<Message>();
    let conn_id = hub
        .register(ConnectionType::Device, "pending".to_string(), tx.clone())
        .await;
    tracing::info!(connection_id = %conn_id, "ws socket accepted");

    // 2. 拆 socket 为 (写半, 读半) — 写半给写循环, 读半在当前 task.
    let (ws_tx, ws_rx) = socket.split();

    // 3. shutdown 通道: 用 watch 而非 mpsc, 因为 receiver 可以 clone 多份.
    //    Stage 2 只需要 1 份 receiver (在读循环), 但 watch 留扩展余地.
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    // 4. 写循环 task — 只看 `rx`, 没人 send 就退出.
    let write_conn_id = conn_id.clone();
    let write_task = tokio::spawn(async move {
        let mut ws_tx = ws_tx;
        let mut rx = rx;
        while let Some(msg) = rx.recv().await {
            if let Err(e) = ws_tx.send(msg).await {
                tracing::debug!(connection_id = %write_conn_id, error = %e, "write loop: send failed");
                break;
            }
        }
        // 尽力 flush Close
        let _ = ws_tx.send(Message::Close(None)).await;
        tracing::debug!(connection_id = %write_conn_id, "write loop exited");
    });

    // 5. Heartbeat monitor task — 30s tick, 已认证 + 90s 无心跳 → 踢.
    let monitor_hub = hub.clone();
    let monitor_conn_id = conn_id.clone();
    let monitor_shutdown_tx = shutdown_tx.clone();
    let monitor_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            // 只对已认证的 conn 监控 (auth 前的 conn 还在握手, 不踢)
            if !monitor_hub.is_authenticated(&monitor_conn_id).await {
                continue;
            }
            let Some(last) = monitor_hub.last_heartbeat_at(&monitor_conn_id).await else {
                continue;
            };
            let elapsed = chrono::Utc::now() - last;
            if elapsed > chrono::Duration::seconds(90) {
                tracing::warn!(
                    connection_id = %monitor_conn_id,
                    elapsed_secs = elapsed.num_seconds(),
                    "heartbeat timeout, kicking connection"
                );
                let _ = monitor_shutdown_tx.send(true);
                break;
            }
        }
    });

    // 6. 读循环 (当前 task): 解析 + 派发 frame.
    let mut ws_rx = ws_rx;
    loop {
        tokio::select! {
            biased;
            // watch 接收端: `changed()` 在每次值变化时 resolve 一次 (初始值之后).
            res = shutdown_rx.changed() => {
                if res.is_err() {
                    // sender dropped, 不太可能但要兜底
                    tracing::debug!(connection_id = %conn_id, "shutdown sender dropped");
                } else {
                    tracing::info!(connection_id = %conn_id, "read loop: shutdown signal received");
                }
                break;
            }
            maybe_msg = ws_rx.next() => {
                let Some(msg_result) = maybe_msg else {
                    // 客户端断开
                    tracing::info!(connection_id = %conn_id, "ws socket closed by client");
                    break;
                };
                let msg = match msg_result {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(connection_id = %conn_id, error = %e, "ws recv error");
                        break;
                    }
                };
                match msg {
                    Message::Text(text) => {
                        handle_text_frame(&hub, &conn_id, text.as_str(), &tx).await;
                    }
                    Message::Close(_) => {
                        tracing::info!(connection_id = %conn_id, "client sent Close");
                        break;
                    }
                    Message::Ping(p) => {
                        // echo Pong (axum 默认会回, 这里冗余一下保险)
                        let _ = tx.send(Message::Pong(p));
                    }
                    Message::Binary(_) | Message::Pong(_) => {
                        // 忽略 (Stage 2 协议只用 Text 帧)
                    }
                }
            }
        }
    }

    // 7. 清理: 关 monitor + 写循环, unregister.
    //    drop tx 让写循环自然退出 (rx.recv() 返回 None).
    drop(tx);
    drop(shutdown_tx);
    monitor_task.abort();
    let _ = write_task.await;
    hub.unregister(&conn_id).await;
    tracing::info!(connection_id = %conn_id, "ws socket fully closed");
}

/// 派发一条文本帧: 解析为 `WsFrame`, 调对应 Hub 方法, 把响应写回 `tx`.
///
/// 容错: 解析失败 → 静默忽略 (记 warn), 不发响应 (协议层不期望 error 帧有 echo).
async fn handle_text_frame(hub: &Arc<WsHub>, conn_id: &str, text: &str, tx: &mpsc::UnboundedSender<Message>) {
    let frame: WsFrame = match serde_json::from_str(text) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(connection_id = %conn_id, error = %e, raw = %text, "invalid WsFrame JSON, ignoring");
            return;
        }
    };
    let frame_type = frame.type_name();
    tracing::debug!(connection_id = %conn_id, frame_type = %frame_type, "ws frame received");

    let response: Option<WsFrame> = match frame {
        WsFrame::Auth { .. } => Some(handle_auth_frame(hub, conn_id, &frame).await),
        WsFrame::Register { .. } => Some(hub.register_device(conn_id, &frame).await),
        WsFrame::Heartbeat { .. } => Some(hub.handle_heartbeat(conn_id, &frame).await),
        WsFrame::Prompt { .. } => handle_prompt_frame(hub, conn_id, &frame, tx).await,
        other => {
            // Stage 2 暂不处理: AuthOk/AuthError/RegisterOk/RegisterError (VPS→Device)
            // / Event/EventDone/EventError/HeartbeatAck. 收到就忽略.
            tracing::debug!(
                connection_id = %conn_id,
                frame_type = %other.type_name(),
                "ignoring frame in stage 2 (handler not implemented yet)"
            );
            None
        }
    };

    if let Some(resp) = response {
        if let Some(msg) = WsHub::encode_frame(&resp) {
            if let Err(e) = tx.send(msg) {
                tracing::debug!(connection_id = %conn_id, error = %e, "failed to enqueue response (write loop gone)");
            }
        }
    }
}

/// 调 `hub.authenticate` 并 flatten `Result<WsFrame, WsFrame>` 为单一 `WsFrame`.
async fn handle_auth_frame(hub: &Arc<WsHub>, conn_id: &str, frame: &WsFrame) -> WsFrame {
    match hub.authenticate(conn_id, frame).await {
        Ok(auth_ok) => auth_ok,
        Err(auth_err) => auth_err,
    }
}

// ─── Stage 4: Prompt 帧路由 + Team RBAC ───────────────────
//
// 处理来自 App 的 `WsFrame::Prompt`:
// 1. 从 conn 拿 user_id (App connection 走 `WsHub::user_id_for`).
// 2. 拿 `target_project_id` (Stage 4 新增字段, 见 `messages.rs`).
// 3. 调 `check_rbac` 验证权限.
// 4. 不通过 → 推 `EventError { code: "forbidden" }` 给 client (经 `tx`).
// 5. 通过 → 当前 Stage 4 是 stub, Stage 5 才会真转发到 target_node 的 Daemon.
//
// ## Stage 4 简化
// - App JWT 验证未接 (Stage 5), 所以 `hub.user_id_for` 现在对所有 conn 返 None
//   (因为目前只支持 Device connection). 收到 Prompt 帧时, 如果 user_id 是 None,
//   视为未授权, 推 `EventError { code: "unauthorized" }`.
// - 不接限流 (governor) / Outbox / Cancel 跟踪 — Stage 5+.
// - 不接 `event` / `event_done` / `event_error` 反向路由 — Stage 5+.

async fn handle_prompt_frame(
    hub: &Arc<WsHub>,
    conn_id: &str,
    frame: &WsFrame,
    tx: &mpsc::UnboundedSender<Message>,
) -> Option<WsFrame> {
    // 1. 解构 Prompt 帧
    let (request_id, target_project_id, target_node_id) = match frame {
        WsFrame::Prompt {
            request_id,
            target_project_id,
            target_node_id,
            ..
        } => (
            request_id.clone(),
            target_project_id.clone(),
            target_node_id.clone(),
        ),
        _ => return None, // 协议错误, 不会触发 (caller 已 match)
    };

    // 2. 取 user_id (App connection 才有, Device 返 None)
    let from_user_id = match hub.user_id_for(conn_id).await {
        Some(uid) => uid,
        None => {
            tracing::warn!(
                connection_id = %conn_id,
                request_id = %request_id,
                "Prompt frame from non-app connection (or app not wired in stage 4)"
            );
            // 推 unauthorized 错误给 caller
            let err = WsFrame::EventError {
                request_id,
                code: "unauthorized".into(),
                message: "prompt requires authenticated app connection".into(),
            };
            // 直接走 tx.send 不走 response Option, 因为 err 是给 client 的 event_error
            // (按 WsFrame 协议: event_error 是 VPS→App 方向, 但 Stage 4 在同 conn 回环, OK)
            if let Some(msg) = WsHub::encode_frame(&err) {
                let _ = tx.send(msg);
            }
            return None;
        }
    };

    // 3. RBAC 检查
    match check_rbac(&hub.team_db, &from_user_id, &target_project_id).await {
        Ok(true) => {
            // 4. 通过: 当前 Stage 4 不真转发 (Stage 5 才会 forward 到 target_node 的 Daemon).
            //    这里只记 trace, 留 hook 给 Stage 5.
            tracing::info!(
                connection_id = %conn_id,
                user_id = %from_user_id,
                request_id = %request_id,
                target_project_id = %target_project_id,
                target_node_id = %target_node_id,
                "RBAC passed for prompt frame (Stage 4: forwarding stubbed, see 02-vps-server.md §9)"
            );
            None
        }
        Ok(false) => {
            tracing::warn!(
                connection_id = %conn_id,
                user_id = %from_user_id,
                request_id = %request_id,
                target_project_id = %target_project_id,
                "RBAC denied for prompt frame"
            );
            let err = WsFrame::EventError {
                request_id,
                code: "forbidden".into(),
                message: format!(
                    "user {from_user_id} has no access to project {target_project_id}"
                ),
            };
            if let Some(msg) = WsHub::encode_frame(&err) {
                let _ = tx.send(msg);
            }
            None
        }
        Err(e) => {
            tracing::error!(
                connection_id = %conn_id,
                error = %e,
                "RBAC check failed (db error)"
            );
            let err = WsFrame::EventError {
                request_id,
                code: "internal".into(),
                message: format!("rbac check error: {e}"),
            };
            if let Some(msg) = WsHub::encode_frame(&err) {
                let _ = tx.send(msg);
            }
            None
        }
    }
}

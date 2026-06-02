//! WebSocket Hub — Stage 1 骨架 + Stage 2 auth/heartbeat 状态.
//!
//! 设计目标:
//! 1. 维护活跃连接的注册表: `internal_id → Connection` + 双索引 (`by_device`, `by_user`).
//! 2. 提供 `register` / `unregister` / `push_to_user` / `push_to_device` / `stats` API.
//! 3. **Stage 2 新增**: auth 状态 (authed_machine) + 心跳跟踪 (last_heartbeat) +
//!    节点注册 (node_id, device_meta) + `authenticate` / `handle_heartbeat` /
//!    `register_device` 三个派发方法.
//! 4. **不**实现 outbox / 完整 pending_command 跟踪 / rate-limit — 那些是 Stage 3+ 的事.
//!
//! 并发模型:
//! - `Arc<RwLock<HashMap<...>>>` 而非 `DashMap`: workspace 没有 dashmap 依赖, 而当前
//!   读多写少, RwLock 足够. Stage 3+ 评估是否升级到 `DashMap` 应对 100+ 并发连接.
//! - `tx: UnboundedSender<Message>`: Hub 把消息扔进 channel, 真正的写循环在
//!   `handle_socket` 里 select 这个 channel. 失败时 `unregister` 兜底.

use axum::extract::ws::Message;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use super::auth_ws;
use super::messages::WsFrame;
use super::team_db::TeamDb;

/// 连接到 VPS 的客户端类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionType {
    /// 来自 dev 机的 daemon
    Device,
    /// 来自 user 的 app (Web / Mobile)
    App,
}

/// 单个 WebSocket 连接的元信息.
#[derive(Debug, Clone)]
pub struct Connection {
    /// 内部唯一 id (VPS 内部生成, 与客户端无关).
    pub id: String,
    pub connection_type: ConnectionType,
    /// Device: 注册时是 "pending", auth 成功后被 `authed_machine` 覆盖.
    /// App: user_id.
    pub principal_id: String,
    /// 把消息发到这个 channel → 由调用方绑定的写循环 select.
    pub tx: mpsc::UnboundedSender<Message>,
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

/// Stage 2 设备元信息 (Register 帧上报, Hub 暂存).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceMeta {
    pub device_id: String,
    pub name: String,
    pub host_type: String,
    pub host_id: String,
    pub tags: Vec<String>,
    pub capabilities: Vec<String>,
    pub daemon_version: String,
    pub os: String,
    pub cpu_cores: u32,
    pub memory_mb: u32,
}

/// WebSocket Hub — 维护所有活跃连接 + 消息路由.
#[derive(Clone)]
pub struct WsHub {
    /// 内部 id → Connection
    connections: Arc<RwLock<HashMap<String, Arc<Connection>>>>,
    /// 索引: device → 内部 id 列表 (一对多, dev 机可能多 connection)
    by_device: Arc<RwLock<HashMap<String, Vec<String>>>>,
    /// 索引: app user → 内部 id 列表 (一对多, user 可多端登录)
    by_user: Arc<RwLock<HashMap<String, Vec<String>>>>,
    // ─── Stage 2 新增: auth + heartbeat + 节点注册状态 ───
    /// Auth 成功后绑定: `conn_id` → 客户端上报的 `machine_id`.
    /// 存在即"已认证", 用于 `Register` 帧的鉴权门.
    authed_machine: Arc<RwLock<HashMap<String, String>>>,
    /// Register 成功后分配: `conn_id` → 节点 id (`node_xxx`).
    node_id: Arc<RwLock<HashMap<String, String>>>,
    /// Register 帧暂存: `conn_id` → `DeviceMeta`.
    device_meta: Arc<RwLock<HashMap<String, DeviceMeta>>>,
    /// 最近一次心跳: `conn_id` → `DateTime<Utc>`.
    /// Auth 成功时初始化为 `now()`, 之后每次 `Heartbeat` 帧更新.
    last_heartbeat: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    // ─── Stage 3 新增: TeamDb 引用 (auth lookup) ───
    /// Stage 3 持久化层 — 持有 `Arc<TeamDb>`, `authenticate` 用它做
    /// device_token 查表 (替换 Stage 2 静态白名单).
    pub team_db: Arc<TeamDb>,
}

impl std::fmt::Debug for WsHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsHub")
            .field("team_db", &"<Arc<TeamDb>>")
            .finish()
    }
}

impl WsHub {
    /// 构造 WsHub, 注入 TeamDb 引用.
    ///
    /// **Stage 3**: 必须传入 `team_db` (供 `authenticate` 做 device_token 查表).
    pub fn new(team_db: Arc<TeamDb>) -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            by_device: Arc::new(RwLock::new(HashMap::new())),
            by_user: Arc::new(RwLock::new(HashMap::new())),
            authed_machine: Arc::new(RwLock::new(HashMap::new())),
            node_id: Arc::new(RwLock::new(HashMap::new())),
            device_meta: Arc::new(RwLock::new(HashMap::new())),
            last_heartbeat: Arc::new(RwLock::new(HashMap::new())),
            team_db,
        }
    }

    /// 注册新连接. 返回 `connection_id`.
    ///
    /// `principal_id`:
    /// - Device → machine_id (设备唯一标识, 由设备注册时上报)
    /// - App → user_id (用户唯一标识, 由 JWT 解析得到)
    pub async fn register(
        &self,
        connection_type: ConnectionType,
        principal_id: String,
        tx: mpsc::UnboundedSender<Message>,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let conn = Arc::new(Connection {
            id: id.clone(),
            connection_type,
            principal_id: principal_id.clone(),
            tx,
            connected_at: chrono::Utc::now(),
        });

        // 1. 主表
        self.connections
            .write()
            .await
            .insert(id.clone(), conn.clone());

        // 2. 索引
        let index_lock = match connection_type {
            ConnectionType::Device => &self.by_device,
            ConnectionType::App => &self.by_user,
        };
        index_lock
            .write()
            .await
            .entry(principal_id)
            .or_default()
            .push(id.clone());

        tracing::info!(
            connection_id = %id,
            ?connection_type,
            "ws connection registered"
        );
        id
    }

    /// 注销连接. 不存在时静默忽略.
    ///
    /// Stage 2 扩展: 同步清理 `authed_machine` / `node_id` / `device_meta` /
    /// `last_heartbeat` 四个 per-conn map, 避免泄漏.
    pub async fn unregister(&self, connection_id: &str) {
        let conn = self.connections.write().await.remove(connection_id);
        let Some(conn) = conn else { return };

        let index_lock = match conn.connection_type {
            ConnectionType::Device => &self.by_device,
            ConnectionType::App => &self.by_user,
        };
        let mut idx = index_lock.write().await;
        if let Some(ids) = idx.get_mut(&conn.principal_id) {
            ids.retain(|x| x != connection_id);
            if ids.is_empty() {
                idx.remove(&conn.principal_id);
            }
        }
        drop(idx);

        // Stage 2: 清理 per-conn state. 单 conn_id, 无竞态.
        self.authed_machine.write().await.remove(connection_id);
        self.node_id.write().await.remove(connection_id);
        self.device_meta.write().await.remove(connection_id);
        self.last_heartbeat.write().await.remove(connection_id);

        tracing::info!(
            connection_id = %connection_id,
            ?conn.connection_type,
            principal_id = %conn.principal_id,
            "ws connection unregistered"
        );
    }

    /// 路由消息: 给指定 user 的所有 App 推.
    /// 返回成功投递的 connection 数.
    pub async fn push_to_user(&self, user_id: &str, msg: Message) -> usize {
        let ids: Vec<String> = self
            .by_user
            .read()
            .await
            .get(user_id)
            .cloned()
            .unwrap_or_default();

        if ids.is_empty() {
            return 0;
        }

        self.fanout(&ids, msg).await
    }

    /// 路由消息: 给指定 device (machine_id) 推.
    /// 返回成功投递的 connection 数.
    pub async fn push_to_device(&self, machine_id: &str, msg: Message) -> usize {
        let ids: Vec<String> = self
            .by_device
            .read()
            .await
            .get(machine_id)
            .cloned()
            .unwrap_or_default();

        if ids.is_empty() {
            return 0;
        }

        self.fanout(&ids, msg).await
    }

    /// 给定 connection 列表, 同步尝试 send, 统计成功数.
    /// send 失败意味着对端已断 (channel 关闭), 但本函数不主动 unregister —
    /// 写循环负责 detect 关闭并 unregister.
    async fn fanout(&self, ids: &[String], msg: Message) -> usize {
        let conns = self.connections.read().await;
        let mut count = 0usize;
        for id in ids {
            if let Some(conn) = conns.get(id) {
                if conn.tx.send(msg.clone()).is_ok() {
                    count += 1;
                }
            }
        }
        count
    }

    /// 当前连接数 (调试用).
    pub async fn stats(&self) -> HubStats {
        let conns = self.connections.read().await;
        let mut total = 0usize;
        let mut devices = 0usize;
        let mut apps = 0usize;
        for c in conns.values() {
            total += 1;
            match c.connection_type {
                ConnectionType::Device => devices += 1,
                ConnectionType::App => apps += 1,
            }
        }
        HubStats {
            total,
            devices,
            apps,
        }
    }

    // ─── Stage 2 新增: auth / heartbeat / 节点注册 API ───

    /// 鉴权握手: 校验 `WsFrame::Auth`, 成功则标记 `authed_machine` + 初始化 `last_heartbeat`,
    /// 返回 `AuthOk`; 失败返回 `AuthError`.
    ///
    /// **返回类型** `Result<WsFrame, WsFrame>`:
    /// - `Ok(WsFrame::AuthOk { ... })` — 鉴权成功, 携带 `session_token` (UUID 前缀 `st_`).
    /// - `Err(WsFrame::AuthError { code, message })` — token 无效 / 过期 / 内部错误.
    ///
    /// **Stage 2 行为**:
    /// - token 走 `auth_ws::validate_device_token` 静态白名单.
    /// - `session_token` 是 VPS 颁发, Stage 3 用于 `register` 帧的二次校验.
    /// - `heartbeat_interval_ms = 30000` (固定), 与 `_shared-contract.md` §3.3 一致.
    ///
    /// **Stage 3 TODO**: `session_token` 写 DB `sessions` 表, `Register` 帧必须带
    /// session_token 二次校验.
    pub async fn authenticate(
        &self,
        connection_id: &str,
        frame: &WsFrame,
    ) -> Result<WsFrame, WsFrame> {
        let (device_token, machine_id) = match frame {
            WsFrame::Auth { device_token, machine_id } => {
                (device_token.clone(), machine_id.clone())
            }
            // 协议错误: 调用方传错了 frame 类型
            _ => {
                return Err(WsFrame::AuthError {
                    code: "protocol_error".into(),
                    message: "expected Auth frame".into(),
                });
            }
        };

        match auth_ws::validate_device_token(&self.team_db, &device_token) {
            Ok(_info) => {
                // 标记已认证, 绑定 machine_id, 初始化心跳时间.
                let now = Utc::now();
                self.authed_machine
                    .write()
                    .await
                    .insert(connection_id.to_string(), machine_id.clone());
                self.last_heartbeat
                    .write()
                    .await
                    .insert(connection_id.to_string(), now);

                tracing::info!(
                    connection_id = %connection_id,
                    machine_id = %machine_id,
                    "ws device authenticated"
                );

                Ok(WsFrame::AuthOk {
                    session_token: format!("st_{}", Uuid::new_v4()),
                    server_time: now.to_rfc3339(),
                    server_version: "0.3.0-stage2".into(),
                    heartbeat_interval_ms: 30000,
                })
            }
            Err(e) => {
                let code = auth_ws::auth_error_code(&e).to_string();
                tracing::warn!(
                    connection_id = %connection_id,
                    machine_id = %machine_id,
                    code = %code,
                    "ws device auth failed"
                );
                Err(WsFrame::AuthError {
                    code,
                    message: e.to_string(),
                })
            }
        }
    }

    /// 处理 `WsFrame::Heartbeat`: 更新 `last_heartbeat` + 返回 `HeartbeatAck`.
    ///
    /// 不要求已认证 (允许 anon 心跳, 监控用). 实际生产应该鉴权, Stage 3 接.
    ///
    /// 协议错误 (传错 frame) → 返回 `HeartbeatAck { ts: 0 }` (不报错, 容错).
    pub async fn handle_heartbeat(
        &self,
        connection_id: &str,
        frame: &WsFrame,
    ) -> WsFrame {
        let ts = match frame {
            WsFrame::Heartbeat { ts } => *ts,
            _ => 0,
        };
        self.last_heartbeat
            .write()
            .await
            .insert(connection_id.to_string(), Utc::now());
        WsFrame::HeartbeatAck { ts }
    }

    /// 处理 `WsFrame::Register`: 分配 `node_id` (`node_xxx`) + 暂存 `DeviceMeta`,
    /// 返回 `RegisterOk`. **要求连接已认证**, 否则返回 `RegisterError`.
    ///
    /// **Stage 2 行为**:
    /// - `node_id` 是 VPS 颁发, Stage 3 写入 `devices.node_id` 字段.
    /// - 多次 register 同 conn: 覆盖 (Stage 3 应拒绝).
    pub async fn register_device(
        &self,
        connection_id: &str,
        frame: &WsFrame,
    ) -> WsFrame {
        // 1. 鉴权门: 未认证拒绝
        if !self.is_authenticated(connection_id).await {
            return WsFrame::RegisterError {
                code: "auth_required".into(),
                message: "must authenticate before register".into(),
            };
        }

        // 2. 解构 Register 帧
        let meta = match frame {
            WsFrame::Register {
                device_id,
                name,
                host_type,
                host_id,
                tags,
                capabilities,
                daemon_version,
                os,
                cpu_cores,
                memory_mb,
            } => DeviceMeta {
                device_id: device_id.clone(),
                name: name.clone(),
                host_type: host_type.clone(),
                host_id: host_id.clone(),
                tags: tags.clone(),
                capabilities: capabilities.clone(),
                daemon_version: daemon_version.clone(),
                os: os.clone(),
                cpu_cores: *cpu_cores,
                memory_mb: *memory_mb,
            },
            _ => {
                return WsFrame::RegisterError {
                    code: "protocol_error".into(),
                    message: "expected Register frame".into(),
                };
            }
        };

        // 3. 分配 node_id + 暂存 meta
        let node_id = format!("node_{}", Uuid::new_v4());
        self.node_id
            .write()
            .await
            .insert(connection_id.to_string(), node_id.clone());
        self.device_meta
            .write()
            .await
            .insert(connection_id.to_string(), meta.clone());

        tracing::info!(
            connection_id = %connection_id,
            node_id = %node_id,
            device_id = %meta.device_id,
            name = %meta.name,
            "ws device registered"
        );

        WsFrame::RegisterOk { node_id }
    }

    /// 该 conn_id 是否已通过 auth.
    pub async fn is_authenticated(&self, connection_id: &str) -> bool {
        self.authed_machine.read().await.contains_key(connection_id)
    }

    /// 该 conn_id 已认证的 machine_id (`Auth` 帧上报).
    pub async fn authed_machine_id(&self, connection_id: &str) -> Option<String> {
        self.authed_machine.read().await.get(connection_id).cloned()
    }

    /// 该 conn_id 分配的 node_id (`Register` 后).
    pub async fn node_id_for(&self, connection_id: &str) -> Option<String> {
        self.node_id.read().await.get(connection_id).cloned()
    }

    /// 该 conn_id 的 `DeviceMeta` (`Register` 后).
    pub async fn device_meta_for(&self, connection_id: &str) -> Option<DeviceMeta> {
        self.device_meta.read().await.get(connection_id).cloned()
    }

    /// 最近一次心跳时间 (`Auth` 成功后初始化, 每次 `Heartbeat` 帧更新).
    /// 未认证或无心跳记录 → `None`.
    pub async fn last_heartbeat_at(&self, connection_id: &str) -> Option<DateTime<Utc>> {
        self.last_heartbeat.read().await.get(connection_id).copied()
    }

    /// 把 `WsFrame` 序列化为 WS `Message::Text` (Hub 内部辅助, 给 `handle_socket` 用).
    pub fn encode_frame(frame: &WsFrame) -> Option<Message> {
        match serde_json::to_string(frame) {
            Ok(s) => Some(Message::Text(s.into())),
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize WsFrame");
                None
            }
        }
    }

    /// 测试用: 取单个 Connection 的 clone.
    #[cfg(test)]
    pub async fn get_connection(&self, id: &str) -> Option<Arc<Connection>> {
        self.connections.read().await.get(id).cloned()
    }
}

/// Hub 快照统计.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubStats {
    pub total: usize,
    pub devices: usize,
    pub apps: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::team_db::TeamDb;
    use axum::extract::ws::Message;
    use rusqlite::Connection;
    use std::time::Duration;
    use tokio::time::timeout;

    /// 构造测试用 WsHub (in-memory TeamDb, 不写文件).
    fn test_hub() -> WsHub {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'user',
                created_at TEXT NOT NULL,
                disabled INTEGER NOT NULL DEFAULT 0
            )",
        )
        .expect("create users");
        let team_db = Arc::new(TeamDb::from_connection(conn).expect("from_connection"));
        WsHub::new(team_db)
    }

    /// 预注册一个 device token (Stage 3 替换 Stage 2 静态白名单).
    fn seed_token(hub: &WsHub, token: &str, machine_id: &str) {
        hub.team_db
            .register_device(token, machine_id, "test-pc")
            .expect("seed token");
    }

    /// 测试 1: register 1 个 device, stats().devices == 1, unregister 后 == 0.
    #[tokio::test]
    async fn test_register_and_unregister() {
        let hub = test_hub();
        let (tx, _rx) = mpsc::unbounded_channel::<Message>();

        let id = hub
            .register(ConnectionType::Device, "machine-A".into(), tx)
            .await;

        let s = hub.stats().await;
        assert_eq!(s.total, 1, "expected 1 total connection");
        assert_eq!(s.devices, 1, "expected 1 device connection");
        assert_eq!(s.apps, 0, "expected 0 app connections");

        hub.unregister(&id).await;

        let s = hub.stats().await;
        assert_eq!(s.total, 0, "expected 0 total connections after unregister");
        assert_eq!(s.devices, 0, "expected 0 device connections after unregister");
    }

    /// 测试 2: register 2 个 app (同 user), push 1 条消息, 2 个 connection 都收到.
    #[tokio::test]
    async fn test_push_to_user_routes_to_all_user_connections() {
        let hub = test_hub();

        let (tx1, mut rx1) = mpsc::unbounded_channel::<Message>();
        let (tx2, mut rx2) = mpsc::unbounded_channel::<Message>();
        let (tx_other, mut rx_other) = mpsc::unbounded_channel::<Message>();

        // 同 user 两条
        let _id1 = hub.register(ConnectionType::App, "user_alice".into(), tx1).await;
        let _id2 = hub.register(ConnectionType::App, "user_alice".into(), tx2).await;
        // 另一个 user, 不应收
        let _id3 = hub
            .register(ConnectionType::App, "user_bob".into(), tx_other)
            .await;

        let pushed = hub
            .push_to_user("user_alice", Message::Text("hello-alice".into()))
            .await;
        assert_eq!(pushed, 2, "expected 2 recipients (user_alice's 2 connections)");

        let m1 = timeout(Duration::from_millis(200), rx1.recv())
            .await
            .expect("rx1 timeout")
            .expect("rx1 closed");
        let m2 = timeout(Duration::from_millis(200), rx2.recv())
            .await
            .expect("rx2 timeout")
            .expect("rx2 closed");
        if let Message::Text(t) = m1 {
            assert_eq!(t.as_str(), "hello-alice");
        } else {
            panic!("rx1 expected Message::Text, got: {:?}", m1);
        }
        if let Message::Text(t) = m2 {
            assert_eq!(t.as_str(), "hello-alice");
        } else {
            panic!("rx2 expected Message::Text, got: {:?}", m2);
        }

        // rx_other 不应收
        let r = timeout(Duration::from_millis(100), rx_other.recv()).await;
        assert!(r.is_err(), "user_bob's connection should not receive");
    }

    /// 测试 3: register 2 个 device (不同 machine_id), push 给其中 1 个, 只 1 个收到.
    #[tokio::test]
    async fn test_push_to_device_routes_correctly() {
        let hub = test_hub();

        let (tx1, mut rx1) = mpsc::unbounded_channel::<Message>();
        let (tx2, mut rx2) = mpsc::unbounded_channel::<Message>();

        let _id1 = hub
            .register(ConnectionType::Device, "machine-A".into(), tx1)
            .await;
        let _id2 = hub
            .register(ConnectionType::Device, "machine-B".into(), tx2)
            .await;

        let pushed = hub
            .push_to_device("machine-A", Message::Text("to-A".into()))
            .await;
        assert_eq!(pushed, 1, "expected 1 recipient (machine-A only)");

        let m1 = timeout(Duration::from_millis(200), rx1.recv())
            .await
            .expect("rx1 timeout")
            .expect("rx1 closed");
        if let Message::Text(t) = m1 {
            assert_eq!(t.as_str(), "to-A");
        } else {
            panic!("rx1 expected Message::Text, got: {:?}", m1);
        }

        // rx2 不应收
        let r = timeout(Duration::from_millis(100), rx2.recv()).await;
        assert!(r.is_err(), "machine-B's connection should not receive");
    }

    /// 测试 4 (额外): unregister 后 push 不应到达.
    ///
    /// 注意: unregister 也会 drop `tx` (因为 `Arc<Connection>` 引用计数归零), 这会关闭 channel.
    /// 所以 `rx.recv()` 可能是 `Err(Elapsed)` (timeout) 或 `Ok(None)` (closed) — 两种都表示
    /// "没收到 ghost 消息".
    #[tokio::test]
    async fn test_push_to_user_after_unregister_returns_zero() {
        let hub = test_hub();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let id = hub
            .register(ConnectionType::App, "user_alice".into(), tx)
            .await;
        hub.unregister(&id).await;

        let pushed = hub
            .push_to_user("user_alice", Message::Text("ghost".into()))
            .await;
        assert_eq!(pushed, 0, "no recipients after unregister");

        let r = timeout(Duration::from_millis(50), rx.recv()).await;
        match r {
            // timeout: 没收到, 符合预期.
            Err(_elapsed) => {}
            // channel closed (unregister drop 了 tx): 也没收到 ghost, 符合预期.
            Ok(None) => {}
            // 收到 Text 才是失败.
            Ok(Some(Message::Text(t))) => {
                panic!("rx unexpectedly received Text: {}", t.as_str())
            }
            Ok(Some(other)) => panic!("rx unexpectedly received: {:?}", other),
        }
    }

    // ──────── Stage 2 测试: auth + heartbeat + register ────────

    /// 测试 5 (Stage 2): 合法 token → 返回 `AuthOk` 帧, 字段正确.
    #[tokio::test]
    async fn test_authenticate_valid_token_emits_auth_ok() {
        let hub = test_hub();
        // 预注册一个 token (Stage 3 替换 Stage 2 静态白名单)
        seed_token(&hub, "test_token_dt_xxx", "machine_1");
        let (tx, _rx) = mpsc::unbounded_channel::<Message>();
        let conn_id = hub
            .register(ConnectionType::Device, "pending".into(), tx)
            .await;

        // 认证前: 未认证, 无心跳
        assert!(!hub.is_authenticated(&conn_id).await);
        assert!(hub.last_heartbeat_at(&conn_id).await.is_none());

        let frame = WsFrame::Auth {
            device_token: "test_token_dt_xxx".into(),
            machine_id: "machine_1".into(),
        };
        let result = hub.authenticate(&conn_id, &frame).await;

        match result {
            Ok(WsFrame::AuthOk {
                session_token,
                server_time,
                server_version,
                heartbeat_interval_ms,
            }) => {
                assert!(
                    session_token.starts_with("st_"),
                    "session_token should start with 'st_', got: {session_token}"
                );
                assert!(
                    !server_time.is_empty(),
                    "server_time should be non-empty"
                );
                assert!(
                    !server_version.is_empty(),
                    "server_version should be non-empty"
                );
                assert_eq!(
                    heartbeat_interval_ms, 30000,
                    "heartbeat_interval_ms should be 30000"
                );
            }
            Ok(other) => panic!("expected AuthOk, got: {:?}", other),
            Err(e) => panic!("expected Ok(AuthOk), got Err: {:?}", e),
        }

        // 认证后: is_authenticated=true, machine_id 绑定, last_heartbeat 初始化
        assert!(hub.is_authenticated(&conn_id).await);
        assert_eq!(
            hub.authed_machine_id(&conn_id).await,
            Some("machine_1".to_string())
        );
        assert!(hub.last_heartbeat_at(&conn_id).await.is_some());
    }

    /// 测试 6 (Stage 2): 非法 token → 返回 `AuthError { code: "invalid_token", ... }`.
    #[tokio::test]
    async fn test_authenticate_invalid_token_emits_auth_error() {
        let hub = test_hub();
        // 不预注册 — 任何 token 都该失败
        let (tx, _rx) = mpsc::unbounded_channel::<Message>();
        let conn_id = hub
            .register(ConnectionType::Device, "pending".into(), tx)
            .await;

        let frame = WsFrame::Auth {
            device_token: "wrong_token".into(),
            machine_id: "machine_1".into(),
        };
        let result = hub.authenticate(&conn_id, &frame).await;

        match result {
            Err(WsFrame::AuthError { code, message }) => {
                assert_eq!(code, "invalid_token", "expected code=invalid_token");
                assert!(!message.is_empty(), "message should be non-empty");
            }
            Err(other) => panic!("expected AuthError, got: {:?}", other),
            Ok(o) => panic!("expected Err(AuthError), got Ok: {:?}", o),
        }

        // 失败后: 仍未认证, 无心跳
        assert!(!hub.is_authenticated(&conn_id).await);
        assert!(hub.last_heartbeat_at(&conn_id).await.is_none());
    }

    /// 测试 7 (Stage 2): `handle_heartbeat` 后 `last_heartbeat_at` 比之前新.
    ///
    /// 两步: 第一次心跳记录 T1, 短暂等待, 第二次心跳记录 T2, 断言 T2 > T1.
    #[tokio::test]
    async fn test_heartbeat_updates_last_heartbeat_at() {
        let hub = test_hub();
        let (tx, _rx) = mpsc::unbounded_channel::<Message>();
        let conn_id = hub
            .register(ConnectionType::Device, "machine-1".into(), tx)
            .await;

        // 初始: 无心跳
        assert!(
            hub.last_heartbeat_at(&conn_id).await.is_none(),
            "no heartbeat before any Heartbeat frame"
        );

        // 第一次心跳
        let frame1 = WsFrame::Heartbeat { ts: 1000 };
        let ack1 = hub.handle_heartbeat(&conn_id, &frame1).await;
        match ack1 {
            WsFrame::HeartbeatAck { ts } => assert_eq!(ts, 1000, "ack should echo ts"),
            other => panic!("expected HeartbeatAck, got: {:?}", other),
        }
        let t1 = hub
            .last_heartbeat_at(&conn_id)
            .await
            .expect("last_heartbeat_at after first heartbeat");

        // 等一会儿 (chrono::DateTime<Utc> 精度 = ns, tokio sleep 10ms 足够区分)
        tokio::time::sleep(Duration::from_millis(20)).await;

        // 第二次心跳
        let frame2 = WsFrame::Heartbeat { ts: 2000 };
        let ack2 = hub.handle_heartbeat(&conn_id, &frame2).await;
        match ack2 {
            WsFrame::HeartbeatAck { ts } => assert_eq!(ts, 2000, "ack should echo ts"),
            other => panic!("expected HeartbeatAck, got: {:?}", other),
        }
        let t2 = hub
            .last_heartbeat_at(&conn_id)
            .await
            .expect("last_heartbeat_at after second heartbeat");

        // T2 必须严格大于 T1
        assert!(
            t2 > t1,
            "second heartbeat ({t2}) should be strictly later than first ({t1})"
        );
    }

    /// 额外 (Stage 2): unregister 清理所有 per-conn state (authed/node/meta/heartbeat).
    #[tokio::test]
    async fn test_unregister_clears_auth_and_heartbeat_state() {
        let hub = test_hub();
        seed_token(&hub, "test_token_dt_xxx", "machine_1");
        let (tx, _rx) = mpsc::unbounded_channel::<Message>();
        let conn_id = hub
            .register(ConnectionType::Device, "pending".into(), tx)
            .await;

        // 认证 + register 一下, 把 4 个 map 都填上
        let auth_frame = WsFrame::Auth {
            device_token: "test_token_dt_xxx".into(),
            machine_id: "machine_1".into(),
        };
        hub.authenticate(&conn_id, &auth_frame)
            .await
            .expect("auth should succeed");
        let reg_frame = WsFrame::Register {
            device_id: "dev_1".into(),
            name: "office-pc".into(),
            host_type: "windows".into(),
            host_id: "win-pc-01".into(),
            tags: vec!["workstation".into()],
            capabilities: vec!["chat".into(), "tools".into()],
            daemon_version: "0.3.0".into(),
            os: "windows-11-23H2".into(),
            cpu_cores: 16,
            memory_mb: 32768,
        };
        let reg_resp = hub.register_device(&conn_id, &reg_frame).await;
        assert!(matches!(reg_resp, WsFrame::RegisterOk { .. }));

        assert!(hub.is_authenticated(&conn_id).await);
        assert!(hub.node_id_for(&conn_id).await.is_some());
        assert!(hub.device_meta_for(&conn_id).await.is_some());
        assert!(hub.last_heartbeat_at(&conn_id).await.is_some());

        // unregister
        hub.unregister(&conn_id).await;

        // 全部清空
        assert!(!hub.is_authenticated(&conn_id).await);
        assert!(hub.node_id_for(&conn_id).await.is_none());
        assert!(hub.device_meta_for(&conn_id).await.is_none());
        assert!(hub.last_heartbeat_at(&conn_id).await.is_none());
    }
}

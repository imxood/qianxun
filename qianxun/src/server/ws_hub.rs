//! WebSocket Hub — Stage 1 骨架.
//!
//! 设计目标 (Stage 1 最小集):
//! 1. 维护活跃连接的注册表: `internal_id → Connection` + 双索引 (`by_device`, `by_user`).
//! 2. 提供 `register` / `unregister` / `push_to_user` / `push_to_device` / `stats` API.
//! 3. **不**实现 auth / outbox / heartbeat manager / rate-limit / pending_command 跟踪 —
//!    那些是 Stage 2-3 的事.
//!
//! 并发模型:
//! - `Arc<RwLock<HashMap<...>>>` 而非 `DashMap`: workspace 没有 dashmap 依赖, 而 Stage 1
//!   读多写少, RwLock 足够. Stage 2+ 评估是否升级到 `DashMap` 应对 100+ 并发连接.
//! - `tx: UnboundedSender<Message>`: Hub 把消息扔进 channel, 真正的写循环在
//!   `handle_socket` 里 select 这个 channel. 失败时 `unregister` 兜底.

use axum::extract::ws::Message;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

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
    /// Device: machine_id; App: user_id.
    /// Stage 1 统一用 `principal_id` 描述"这个连接代表谁", 不区分.
    pub principal_id: String,
    /// 把消息发到这个 channel → 由调用方绑定的写循环 select.
    pub tx: mpsc::UnboundedSender<Message>,
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

/// WebSocket Hub — 维护所有活跃连接 + 消息路由.
#[derive(Debug, Default)]
pub struct WsHub {
    /// 内部 id → Connection
    connections: Arc<RwLock<HashMap<String, Arc<Connection>>>>,
    /// 索引: device → 内部 id 列表 (一对多, dev 机可能多 connection)
    by_device: Arc<RwLock<HashMap<String, Vec<String>>>>,
    /// 索引: app user → 内部 id 列表 (一对多, user 可多端登录)
    by_user: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl WsHub {
    pub fn new() -> Self {
        Self::default()
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
    use axum::extract::ws::Message;
    use std::time::Duration;
    use tokio::time::timeout;

    /// 测试 1: register 1 个 device, stats().devices == 1, unregister 后 == 0.
    #[tokio::test]
    async fn test_register_and_unregister() {
        let hub = WsHub::new();
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
        let hub = WsHub::new();

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
        let hub = WsHub::new();

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
        let hub = WsHub::new();
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
}

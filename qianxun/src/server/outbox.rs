//! 消息 outbox (Stage 6a): 节点离线时缓冲, 重连后 replay. SQLite 持久化.
//!
//! persist/persist_call_count/user_count 暂未调用, 留 Phase 4.
#![allow(dead_code)]
//!
//! ## 设计
//!
//! 按 `node_id` 索引的持久化队列, 存 `outbox` 表:
//!
//! - `enqueue(node_id, request_id, event)` — 插一行, `delivered=0`.
//! - `drain(node_id)` — 拿该 node_id 所有 `delivered=0` 的事件 (FIFO by id),
//!   标 `delivered=1`, 返回 events JSON.
//!
//! ## Stage 6a 简化
//!
//! - **单表 + 单 connection**: `Arc<Mutex<Connection>>`, 跟 `team_db` 风格一致.
//!   单 VPS 实例无高并发写需求, WAL 模式 + 串行化写入足够.
//! - **delivered 是 0/1 标志位**: 真正的"投递确认"(device ack) 在 Stage 6c 接 WS
//!   客户端 ack 后再加, 本次只做"已 drain"标记. 即 drained ≠ 已送达, 仅是"已取出
//!   推给重连的 conn".
//! - **不限制容量 + 不带 TTL**: Stage 7 加 cap=256 + 过期清理.
//! - **不接 metrics**: Stage 6b 加 `outbox.size(node_id)` 暴露.
//! - **不接 WS 客户端实际消息路由**: Stage 6c 才接 Event 帧回包, 这次只持久化
//!   + drain.
//!
//! ## 调用方
//!
//! - `mod.rs::handle_prompt_frame`: 验证 RBAC + 限流后, 推 prompt 给 target_node,
//!   并把同一事件入 outbox (节点离线时落盘, 重连后 `ws_hub::authenticate` 触发 drain).
//! - `ws_hub.rs::authenticate`: device auth 成功后, 调 `outbox.drain(machine_id)`
//!   拿所有未送达事件, 通过 conn 的 tx channel 顺序推给新连接.
//!
//! ## 并发
//!
//! `Arc<Mutex<Connection>>` (std::sync::Mutex). 跟 team_db 风格一致 — 写串行, 单
//! VPS 实例不构成瓶颈. 阻塞锁在 async 上下文里是简化权衡, Stage 7 评估换 r2d2 pool.

use chrono::Utc;
use rusqlite::{params, Connection, Result as SqlResult};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// 消息 outbox: 节点离线时缓冲 (SQLite 持久化), 重连后 replay.
///
/// ## 线程安全
///
/// 内部用 `Arc<Mutex<Connection>>` 序列化所有访问. 多线程并发安全,
/// 写操作串行执行 (单 VPS 实例无高并发需求).
#[derive(Clone)]
pub struct Outbox {
    conn: Arc<Mutex<Connection>>,
}

impl Outbox {
    /// 打开或创建 outbox 数据库, 应用 DDL.
    ///
    /// # Errors
    /// - 路径无权限 / 父目录创建失败
    /// - SQL 语法错 (schema 是硬编码, 实际不会触发)
    pub fn new(path: &Path) -> SqlResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        }
        let conn = Connection::open(path)?;
        // 与 team_db 一致: WAL + foreign_keys
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::apply_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 从已有 Connection 构造 (Stage 6a: 与 TeamDb 共享同一 vps.db).
    ///
    /// 与 `new` 不同: 不创建父目录, 不打开文件, 直接应用 schema.
    /// 用于嵌入到 `mod.rs::init_db` 已经持有 Connection 的场景.
    pub fn from_connection(conn: Connection) -> SqlResult<Self> {
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Self::apply_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn apply_schema(conn: &Connection) -> SqlResult<()> {
        conn.execute_batch(SCHEMA_SQL)
    }

    /// 把 event 入对应 node 的缓冲 (`delivered=0`).
    ///
    /// # Errors
    /// - SQLite 写入失败 (disk full / IO error / db locked)
    /// - `event` JSON 序列化失败 (基本不会触发, `serde_json::Value` 都能 to_string)
    pub fn enqueue(
        &self,
        node_id: &str,
        request_id: &str,
        event: &serde_json::Value,
    ) -> SqlResult<()> {
        let event_json = serde_json::to_string(event).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?;
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO outbox (node_id, request_id, event_json, created_at, delivered)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![node_id, request_id, event_json, now],
        )?;
        tracing::debug!(node_id = %node_id, request_id = %request_id, "outbox enqueued");
        Ok(())
    }

    /// 当节点重连, 拿该节点所有 `delivered=0` 的事件 (FIFO by id), 标 `delivered=1`.
    ///
    /// node_id 不存在 / 没有 undelivered 事件 → 返回空 vec (非错误).
    ///
    /// **重要**: "drained" 仅表示"已从 outbox 取出推给重连的 conn", **不**意味着
    /// 设备已收到 / 处理完. 真正的 ack 在 Stage 6c 接 WS 客户端 EventAck 后再加,
    /// 把 `delivered` 重新升级为三态 (pending / in_flight / acked).
    ///
    /// # Errors
    /// - SQLite 读 / 写失败
    /// - `event_json` 反序列化失败 (schema 漂移, 实际不会触发)
    pub fn drain(&self, node_id: &str) -> SqlResult<Vec<serde_json::Value>> {
        let conn = self.conn.lock().unwrap();
        // 用事务包住 select + update, 保证读到的每条都标 delivered=1 (没有
        // 中间态被另一个 drain 看到). 单 connection 串行化已经够了, 但事务
        // 让代码意图更清晰, 也方便 Stage 7 加 TTL 清理时不破坏 drain 语义.
        let tx = conn.unchecked_transaction()?;
        let mut events: Vec<serde_json::Value> = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT event_json FROM outbox
                 WHERE node_id = ?1 AND delivered = 0
                 ORDER BY id ASC",
            )?;
            let rows = stmt.query_map(params![node_id], |row| {
                let s: String = row.get(0)?;
                Ok(s)
            })?;
            for row in rows {
                let s = row?;
                let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                events.push(v);
            }
        }
        // 标 delivered=1 — 用同一事务, 保证与上面 SELECT 一致
        tx.execute(
            "UPDATE outbox SET delivered = 1
             WHERE node_id = ?1 AND delivered = 0",
            params![node_id],
        )?;
        tx.commit()?;
        tracing::debug!(node_id = %node_id, count = events.len(), "outbox drained");
        Ok(events)
    }

    /// 调试用: 该 node_id 当前 `delivered=0` 的事件数 (即"待 replay" backlog).
    pub fn list_undelivered_count(&self, node_id: &str) -> SqlResult<u32> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM outbox WHERE node_id = ?1 AND delivered = 0",
            params![node_id],
            |row| row.get(0),
        )?;
        Ok(n as u32)
    }
}

// ─── Schema DDL ───────────────────────────────────────────

/// outbox 表 + 索引. 与 team_db 共享 vps.db 文件, 但走独立 connection (WAL 模式
/// 允许多 connection 共存).
const SCHEMA_SQL: &str = r#"
-- === Outbox (Stage 6a: 节点离线缓冲) ===
-- delivered: 0 = pending, 1 = drained (已从 outbox 取出推给重连 conn, 不代表设备 ack)
CREATE TABLE IF NOT EXISTS outbox (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id     TEXT NOT NULL,
    request_id  TEXT NOT NULL,
    event_json  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    delivered   INTEGER NOT NULL DEFAULT 0
);
-- 查询热点: "drain(node_id) 拿 undelivered 事件" + list_undelivered_count
-- 复合索引覆盖 (node_id, delivered) + id (隐含 rowid 序).
CREATE INDEX IF NOT EXISTS idx_outbox_node_undelivered ON outbox(node_id, delivered);
"#;

// ─── 单测 ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造 in-memory Outbox (不写文件). WAL 在 in-memory 模式下不生效, 单 conn
    /// 串行化与 file 模式行为等价 (单线程测试).
    fn test_outbox() -> Outbox {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        Outbox::from_connection(conn).expect("from_connection")
    }

    /// 测试 1: enqueue 3 个 event 给 node_a, drain 拿 3 个, 顺序保持 (FIFO by id).
    #[test]
    fn test_enqueue_and_drain() {
        let outbox = test_outbox();
        outbox
            .enqueue("node_a", "req_1", &json!({"i": 0}))
            .expect("enqueue 0");
        outbox
            .enqueue("node_a", "req_2", &json!({"i": 1}))
            .expect("enqueue 1");
        outbox
            .enqueue("node_a", "req_3", &json!({"i": 2}))
            .expect("enqueue 2");

        let drained = outbox.drain("node_a").expect("drain");
        assert_eq!(drained.len(), 3, "expected 3 events drained");

        // 顺序保持 (FIFO by id)
        assert_eq!(drained[0]["i"], 0);
        assert_eq!(drained[1]["i"], 1);
        assert_eq!(drained[2]["i"], 2);

        // drain 后再 drain 返空
        let empty = outbox.drain("node_a").expect("drain again");
        assert!(empty.is_empty(), "drain should clear undelivered buffer");

        // list_undelivered_count 也确认 0
        let count = outbox
            .list_undelivered_count("node_a")
            .expect("count");
        assert_eq!(count, 0, "undelivered count should be 0 after drain");
    }

    /// 测试 2: enqueue 2 个给 node_a + 1 个给 node_b, drain(node_a) 拿 2 个,
    /// node_b 的 1 个仍在 (不互相影响).
    #[test]
    fn test_drain_only_for_specific_node() {
        let outbox = test_outbox();
        outbox
            .enqueue("node_a", "req_a1", &json!({"prompt": "hello-a-1"}))
            .expect("a1");
        outbox
            .enqueue("node_b", "req_b1", &json!({"prompt": "hello-b-1"}))
            .expect("b1");
        outbox
            .enqueue("node_a", "req_a2", &json!({"prompt": "hello-a-2"}))
            .expect("a2");

        // node_a drain 拿 2 个
        let drained_a = outbox.drain("node_a").expect("drain a");
        assert_eq!(drained_a.len(), 2, "node_a has 2 events");
        assert_eq!(drained_a[0]["prompt"], "hello-a-1");
        assert_eq!(drained_a[1]["prompt"], "hello-a-2");

        // node_b 的 1 个事件还在 (没被 node_a 的 drain 误删)
        let drained_b = outbox.drain("node_b").expect("drain b");
        assert_eq!(drained_b.len(), 1, "node_b still has 1 event");
        assert_eq!(drained_b[0]["prompt"], "hello-b-1");

        // node_a 二次 drain 拿空
        let drained_a_again = outbox.drain("node_a").expect("drain a again");
        assert!(drained_a_again.is_empty(), "node_a already drained");

        // 验证 list_undelivered_count 也按 node 隔离
        let pending_a = outbox.list_undelivered_count("node_a").expect("count a");
        assert_eq!(pending_a, 0, "node_a all drained");
        let pending_b = outbox.list_undelivered_count("node_b").expect("count b");
        assert_eq!(pending_b, 0, "node_b drained in this test");
    }

    /// 测试 3: drain 后, 再次 drain 同一 node_id 拿空 (delivered 标记生效).
    #[test]
    fn test_drain_marks_delivered() {
        let outbox = test_outbox();
        outbox
            .enqueue("node_x", "req_x", &json!({"x": 1, "payload": "hello"}))
            .expect("enqueue");

        // 第一次 drain 拿 1 个
        let first = outbox.drain("node_x").expect("drain 1");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["x"], 1);
        assert_eq!(first[0]["payload"], "hello");

        // 第二次 drain 拿空
        let second = outbox.drain("node_x").expect("drain 2");
        assert!(second.is_empty(), "drain twice should return empty");

        // undelivered count 确认 0
        let count = outbox
            .list_undelivered_count("node_x")
            .expect("count");
        assert_eq!(count, 0, "undelivered count should be 0 after drain");

        // drain 一个不存在 / 已 delivered 完的 node 也不报错
        let empty = outbox.drain("nonexistent_node").expect("drain missing");
        assert!(empty.is_empty(), "drain missing node returns empty vec");
    }
}

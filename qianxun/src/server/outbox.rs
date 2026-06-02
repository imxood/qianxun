//! 消息 outbox (Stage 5): 节点离线时缓冲, 重连后 replay.
//!
//! ## 设计
//!
//! 按 `node_id` 索引的 in-memory 队列:
//!
//! - `enqueue(node_id, event)` — 把消息入对应 node 的缓冲.
//! - `drain(node_id)` — 节点重连时调用, 返回该 node 所有未送达的事件并清空.
//!
//! ## Stage 5 简化
//!
//! - **in-memory `HashMap<node_id, Vec<Value>>`**, 进程重启即丢. Stage 6
//!   改 SQLite 持久化 (`outbox` 表) + 带 TTL 自动清理.
//! - **不限制容量**: 单个 node 的缓冲可以无限增长. Stage 6 加 cap=256,
//!   满后丢老 + 触发 `event_error` 给 app. 见 `02-vps-server.md` §6.5.
//! - **不区分 event 类型**: 入的是 `serde_json::Value` (WS frame JSON 序列化).
//!   Stage 6 改为结构化 `PendingOutbound { request_id, message, deadline }`.
//! - **不接 metrics**: Stage 6 暴露 `outbox.size(node_id)` + `outbox.total`.
//!
//! ## 调用方
//!
//! - `mod.rs::handle_prompt_frame`: 验证 RBAC + 限流后, 推 prompt 给 target_node,
//!   并把同一事件入 outbox (Stage 5 不区分 in-flight / pending, 一律入).
//! - Stage 6+ 会在 `ws_hub.rs::WsHub::register_device` 节点重连时调
//!   `outbox.drain(node_id)` 顺序 flush, 见 `02-vps-server.md` §6.5.
//!
//! ## 并发
//!
//! `Arc<Mutex<HashMap<...>>>`. 用 `tokio::sync::Mutex` 而非 `std::sync::Mutex`,
//! 因为 `drain` 可能在 `ws_hub` 持有读锁时调用 (`drain_for` 之后会更新
//! `last_heartbeat` 等), 跨 await 一致更好.
//!
//! ## 测试
//!
//! 见末尾 `tests::test_enqueue_and_drain` — 同一 node 入 3 个事件, drain 全拿.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 消息 outbox: 节点离线时缓冲, 重连后 replay.
#[derive(Clone)]
pub struct Outbox {
    inner: Arc<Mutex<HashMap<String, Vec<serde_json::Value>>>>,
}

impl Outbox {
    /// 构造空 outbox.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 把 event 入对应 node 的缓冲. 若 node_id 首次出现, 自动建空队列.
    pub async fn enqueue(&self, node_id: &str, event: serde_json::Value) {
        let mut map = self.inner.lock().await;
        map.entry(node_id.to_string())
            .or_insert_with(Vec::new)
            .push(event);
        tracing::debug!(node_id = %node_id, "outbox enqueued");
    }

    /// 当节点重连, 返回该节点所有未送达的事件并从缓冲中清空.
    /// node_id 不存在时返回空 vec (非错误).
    pub async fn drain(&self, node_id: &str) -> Vec<serde_json::Value> {
        let mut map = self.inner.lock().await;
        let drained = map.remove(node_id).unwrap_or_default();
        tracing::debug!(node_id = %node_id, count = drained.len(), "outbox drained");
        drained
    }

    /// 当前 node 数量 (测试用 / metrics 暴露用).
    pub async fn node_count(&self) -> usize {
        self.inner.lock().await.len()
    }

    /// 指定 node 当前缓冲长度 (测试用). 不存在返 0.
    pub async fn len(&self, node_id: &str) -> usize {
        self.inner
            .lock()
            .await
            .get(node_id)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

impl Default for Outbox {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 单测 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 测试: 同一 node 入 3 个事件, drain 全拿且清空.
    ///
    /// 验证 enqueue + drain 的基本正确性:
    /// - drain 后内部缓冲为空 (再 drain 返空 vec).
    /// - 返回顺序与入队顺序一致 (Stage 6 可能改 LIFO, 但本版本 FIFO).
    #[tokio::test]
    async fn test_enqueue_and_drain() {
        let outbox = Outbox::new();

        outbox.enqueue("node_1", json!({"type": "event", "i": 0})).await;
        outbox.enqueue("node_1", json!({"type": "event", "i": 1})).await;
        outbox.enqueue("node_1", json!({"type": "event", "i": 2})).await;

        let drained = outbox.drain("node_1").await;
        assert_eq!(drained.len(), 3, "expected 3 events drained");

        // 顺序保持 (FIFO)
        assert_eq!(drained[0]["i"], 0);
        assert_eq!(drained[1]["i"], 1);
        assert_eq!(drained[2]["i"], 2);

        // drain 后再 drain 返空
        let empty = outbox.drain("node_1").await;
        assert!(empty.is_empty(), "drain should clear the buffer");

        // 其他 node 不受影响
        outbox.enqueue("node_2", json!({"x": 1})).await;
        assert_eq!(outbox.drain("node_2").await.len(), 1);
        assert_eq!(outbox.drain("node_1").await.len(), 0);
    }
}

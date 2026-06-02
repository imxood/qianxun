//! 用户级 token bucket 限流 (Stage 5) + 文件持久化 (Stage 6b).
//!
//! ## 设计
//!
//! 限流维度: per-user_id. 不区分 device/app/msg type.
//!
//! 参数 (Stage 5 硬编码, Stage 6 走 env 覆盖, 见 `02-vps-server.md` §6.6):
//! - **容量 (burst)**: 10 token — 单次允许突发 10 个请求
//! - **补充速率**: 1 token / sec (= 60 token / min, 与 `02-vps-server.md` §6.6 表对齐)
//!
//! ## 行为
//!
//! - `check(user_id)`: 扣 1 token. 有 token → `Ok(())`, 无 token → `Err(TooMany)`.
//! - 首次见到 user_id: 建桶, 满 token (= 10).
//! - 后台无回收 task — 仅在 `check` 调用时按时间差补充 token, 单线程安全.
//!
//! ## Stage 6b 持久化
//!
//! Token bucket 状态可持久化到 JSON 文件 (`~/.qianxun/rate-limit.json`).
//! - **节流**: 每次 `check` 后, 若距上次 `persist` ≥ 5s 才写盘, 避免高频 IO.
//! - **崩溃安全**: 写盘是 best-effort (直接覆盖, 不做 atomic rename), 进程崩溃
//!   接受最多 5s 的状态丢失.
//! - **冷启动**: `RateLimiter::with_persist(path)` → 构造 → 显式 `load()` 恢复
//!   状态. 跨重启 token 数会按 `(now - last_refill_unix) * refill_rate` 继续补充,
//!   上限 burst — 长时间停机后基本会"满血复活" (`min(BURST, saved + elapsed)`).
//! - **时区/时钟**: 用 `Instant` (单调) 做内存时基, `unix_secs` 做持久化时基,
//!   启动时记录 `(boot_instant, boot_unix)` 做桥接, 不受 wall clock 漂移影响.
//!
//! ## Stage 5/6 简化
//!
//! - in-memory `Arc<Mutex<HashMap>>`, **per-process**. 多实例 / 多节点部署会
//!   各自独立计数 (Stage 7 改 Redis 或 sticky session).
//! - 不做 per-IP / per-conn / per-msg-size 维度限流.
//! - 不做 metrics 暴露 (`metrics::RATE_LIMITED.inc()`), Stage 6+ 接 `tracing` + `metrics`.
//!
//! ## 测试
//!
//! 见末尾 `tests` 模块 — 4 个测试, 覆盖核心不变量 + 持久化 roundtrip.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// 限流参数 (Stage 5 硬编码).
///
/// - `BURST` — 桶容量, 允许的瞬时突发量.
/// - `REFILL_PER_SEC` — token 补充速率 (token / sec).
const BURST: f64 = 10.0;
const REFILL_PER_SEC: f64 = 1.0; // 60 token / 60 sec

/// 持久化节流间隔 (秒). `check` 后若距上次 `persist` < 5s, 跳过写盘.
const PERSIST_THROTTLE_SECS: u64 = 5;

/// 单个用户的 token bucket (运行时表示).
#[derive(Debug)]
struct Bucket {
    /// 当前可用 token 数.
    tokens: f64,
    /// 上次补充时间 (用于按时间差累加).
    last_refill: Instant,
}

impl Bucket {
    fn new_full() -> Self {
        Self {
            tokens: BURST,
            last_refill: Instant::now(),
        }
    }

    /// 按 `now - last_refill` 累加 token, 上限 `BURST`.
    fn refill(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * REFILL_PER_SEC).min(BURST);
            self.last_refill = now;
        }
    }

    /// 尝试扣 1 token. 成功 → `true`, 失败 → `false`.
    fn try_consume(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// 持久化的 bucket 表示 — 只保留可序列化的标量.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PersistedBucket {
    /// 持久化时刻的 token 数.
    tokens: f64,
    /// 持久化时刻的 unix 时间戳 (秒).
    last_refill_unix: u64,
}

impl PersistedBucket {
    /// 从运行时 `Bucket` 序列化, 借助 `(boot_instant, boot_unix)` 桥接.
    fn from_runtime(b: &Bucket, boot_instant: Instant, boot_unix: u64) -> Self {
        // `Instant` 是单调时钟, 不可与 wall clock 直接转换.
        // 我们记录启动时的 (instant, unix), 用 elapsed offset 推算 last_refill 的 unix.
        // last_refill >= boot_instant (refill 不会回拨), 用 saturating_add 防御.
        let elapsed_secs = b.last_refill.duration_since(boot_instant).as_secs();
        Self {
            tokens: b.tokens,
            last_refill_unix: boot_unix.saturating_add(elapsed_secs),
        }
    }
}

/// 持久化文件 schema (version 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    /// schema 版本号, 未来升级时做 migration.
    version: u32,
    /// user_id → 持久化桶.
    users: HashMap<String, PersistedBucket>,
}

/// 用户级 rate limiter.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, Bucket>>>,
    /// 持久化文件路径. `None` = 不持久化 (`new()` 默认, 测试用).
    persist_path: Option<PathBuf>,
    /// 上次持久化时间 (用于 5s 节流).
    last_persist_at: Arc<StdMutex<Option<Instant>>>,
    /// 启动时刻 (单调时钟), 用于 `Instant` ↔ `unix_secs` 桥接.
    boot_instant: Instant,
    /// 启动时对应的 unix 时间戳 (秒).
    boot_unix: u64,
    /// 持久化实际调用次数 (测试 + 监控用).
    persist_calls: Arc<AtomicU64>,
}

impl RateLimiter {
    /// 构造空 limiter, **不持久化**. 兼容 Stage 5 调用方 + 单元测试.
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            persist_path: None,
            last_persist_at: Arc::new(StdMutex::new(None)),
            boot_instant: Instant::now(),
            boot_unix: current_unix_secs(),
            persist_calls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 构造带持久化的 limiter. 不会自动 `load` — 留给 caller 显式调,
    /// 方便测试时直接构造空状态.
    pub fn with_persist(persist_path: PathBuf) -> Result<Self, RateError> {
        Ok(Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            persist_path: Some(persist_path),
            last_persist_at: Arc::new(StdMutex::new(None)),
            boot_instant: Instant::now(),
            boot_unix: current_unix_secs(),
            persist_calls: Arc::new(AtomicU64::new(0)),
        })
    }

    /// 检查 user_id 是否允许发送. 允许 → 扣 1 token 返回 `Ok(())`,
    /// 拒绝 → 返回 `Err(RateError::TooMany)`.
    ///
    /// `check` 内部走 `tokio::sync::Mutex`, caller 需在 async 上下文调.
    /// 若 limiter 启用了持久化, 每次成功消费后会触发**节流**写盘
    /// (每 `PERSIST_THROTTLE_SECS` 秒最多一次).
    pub async fn check(&self, user_id: &str) -> Result<(), RateError> {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(user_id.to_string())
            .or_insert_with(Bucket::new_full);
        let allowed = bucket.try_consume(now);

        // 收集待持久化的快照, **释放 lock 后**再做 IO, 避免阻塞其它 check.
        // 失败 (allowed=false) 路径不触发 persist — 状态无变化.
        let snapshot: Vec<(String, PersistedBucket)> = if allowed && self.persist_path.is_some() {
            buckets
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        PersistedBucket::from_runtime(v, self.boot_instant, self.boot_unix),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        drop(buckets);

        if !snapshot.is_empty() {
            self.maybe_persist_throttled(snapshot).await;
        }

        if allowed {
            Ok(())
        } else {
            tracing::debug!(user_id = %user_id, "rate limit exceeded");
            Err(RateError::TooMany)
        }
    }

    /// 节流持久化: 距上次 `persist` < 5s → 跳过.
    ///
    /// 通过 `StdMutex<Option<Instant>>` 串行化, 避免并发 `check` 触发多次写盘.
    async fn maybe_persist_throttled(&self, snapshot: Vec<(String, PersistedBucket)>) {
        let should_persist = {
            let mut last = self.last_persist_at.lock().unwrap();
            match *last {
                Some(t) if t.elapsed().as_secs() < PERSIST_THROTTLE_SECS => false,
                _ => {
                    *last = Some(Instant::now());
                    true
                }
            }
        };
        if !should_persist {
            return;
        }
        if let Err(e) = self.write_persist_now(&snapshot).await {
            tracing::warn!(error = %e, "rate limit throttled persist failed (state lost up to throttle window)");
        }
    }

    /// 实际写盘: `spawn_blocking` 跑 `std::fs`, 不阻塞 async runtime.
    async fn write_persist_now(
        &self,
        snapshot: &[(String, PersistedBucket)],
    ) -> Result<(), RateError> {
        let path = self
            .persist_path
            .as_ref()
            .ok_or_else(|| RateError::Persist("no persist_path configured".into()))?;
        let state = PersistedState {
            version: 1,
            users: snapshot.iter().cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| RateError::Persist(format!("serialize: {e}")))?;
        let path_clone = path.clone();
        let write_result =
            tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
                if let Some(parent) = path_clone.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path_clone, json)
            })
            .await
            .map_err(|e| RateError::Persist(format!("join: {e}")))?;
        write_result.map_err(|e| RateError::Persist(format!("io: {e}")))?;
        self.persist_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// 显式 flush — 写盘不节流, 专供 shutdown / 测试用.
    ///
    /// `persist_path` 为 `None` 时是 noop, 方便无持久化场景.
    pub async fn persist(&self) -> Result<(), RateError> {
        if self.persist_path.is_none() {
            return Ok(());
        }
        let snapshot: Vec<(String, PersistedBucket)> = {
            let buckets = self.buckets.lock().await;
            buckets
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        PersistedBucket::from_runtime(v, self.boot_instant, self.boot_unix),
                    )
                })
                .collect()
        };
        self.write_persist_now(&snapshot).await?;
        // 重置 throttle 锚点, 避免紧接的 `check` 立即再写.
        *self.last_persist_at.lock().unwrap() = Some(Instant::now());
        Ok(())
    }

    /// 启动加载: 从文件恢复 bucket 状态.
    ///
    /// - 文件不存在 → `Ok(())` (视为空 map, 首次启动正常路径).
    /// - 文件存在但解析失败 / version 不匹配 → `Err(RateError::Persist(...))`,
    ///   让 caller 决定是否回退到空 map (Stage 6b: 直接传 Err 给 caller 决定).
    pub async fn load(&self) -> Result<(), RateError> {
        let path = match &self.persist_path {
            Some(p) => p,
            None => return Ok(()), // 无 persist 路径, noop
        };
        if !path.exists() {
            return Ok(());
        }
        let json = std::fs::read_to_string(path)
            .map_err(|e| RateError::Persist(format!("read: {e}")))?;
        let state: PersistedState = serde_json::from_str(&json)
            .map_err(|e| RateError::Persist(format!("parse: {e}")))?;
        if state.version != 1 {
            return Err(RateError::Persist(format!(
                "unsupported schema version: {}",
                state.version
            )));
        }
        let mut buckets = self.buckets.lock().await;
        for (k, pb) in state.users {
            // 把 unix 时间戳转回运行时 Instant, 并按经过的秒数补满 token.
            // 关键: 冷启动后 token = min(BURST, saved + elapsed * REFILL_PER_SEC).
            let now_unix = current_unix_secs();
            let elapsed_secs = now_unix.saturating_sub(pb.last_refill_unix) as f64;
            let tokens = (pb.tokens + elapsed_secs * REFILL_PER_SEC).min(BURST);
            buckets.insert(
                k,
                Bucket {
                    tokens,
                    last_refill: Instant::now(),
                },
            );
        }
        Ok(())
    }

    /// 持久化实际调用次数 (测试 / metrics 用). `Relaxed` ordering 即可 — 仅计数.
    pub fn persist_call_count(&self) -> u64 {
        self.persist_calls.load(Ordering::Relaxed)
    }

    /// 当前 user 数量 (测试用 / metrics 暴露用).
    pub async fn user_count(&self) -> usize {
        self.buckets.lock().await.len()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// 限流失败原因. Stage 5 仅一种, Stage 6b 加 `Persist(String)` 用于 IO/schema 错误.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateError {
    /// 用户当前桶空, 拒绝.
    TooMany,
    /// 持久化 / 加载失败 (IO, JSON 解析, schema version 不匹配等).
    /// 携带原因字符串, 用于日志 / 上报, 不暴露给客户端.
    Persist(String),
}

impl std::fmt::Display for RateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany => write!(f, "rate limit exceeded"),
            Self::Persist(s) => write!(f, "rate limit persist error: {s}"),
        }
    }
}

impl std::error::Error for RateError {}

/// 当前 unix 时间戳 (秒). `SystemTime` 错误兜底返 0 — 仅用于持久化时基,
/// 兜底会让 `load` 算出极大的 elapsed_secs 把 bucket 补满, 不会出 panic.
fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ─── 单测 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成一个临时路径 (uuid 命名避免并发测试冲突), 路径不含文件 — 测试自己创建.
    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "qx-ratelimit-{}-{}.json",
            name,
            uuid::Uuid::new_v4()
        ))
    }

    /// 测试: 70 次连续 check, 第 1-10 通过, 第 11-70 全部 TooMany.
    ///
    /// 验证 burst=10 的核心不变量: 单次突发允许 10, 之后桶空 (无时间流逝 → 不补充),
    /// 所以 11-70 全部拒绝. 总数 70 = 10 pass + 60 fail.
    #[tokio::test]
    async fn test_burst_then_throttle() {
        let limiter = RateLimiter::new();

        // 1-10: 全部 Ok
        for i in 1..=10 {
            let r = limiter.check("user_alice").await;
            assert!(r.is_ok(), "check #{i} should pass (burst=10), got: {r:?}");
        }

        // 11-70: 全部 Err(TooMany)
        for i in 11..=70 {
            let r = limiter.check("user_alice").await;
            assert_eq!(
                r,
                Err(RateError::TooMany),
                "check #{i} should be TooMany (no time for refill)"
            );
        }
    }

    /// 测试: 强制消耗掉全部 token 后, sleep 1.1s 应该能再 1 个.
    #[tokio::test]
    async fn test_refill_after_time() {
        let limiter = RateLimiter::new();
        // 10 次通过
        for _ in 0..10 {
            limiter.check("u").await.unwrap();
        }
        // 立即再 check → TooMany
        assert_eq!(limiter.check("u").await, Err(RateError::TooMany));
        // 等 1.1s 让 1 token 补回来
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        // 1 个补回, 通过
        assert!(limiter.check("u").await.is_ok());
        // 立即再 check → TooMany (还没补第 2 个)
        assert_eq!(limiter.check("u").await, Err(RateError::TooMany));
    }

    /// 测试: 不同 user 各自独立桶. user_alice 限流不影响 user_bob.
    #[tokio::test]
    async fn test_per_user_isolation() {
        let limiter = RateLimiter::new();

        // alice 用完 burst
        for _ in 0..10 {
            limiter.check("alice").await.unwrap();
        }
        assert_eq!(limiter.check("alice").await, Err(RateError::TooMany));

        // bob 完全不受影响
        for i in 1..=10 {
            limiter.check("bob").await.expect("bob should have its own bucket");
        }
    }

    /// 测试: persist → load 完整 roundtrip.
    ///
    /// 1. limiter1 消费 1 token, 显式 persist().
    /// 2. 验证文件内容: version=1, alice.tokens ≈ 9.0.
    /// 3. limiter2 用同一路径, load() 后 user_count == 1.
    /// 4. 验证 limiter2 中 alice 桶非空: 至少能消费 9 个 (可能 10, 若 load 跨越了
    ///    1s 边界 refill 一次), 至多消费 10 个后 fail.
    #[tokio::test]
    async fn test_persist_and_load_roundtrip() {
        let path = temp_path("roundtrip");

        // 1. 第一次: 消费 1 token. 第 1 次 check 触发节流写盘 (count=1),
        //    显式 persist() 绕过节流再写一次 (count=2).
        let limiter1 = RateLimiter::with_persist(path.clone()).unwrap();
        limiter1.check("alice").await.unwrap();
        // 等节流写盘完成, 再调显式 persist, 避免两次写盘并发
        // (显式 persist 内部也加 throttle 锚点, 不会和 check 抢).
        limiter1.persist().await.unwrap();
        assert_eq!(
            limiter1.persist_call_count(),
            2,
            "expected 1 throttled (from check) + 1 explicit = 2, got {}",
            limiter1.persist_call_count()
        );

        // 2. 验证文件内容 (不依赖时间算术).
        let json = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["version"], 1);
        let tokens = v["users"]["alice"]["tokens"]
            .as_f64()
            .expect("alice.tokens should be f64");
        assert!(
            (tokens - 9.0).abs() < 0.001,
            "expected alice.tokens ≈ 9.0, got {tokens}"
        );
        assert!(v["users"]["alice"]["last_refill_unix"].is_u64());

        // 3. 第二次: 重建 limiter, 加载后 user_count == 1.
        let limiter2 = RateLimiter::with_persist(path.clone()).unwrap();
        limiter2.load().await.unwrap();
        assert_eq!(limiter2.user_count().await, 1);

        // 4. 验证桶状态恢复: 至少 9 个 pass, 至多 10 个 pass.
        //    (同秒内 = 9 pass, 跨过 1s 边界 = 10 pass)
        let mut passes = 0;
        for _ in 0..12 {
            if limiter2.check("alice").await.is_ok() {
                passes += 1;
            } else {
                break;
            }
        }
        assert!(
            (9..=10).contains(&passes),
            "expected 9 or 10 passes (proving bucket restored with ≤9 tokens), got {passes}"
        );

        // 清理
        let _ = std::fs::remove_file(&path);
    }

    /// 测试: 持久化节流到 5s 间隔.
    ///
    /// 用 10 个**不同** user 各 check 一次, 验证节流:
    /// - 第 1 个 check 触发节流写盘 (count=1).
    /// - 第 2-10 个 check 因 < 5s 被节流跳过 (count 仍 =1).
    /// - 再 1 个 check 仍被节流 (count 仍 =1).
    /// - 显式 persist() 绕过节流再写一次 (count=2).
    /// - 用不同 user 避免单个用户桶被消耗光 (burst=10) 导致后续 check 返 Err.
    #[tokio::test]
    async fn test_persist_throttled_to_5s_interval() {
        let path = temp_path("throttle");
        let limiter = RateLimiter::with_persist(path.clone()).unwrap();

        // 10 个不同 user 各 check 1 次: 第 1 次 persist, 第 2-10 次跳过.
        for i in 0..10 {
            limiter
                .check(&format!("user_{i}"))
                .await
                .expect("first check per user should pass (full bucket)");
        }
        // 等 spawn_blocking 写盘完成.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            limiter.persist_call_count(),
            1,
            "10 rapid checks on distinct users should produce exactly 1 persist (first one), got {}",
            limiter.persist_call_count()
        );

        // 再来 1 个 check (新 user), 距上次 < 5s, 仍应被节流.
        limiter.check("user_extra").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            limiter.persist_call_count(),
            1,
            "11th check (within 5s) should not persist, got {}",
            limiter.persist_call_count()
        );

        // 显式 persist() 强制写一次 (不节流), 用于验证手动 flush 路径.
        limiter.persist().await.unwrap();
        assert_eq!(
            limiter.persist_call_count(),
            2,
            "explicit persist() should bypass throttle"
        );

        // 清理
        let _ = std::fs::remove_file(&path);
    }

    /// 测试: load() 在文件不存在时返 Ok(空 map), 不报错.
    #[tokio::test]
    async fn test_load_missing_file_creates_empty() {
        let path = temp_path("missing");
        assert!(!path.exists(), "precondition: path should not exist");

        let limiter = RateLimiter::with_persist(path.clone()).unwrap();
        let r = limiter.load().await;
        assert!(
            r.is_ok(),
            "load on missing file should be Ok, got: {r:?}"
        );
        assert_eq!(limiter.user_count().await, 0);

        // 文件仍未被创建 (load 不创建文件, 只读).
        assert!(!path.exists(), "load should not create the file");
    }
}

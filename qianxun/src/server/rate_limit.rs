//! 用户级 token bucket 限流 (Stage 5).
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
//! ## Stage 5 简化
//!
//! - in-memory `Arc<Mutex<HashMap>>`, **per-process**. 多实例 / 多节点部署会
//!   各自独立计数 (Stage 6 改 Redis 或 sticky session).
//! - 不做 per-IP / per-conn / per-msg-size 维度限流 (Stage 6+).
//! - 不做 metrics 暴露 (`metrics::RATE_LIMITED.inc()`), Stage 6 接 `tracing` + `metrics`.
//!
//! ## 测试
//!
//! 见末尾 `tests::test_burst_then_throttle` — 70 次连续 check, 第 1-10 成功,
//! 第 11-70 全部 Err(TooMany), 验证 burst=10 + refill=1/sec 的核心不变量.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// 限流参数 (Stage 5 硬编码).
///
/// - `BURST` — 桶容量, 允许的瞬时突发量.
/// - `REFILL_PER_SEC` — token 补充速率 (token / sec).
const BURST: f64 = 10.0;
const REFILL_PER_SEC: f64 = 1.0; // 60 token / 60 sec

/// 单个用户的 token bucket.
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

/// 用户级 rate limiter.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, Bucket>>>,
}

impl RateLimiter {
    /// 构造空 limiter. Stage 5 无需配置参数, 用常量.
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 检查 user_id 是否允许发送. 允许 → 扣 1 token 返回 `Ok(())`,
    /// 拒绝 → 返回 `Err(RateError::TooMany)`.
    ///
    /// 这是 `pub fn` (非 async) 但内部用 `tokio::sync::Mutex`. 调用方需在
    /// `tokio::runtime` 上下文中通过 `blocking_lock` 包装; 但本文件直接用
    /// `lock().await` 即可 — caller 在 async fn 里调.
    pub async fn check(&self, user_id: &str) -> Result<(), RateError> {
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(user_id.to_string())
            .or_insert_with(Bucket::new_full);
        let now = Instant::now();
        if bucket.try_consume(now) {
            Ok(())
        } else {
            tracing::debug!(user_id = %user_id, "rate limit exceeded");
            Err(RateError::TooMany)
        }
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

/// 限流失败原因. Stage 5 仅一种.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateError {
    /// 用户当前桶空, 拒绝.
    TooMany,
}

impl std::fmt::Display for RateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooMany => write!(f, "rate limit exceeded"),
        }
    }
}

impl std::error::Error for RateError {}

// ─── 单测 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}

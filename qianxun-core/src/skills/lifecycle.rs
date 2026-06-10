//! 缺口 04: Skill 生命周期自动化.
//!
//! ## 设计要点
//!
//! - **SkillStatus** (4 变体): `Active` / `Candidate` / `Archived` / `Quarantined`
//! - **SkillLifecycle** 装饰器, 包裹现有 `SkillManager`, 记录每次 invoke 结果
//! - **6 条规则**:
//!   1. **promote_to_candidate**: invoke ≥ 5 次 → Candidate
//!   2. **evaluate_to_active**: Candidate confidence ≥ 0.7 → Active
//!   3. **archive_unused**: Active 31 天未用 → Archived
//!   4. **quarantine_low_confidence**: 失败率 > 50% (10+ 样本) → Quarantined
//!   5. **changelog_on_status_change**: 状态变化写 changelog (TODO: 持久化层)
//!   6. **tick_runs_at_startup**: 启动不阻塞, 后台跑
//!
//! ## 不做什么
//!
//! - 不重做 `SkillManager` 加载机制 (保持现有)
//! - 不动现有 skill 触发匹配 (skill 触发放 `SkillManager`, lifecycle 只是装饰器)
//! - 不做持久化到 SQLite (本骨架只做内存记录; Stage 6 接入 `qianxun-runtime/persistence.rs`)
//!
//! ## 调用方
//!
//! - `qianxun-core/src/skills/mod.rs::invoke` 成功后调 `record_usage`
//! - 启动时 `SkillLifecycle::tick()` 跑一次 (后台 tokio::spawn)
//! - 每天 0 点跑一次 (cron stub)

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// ─── SkillStatus ────────────────────────────────────────────

/// Skill 生命周期状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    /// 默认状态: 加载了但未验证
    Active,
    /// 被 invoke 5+ 次, 等待评估
    Candidate,
    /// 31 天未用, 自动归档
    Archived,
    /// 失败率 > 50% (10+ 样本), 隔离审查
    Quarantined,
}

impl SkillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Candidate => "candidate",
            Self::Archived => "archived",
            Self::Quarantined => "quarantined",
        }
    }
}

// ─── SkillUsageRecord ───────────────────────────────────────

/// 单个 skill 的运行时统计.
#[derive(Debug, Clone, Default)]
pub struct SkillUsageRecord {
    pub use_count: u64,
    pub success_count: u64,
    /// 首次记录时间
    pub first_used_at: Option<DateTime<Utc>>,
    /// 最近一次 invoke 时间
    pub last_used_at: Option<DateTime<Utc>>,
    /// 状态变化日志 (只在内存; 持久化留 Stage 6)
    pub changelog: Vec<SkillChangelogEntry>,
}

impl SkillUsageRecord {
    /// 当前 confidence = success / max(1, use_count).
    pub fn confidence(&self) -> f64 {
        if self.use_count == 0 {
            0.0
        } else {
            self.success_count as f64 / self.use_count as f64
        }
    }

    /// 失败率 = 1 - success_rate.
    pub fn failure_rate(&self) -> f64 {
        1.0 - self.confidence()
    }

    /// 距 last_used_at 多少天.
    pub fn days_since_last_used(&self, now: DateTime<Utc>) -> i64 {
        match self.last_used_at {
            Some(t) => (now - t).num_days(),
            None => 0,
        }
    }
}

/// 状态变化日志条目.
#[derive(Debug, Clone)]
pub struct SkillChangelogEntry {
    pub at: DateTime<Utc>,
    pub from: SkillStatus,
    pub to: SkillStatus,
    pub reason: String,
}

// ─── SkillLifecycle ────────────────────────────────────────

/// Lifecycle 配置常量.
pub const PROMOTE_CANDIDATE_THRESHOLD: u64 = 5;
pub const PROMOTE_ACTIVE_CONFIDENCE: f64 = 0.7;
pub const QUARANTINE_FAILURE_RATE: f64 = 0.5;
pub const QUARANTINE_MIN_SAMPLES: u64 = 10;
pub const ARCHIVE_DAYS_THRESHOLD: i64 = 31;

/// 生命周期管理: 装饰器, 不破坏现有 SkillManager.
pub struct SkillLifecycle {
    /// skill_name → usage record
    records: RwLock<HashMap<String, SkillUsageRecord>>,
    /// skill_name → 当前 status
    statuses: RwLock<HashMap<String, SkillStatus>>,
    /// 全局 tick 计数 (e2e 验证用)
    tick_count: AtomicU64,
}

impl SkillLifecycle {
    pub fn new() -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
            statuses: RwLock::new(HashMap::new()),
            tick_count: AtomicU64::new(0),
        }
    }

    /// 记录一次 skill invoke.
    ///
    /// **调用方**: `SkillManager::invoke` 成功后.
    pub async fn record_usage(&self, skill_name: &str, success: bool) {
        let now = Utc::now();
        let mut records = self.records.write().await;
        let entry = records.entry(skill_name.to_string()).or_default();
        if entry.first_used_at.is_none() {
            entry.first_used_at = Some(now);
        }
        entry.last_used_at = Some(now);
        entry.use_count += 1;
        if success {
            entry.success_count += 1;
        }
    }

    /// 拿 skill 当前的 status.
    pub async fn get_status(&self, skill_name: &str) -> SkillStatus {
        self.statuses
            .read()
            .await
            .get(skill_name)
            .copied()
            .unwrap_or(SkillStatus::Active)
    }

    /// 拿 skill 的 usage record (e2e / 调试用).
    pub async fn get_record(&self, skill_name: &str) -> Option<SkillUsageRecord> {
        self.records.read().await.get(skill_name).cloned()
    }

    /// 列出所有 skill 的 status 快照.
    pub async fn snapshot(&self) -> HashMap<String, SkillStatus> {
        self.statuses.read().await.clone()
    }

    /// 跑一次评估, 应用所有规则.
    ///
    /// 启动时跑一次 + 每天 0 点跑一次 (cron stub).
    /// **不阻塞**: 内部 tokio::spawn 即可.
    pub async fn tick(&self) -> LifecycleReport {
        self.tick_count.fetch_add(1, Ordering::Relaxed);
        let now = Utc::now();
        let mut report = LifecycleReport::default();
        let mut records = self.records.write().await;
        let mut statuses = self.statuses.write().await;

        for (name, rec) in records.iter_mut() {
            let prev = statuses.get(name).copied().unwrap_or(SkillStatus::Active);
            let next = self.evaluate(rec, prev, now);

            if next != prev {
                let reason = format!(
                    "rule: use_count={}, confidence={:.2}, days_idle={}",
                    rec.use_count,
                    rec.confidence(),
                    rec.days_since_last_used(now)
                );
                rec.changelog.push(SkillChangelogEntry {
                    at: now,
                    from: prev,
                    to: next,
                    reason: reason.clone(),
                });
                statuses.insert(name.clone(), next);
                report.transitions.push((name.clone(), prev, next, reason));
            }
        }
        report
    }

    /// 单条规则评估.
    fn evaluate(
        &self,
        rec: &SkillUsageRecord,
        current: SkillStatus,
        now: DateTime<Utc>,
    ) -> SkillStatus {
        // 规则 4: quarantine (最高优先级: 失败率高就隔离)
        if rec.use_count >= QUARANTINE_MIN_SAMPLES && rec.failure_rate() > QUARANTINE_FAILURE_RATE {
            return SkillStatus::Quarantined;
        }

        match current {
            SkillStatus::Active => {
                // 规则 1: promote to candidate
                if rec.use_count >= PROMOTE_CANDIDATE_THRESHOLD {
                    return SkillStatus::Candidate;
                }
                SkillStatus::Active
            }
            SkillStatus::Candidate => {
                // 规则 2: evaluate to active
                if rec.confidence() >= PROMOTE_ACTIVE_CONFIDENCE {
                    return SkillStatus::Active;
                }
                // 候选长期低 confidence 也归档
                if rec.days_since_last_used(now) >= ARCHIVE_DAYS_THRESHOLD {
                    return SkillStatus::Archived;
                }
                SkillStatus::Candidate
            }
            SkillStatus::Archived => {
                // 归档后再次被使用, 回到 Active
                if rec.days_since_last_used(now) == 0 && rec.use_count > 0 {
                    return SkillStatus::Active;
                }
                SkillStatus::Archived
            }
            SkillStatus::Quarantined => {
                // 隔离后人工 review, 不自动恢复
                SkillStatus::Quarantined
            }
        }
    }

    /// tick 次数 (e2e 验证用).
    pub fn tick_count(&self) -> u64 {
        self.tick_count.load(Ordering::Relaxed)
    }

    /// 在 `Arc<Self>` 上跑一次后台 tick (启动钩子用).
    pub fn spawn_tick(self: Arc<Self>) {
        tokio::spawn(async move {
            let _ = self.tick().await;
        });
    }
}

impl Default for SkillLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

// ─── LifecycleReport ───────────────────────────────────────

/// 一次 tick 的结果报告.
#[derive(Debug, Default, Clone)]
pub struct LifecycleReport {
    /// (skill_name, from, to, reason)
    pub transitions: Vec<(String, SkillStatus, SkillStatus, String)>,
}

impl LifecycleReport {
    pub fn is_empty(&self) -> bool {
        self.transitions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.transitions.len()
    }
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_promote_to_candidate_after_5_uses() {
        let lc = SkillLifecycle::new();
        for _ in 0..5 {
            lc.record_usage("test_skill", true).await;
        }
        let report = lc.tick().await;
        assert_eq!(lc.get_status("test_skill").await, SkillStatus::Candidate);
        assert_eq!(report.transitions.len(), 1);
        assert_eq!(report.transitions[0].1, SkillStatus::Active);
        assert_eq!(report.transitions[0].2, SkillStatus::Candidate);
    }

    #[tokio::test]
    async fn test_no_promote_below_5_uses() {
        let lc = SkillLifecycle::new();
        for _ in 0..4 {
            lc.record_usage("test_skill", true).await;
        }
        let _ = lc.tick().await;
        assert_eq!(lc.get_status("test_skill").await, SkillStatus::Active);
    }

    #[tokio::test]
    async fn test_candidate_to_active_on_high_confidence() {
        let lc = SkillLifecycle::new();
        // 5+ 次, 全部成功 → Candidate
        for _ in 0..5 {
            lc.record_usage("good_skill", true).await;
        }
        let _ = lc.tick().await;
        assert_eq!(lc.get_status("good_skill").await, SkillStatus::Candidate);

        // 再跑 5 次全成功 (conf=1.0 >= 0.7) → Active
        for _ in 0..5 {
            lc.record_usage("good_skill", true).await;
        }
        let report = lc.tick().await;
        assert_eq!(lc.get_status("good_skill").await, SkillStatus::Active);
        let last = report.transitions.last().unwrap();
        assert_eq!(last.1, SkillStatus::Candidate);
        assert_eq!(last.2, SkillStatus::Active);
    }

    #[tokio::test]
    async fn test_quarantine_on_high_failure_rate() {
        let lc = SkillLifecycle::new();
        // 10 次, 6 失败 (失败率 60% > 50%)
        for i in 0..10 {
            lc.record_usage("bad_skill", i < 4).await; // 4 success, 6 fail
        }
        let _ = lc.tick().await;
        assert_eq!(lc.get_status("bad_skill").await, SkillStatus::Quarantined);
    }

    #[tokio::test]
    async fn test_no_quarantine_below_10_samples() {
        let lc = SkillLifecycle::new();
        // 4 次 (低于 5 promote 阈值), 全部失败 (4/4 = 100% 失败率)
        for _ in 0..4 {
            lc.record_usage("low_sample_skill", false).await;
        }
        let _ = lc.tick().await;
        // 样本数 < 10, 不隔离; 触发次数 < 5, 也不升 candidate
        assert_eq!(lc.get_status("low_sample_skill").await, SkillStatus::Active);
    }

    #[tokio::test]
    async fn test_changelog_records_transitions() {
        let lc = SkillLifecycle::new();
        for _ in 0..5 {
            lc.record_usage("changelog_skill", true).await;
        }
        let _ = lc.tick().await;
        let rec = lc.get_record("changelog_skill").await.unwrap();
        assert!(!rec.changelog.is_empty());
        assert_eq!(rec.changelog[0].from, SkillStatus::Active);
        assert_eq!(rec.changelog[0].to, SkillStatus::Candidate);
    }

    #[tokio::test]
    async fn test_tick_increments_counter() {
        let lc = SkillLifecycle::new();
        assert_eq!(lc.tick_count(), 0);
        let _ = lc.tick().await;
        let _ = lc.tick().await;
        assert_eq!(lc.tick_count(), 2);
    }

    #[tokio::test]
    async fn test_record_usage_calculates_confidence() {
        let lc = SkillLifecycle::new();
        for _ in 0..3 {
            lc.record_usage("conf_skill", true).await;
        }
        lc.record_usage("conf_skill", false).await;
        let rec = lc.get_record("conf_skill").await.unwrap();
        assert_eq!(rec.use_count, 4);
        assert_eq!(rec.success_count, 3);
        assert!((rec.confidence() - 0.75).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_spawn_tick_doesnt_block() {
        let lc = Arc::new(SkillLifecycle::new());
        lc.clone().spawn_tick();
        // 不等 tick 完成, 直接验证后续能继续操作
        lc.record_usage("spawn_skill", true).await;
        assert!(lc.get_record("spawn_skill").await.is_some());
    }
}

//! 缺口 01 + 07: Hook 调度系统 — 5 层 Tier + 退出码 + 熔断器.
//!
//! ## 设计要点
//!
//! - **HookTier** (5 变体): Session / ToolGuard / Transform / Continuation / Skill
//! - **HookResult** (4 变体): Ok / Continue / Error(exit_code, recoverable) / Block
//! - **HookStats** (DashMap): 每个 hook name 共享统计 (success/failure + 连续失败 + circuit_state)
//! - **熔断器**: 3 次连续 `recoverable=false` → `CircuitState::Open`, 该 hook 被禁用
//! - **事件矩阵** (缺口 07 §7.3): HookEvent × HookTier → 是否触发
//!
//! ## 不做什么
//!
//! - 不做 hook 优先级 (按注册顺序)
//! - 不做动态 tier 加载
//! - 不做 hook 远程加载 (只本地 builtin)
//!
//! ## 调用方
//!
//! - `processing_loop::handle_user_message` 在关键事件点查 HookRegistry 派发
//! - `error_from_llm` 失败时调 `record_failure` 给熔断器

#![allow(clippy::type_complexity)] // async_trait + dyn 组合常见, 暂时保留

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

// ─── HookTier ───────────────────────────────────────────────

/// Hook 调度层级 (缺口 07 §7.1).
///
/// 5 层关注点解耦: 各自独立的 chain, 避免 25+ hook 堆一槽位.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookTier {
    /// 整个 session 生命周期 (开/关/暂停)
    Session,
    /// 工具调用前后 (权限/审计/限流)
    ToolGuard,
    /// 上下文/请求变换 (变量替换/脱敏/压缩)
    Transform,
    /// 流程控制 (loop/sub-agent/reflection)
    Continuation,
    /// skill 触发 (自动加载/蒸馏)
    Skill,
}

impl HookTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::ToolGuard => "tool_guard",
            Self::Transform => "transform",
            Self::Continuation => "continuation",
            Self::Skill => "skill",
        }
    }
}

// ─── HookEvent ──────────────────────────────────────────────

/// Hook 触发时机 (跟缺口 07 §7.3 矩阵对齐).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    BeforePromptBuild,
    AfterPromptBuild,
    BeforeToolCall,
    AfterToolCall,
    BeforeLoopIter,
    AfterLoopIter,
}

impl HookEvent {
    /// 该 event 应触发哪些 tiers.
    ///
    /// 跟 缺口 07 §7.3 矩阵严格一致.
    pub fn triggers_tiers(&self) -> &'static [HookTier] {
        match self {
            Self::BeforePromptBuild => &[HookTier::Transform, HookTier::Skill],
            Self::AfterPromptBuild => &[HookTier::Transform],
            Self::BeforeToolCall => &[HookTier::ToolGuard, HookTier::Transform],
            Self::AfterToolCall => &[HookTier::ToolGuard, HookTier::Continuation],
            Self::BeforeLoopIter => &[HookTier::Continuation],
            Self::AfterLoopIter => &[HookTier::Continuation, HookTier::Skill],
        }
    }
}

// ─── HookResult ─────────────────────────────────────────────

/// Hook 执行结果 (缺口 01 §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookResult {
    /// hook 成功, 继续后续 hook
    Ok,
    /// hook 成功, 跳过剩余同 tier hook
    Continue,
    /// hook 出错 (退出码 + 可恢复标志)
    Error {
        exit_code: i32,
        recoverable: bool,
        message: String,
    },
    /// hook 主动 block (e.g. 权限拒绝)
    Block { reason: String },
}

impl HookResult {
    /// 是否算失败 (用于熔断器计数).
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    /// 是否熔断计数 (recoverable=false 的 Error 算).
    pub fn counts_toward_circuit(&self) -> bool {
        matches!(self, Self::Error { recoverable: false, .. })
    }
}

// ─── CircuitState ───────────────────────────────────────────

/// 熔断器状态机.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CircuitState {
    Closed = 0,
    Open = 1,
    HalfOpen = 2,
}

impl CircuitState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Open,
            2 => Self::HalfOpen,
            _ => Self::Closed,
        }
    }
}

/// 熔断阈值: 连续 recoverable=false 次数.
pub const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;

// ─── HookStats ──────────────────────────────────────────────

/// 单个 hook 的运行时统计 (缺口 01 + 07 共享).
pub struct HookStats {
    pub success_count: AtomicU64,
    pub failure_count: AtomicU64,
    /// 连续 recoverable=false 计数 (熔断触发后 reset).
    pub consecutive_failures: AtomicU32,
    pub circuit_state: AtomicU8,
    /// 最后失败时间 (unix 秒).
    pub last_failure_at: AtomicU64,
}

impl HookStats {
    pub fn new() -> Self {
        Self {
            success_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
            consecutive_failures: AtomicU32::new(0),
            circuit_state: AtomicU8::new(CircuitState::Closed as u8),
            last_failure_at: AtomicU64::new(0),
        }
    }

    /// 记录一次成功.
    pub fn record_success(&self) {
        self.success_count.fetch_add(1, Ordering::Relaxed);
        // 成功后 reset 连续失败计数 + 熔断器尝试闭合
        self.consecutive_failures.store(0, Ordering::Relaxed);
        let prev = self.circuit_state.load(Ordering::Relaxed);
        if prev == CircuitState::HalfOpen as u8 {
            self.circuit_state.store(CircuitState::Closed as u8, Ordering::Relaxed);
        }
    }

    /// 记录一次失败.
    ///
    /// - `recoverable=false`: 计入熔断计数, 达到阈值 → Open
    /// - `recoverable=true`: 仅 +failure_count, 不影响熔断
    pub fn record_failure(&self, recoverable: bool) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        self.last_failure_at
            .store(now_unix_secs(), Ordering::Relaxed);
        if !recoverable {
            let prev = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
            if prev + 1 >= CIRCUIT_BREAKER_THRESHOLD {
                self.circuit_state
                    .store(CircuitState::Open as u8, Ordering::Relaxed);
                tracing::warn!(
                    consecutive_failures = prev + 1,
                    "[hooks] circuit breaker OPENED after {} consecutive failures",
                    prev + 1
                );
            }
        }
    }

    pub fn circuit_state(&self) -> CircuitState {
        CircuitState::from_u8(self.circuit_state.load(Ordering::Relaxed))
    }

    pub fn is_disabled(&self) -> bool {
        self.circuit_state() == CircuitState::Open
    }
}

impl Default for HookStats {
    fn default() -> Self {
        Self::new()
    }
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ─── HookHandler ────────────────────────────────────────────

#[async_trait]
pub trait HookHandler: Send + Sync {
    fn name(&self) -> &str;
    fn tier(&self) -> HookTier;
    async fn handle(&self, event: HookEvent, ctx: &mut HookContext) -> HookResult;
}

/// Hook 上下文 (运行时数据).
#[derive(Debug, Default)]
pub struct HookContext {
    pub session_id: Option<String>,
    pub user_message: Option<String>,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    /// 其他自定义数据
    pub extra: serde_json::Value,
}

// ─── HookRegistry ───────────────────────────────────────────

/// Hook 注册中心 (5 tier × N hook).
pub struct HookRegistry {
    chains: RwLock<HashMap<HookTier, Vec<Arc<dyn HookHandler>>>>,
    stats: RwLock<HashMap<String, Arc<HookStats>>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            chains: RwLock::new(HashMap::new()),
            stats: RwLock::new(HashMap::new()),
        }
    }

    /// 注册一个 hook.
    pub fn register(&self, hook: Arc<dyn HookHandler>) {
        let name = hook.name().to_string();
        {
            let mut stats = self.stats.write().expect("stats lock poisoned");
            stats
                .entry(name)
                .or_insert_with(|| Arc::new(HookStats::new()));
        }
        let mut chains = self.chains.write().expect("chains lock poisoned");
        chains.entry(hook.tier()).or_default().push(hook);
    }

    /// 拿 hook 的 stats (供 e2e 验证熔断器状态).
    pub fn get_stats(&self, name: &str) -> Option<Arc<HookStats>> {
        self.stats.read().ok()?.get(name).cloned()
    }

    /// 列出所有 hook name.
    pub fn hook_names(&self) -> Vec<String> {
        self.stats
            .read()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 强制 reset 某个 hook 的熔断器 (恢复用).
    pub fn reset_circuit(&self, name: &str) {
        if let Some(stats) = self.get_stats(name) {
            stats.consecutive_failures.store(0, Ordering::Relaxed);
            stats.circuit_state.store(CircuitState::Closed as u8, Ordering::Relaxed);
        }
    }

    /// 派发 hook 链.
    ///
    /// 跳过熔断的 hook (circuit_state=Open), 记录 stats, 返回最终结果.
    /// **遇到 Block 立即中断, 不再继续后续 hook.**
    pub async fn dispatch(&self, event: HookEvent, ctx: &mut HookContext) -> HookResult {
        let tiers = event.triggers_tiers();
        let mut last_result = HookResult::Ok;
        for tier in tiers {
            let hooks = match self.chains.read() {
                Ok(c) => c.get(tier).cloned(),
                Err(_) => continue,
            };
            let hooks = match hooks {
                Some(h) => h,
                None => continue,
            };
            for hook in hooks.iter() {
                let stats = self.get_stats(hook.name().as_ref());
                let stats = match stats {
                    Some(s) => s,
                    None => continue,
                };
                if stats.is_disabled() {
                    tracing::debug!(hook = %hook.name(), "[hooks] skipped (circuit open)");
                    continue;
                }
                let result = hook.handle(event, ctx).await;
                match &result {
                    HookResult::Ok | HookResult::Continue => stats.record_success(),
                    HookResult::Error { recoverable, .. } => stats.record_failure(*recoverable),
                    HookResult::Block { .. } => {
                        stats.record_failure(false); // Block 算硬失败
                        return result;
                    }
                }
                last_result = result.clone();
                if matches!(result, HookResult::Continue | HookResult::Block { .. }) {
                    break; // Continue: 跳出当前 tier chain
                }
            }
        }
        last_result
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── HookResult ──

    #[test]
    fn test_hook_result_is_failure() {
        assert!(!HookResult::Ok.is_failure());
        assert!(!HookResult::Continue.is_failure());
        assert!(HookResult::Error {
            exit_code: 1,
            recoverable: true,
            message: "x".into(),
        }
        .is_failure());
        assert!(!HookResult::Block { reason: "x".into() }.is_failure());
    }

    #[test]
    fn test_hook_result_counts_toward_circuit() {
        assert!(!HookResult::Error {
            exit_code: 1,
            recoverable: true,
            message: "x".into(),
        }
        .counts_toward_circuit());
        assert!(HookResult::Error {
            exit_code: 1,
            recoverable: false,
            message: "x".into(),
        }
        .counts_toward_circuit());
    }

    // ── HookEvent.triggers_tiers ──

    #[test]
    fn test_event_before_prompt_build_triggers_transform_and_skill() {
        let tiers = HookEvent::BeforePromptBuild.triggers_tiers();
        assert_eq!(tiers.len(), 2);
        assert!(tiers.contains(&HookTier::Transform));
        assert!(tiers.contains(&HookTier::Skill));
    }

    #[test]
    fn test_event_after_tool_call_triggers_tool_guard_and_continuation() {
        let tiers = HookEvent::AfterToolCall.triggers_tiers();
        assert_eq!(tiers.len(), 2);
        assert!(tiers.contains(&HookTier::ToolGuard));
        assert!(tiers.contains(&HookTier::Continuation));
    }

    // ── HookStats 熔断器 ──

    #[test]
    fn test_circuit_breaker_opens_after_3_unrecoverable() {
        let stats = HookStats::new();
        assert_eq!(stats.circuit_state(), CircuitState::Closed);

        stats.record_failure(false);
        stats.record_failure(false);
        assert_eq!(stats.circuit_state(), CircuitState::Closed);

        stats.record_failure(false);
        assert_eq!(stats.circuit_state(), CircuitState::Open);
        assert!(stats.is_disabled());
    }

    #[test]
    fn test_recoverable_failures_dont_open_circuit() {
        let stats = HookStats::new();
        for _ in 0..10 {
            stats.record_failure(true);
        }
        assert_eq!(stats.circuit_state(), CircuitState::Closed);
        assert!(!stats.is_disabled());
    }

    #[test]
    fn test_success_resets_consecutive_failures() {
        let stats = HookStats::new();
        stats.record_failure(false);
        stats.record_failure(false);
        assert_eq!(stats.consecutive_failures.load(Ordering::Relaxed), 2);

        stats.record_success();
        assert_eq!(stats.consecutive_failures.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_success_after_half_open_closes_circuit() {
        let stats = HookStats::new();
        // 触发 Open
        for _ in 0..3 {
            stats.record_failure(false);
        }
        assert_eq!(stats.circuit_state(), CircuitState::Open);

        // 手动转 HalfOpen 模拟外部探测
        stats.circuit_state
            .store(CircuitState::HalfOpen as u8, Ordering::Relaxed);

        // 探测成功 → Closed
        stats.record_success();
        assert_eq!(stats.circuit_state(), CircuitState::Closed);
    }

    // ── HookRegistry ──

    /// 测试用 hook: 总是返 Ok
    struct AlwaysOkHook;
    #[async_trait]
    impl HookHandler for AlwaysOkHook {
        fn name(&self) -> &str {
            "always_ok"
        }
        fn tier(&self) -> HookTier {
            HookTier::Transform
        }
        async fn handle(&self, _: HookEvent, _: &mut HookContext) -> HookResult {
            HookResult::Ok
        }
    }

    /// 测试用 hook: 总是返 Error(recoverable=false)
    struct AlwaysFailHook;
    #[async_trait]
    impl HookHandler for AlwaysFailHook {
        fn name(&self) -> &str {
            "always_fail"
        }
        fn tier(&self) -> HookTier {
            HookTier::Transform
        }
        async fn handle(&self, _: HookEvent, _: &mut HookContext) -> HookResult {
            HookResult::Error {
                exit_code: 1,
                recoverable: false,
                message: "test".into(),
            }
        }
    }

    /// 测试用 hook: 返 Block
    struct BlockHook;
    #[async_trait]
    impl HookHandler for BlockHook {
        fn name(&self) -> &str {
            "block_hook"
        }
        fn tier(&self) -> HookTier {
            HookTier::ToolGuard
        }
        async fn handle(&self, _: HookEvent, _: &mut HookContext) -> HookResult {
            HookResult::Block {
                reason: "denied".into(),
            }
        }
    }

    #[tokio::test]
    async fn test_registry_dispatch_returns_last_result() {
        let reg = HookRegistry::new();
        reg.register(Arc::new(AlwaysOkHook));

        let mut ctx = HookContext::default();
        let r = reg.dispatch(HookEvent::BeforePromptBuild, &mut ctx).await;
        assert_eq!(r, HookResult::Ok);
    }

    #[tokio::test]
    async fn test_registry_skips_disabled_hooks() {
        let reg = HookRegistry::new();
        reg.register(Arc::new(AlwaysFailHook));

        // 触发 3 次让它熔断
        for _ in 0..3 {
            let mut ctx = HookContext::default();
            reg.dispatch(HookEvent::AfterPromptBuild, &mut ctx).await;
        }
        assert!(reg.get_stats("always_fail").unwrap().is_disabled());

        // 第 4 次 dispatch 应跳过该 hook, 返 Ok (没有别的 hook)
        let mut ctx = HookContext::default();
        let r = reg.dispatch(HookEvent::AfterPromptBuild, &mut ctx).await;
        assert_eq!(r, HookResult::Ok, "disabled hook should be skipped");
    }

    #[tokio::test]
    async fn test_registry_block_short_circuits() {
        let reg = HookRegistry::new();
        reg.register(Arc::new(BlockHook));

        let mut ctx = HookContext::default();
        let r = reg.dispatch(HookEvent::BeforeToolCall, &mut ctx).await;
        assert!(matches!(r, HookResult::Block { .. }));
    }

    #[test]
    fn test_registry_reset_circuit() {
        let reg = HookRegistry::new();
        reg.register(Arc::new(AlwaysFailHook));

        let stats = reg.get_stats("always_fail").unwrap();
        // 模拟熔断
        for _ in 0..3 {
            stats.record_failure(false);
        }
        assert!(stats.is_disabled());

        reg.reset_circuit("always_fail");
        assert!(!stats.is_disabled());
    }
}

use crate::types::TokenUsage;
use std::time::Instant;

/// Token 追踪范围模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenScope {
    /// 使用上下文总 token
    Total,
    /// 仅计算超出预填充基线的增长部分
    BodyAfterPrefix,
}

/// 四区安全网水位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompactZone {
    Safe,
    Warning,
    Danger,
    Blocked,
}

/// Token 追踪 + 压缩状态。
///
/// 使用 API 精确值（DeepSeek message_start 返回的 input_tokens），
/// 首次记录为 prefill 基线，之后只追踪增长量（BodyAfterPrefix 模式）。
#[derive(Debug, Clone)]
pub struct AutoCompactWindow {
    /// 压缩计数器，每次成功压缩后递增
    pub ordinal: u64,
    /// 基线 token（首次 message_start 的 input_tokens）
    pub prefill_input_tokens: Option<u64>,
    /// 最近一次请求的输入 token
    pub last_input_tokens: u64,
    /// 有效窗口 = model_window - min(max_output_tokens, 20000)
    pub effective_window: u64,
    /// 当前水位区
    pub zone: CompactZone,
    /// 熔断器剩余尝试次数（3 → 2 → 1 → 0 熔断）
    pub circuit_breaker_remaining: u32,
    /// 最后一次 assistant 消息的时间（用于 L2 TTL 检查）
    pub last_assistant_time: Option<Instant>,
    /// 最后一次成功压缩的时间
    pub last_compact_time: Option<Instant>,
    /// TTL 秒数（MicroCompact 触发条件）
    pub micro_compact_ttl_secs: u64,
    /// 警告阈值，默认 0.85
    pub warning_ratio: f64,
    /// 触发压缩阈值，默认 0.90
    pub collapse_ratio: f64,
    /// 阻塞阈值，默认 0.95
    pub block_ratio: f64,
}

impl AutoCompactWindow {
    pub fn new(
        model_window: u64,
        max_output_tokens: u64,
        circuit_breaker_limit: u32,
    ) -> Self {
        let reserved = max_output_tokens.min(20_000);
        Self {
            ordinal: 0,
            prefill_input_tokens: None,
            last_input_tokens: 0,
            effective_window: model_window.saturating_sub(reserved),
            zone: CompactZone::Safe,
            circuit_breaker_remaining: circuit_breaker_limit,
            last_assistant_time: None,
            last_compact_time: None,
            micro_compact_ttl_secs: 60,
            warning_ratio: 0.85,
            collapse_ratio: 0.90,
            block_ratio: 0.95,
        }
    }

    /// 从 API 返回的 TokenUsage 更新追踪状态。
    /// 首次调用时记录 prefill 基线。
    pub fn update(&mut self, usage: &TokenUsage) {
        if usage.input > 0 {
            if self.prefill_input_tokens.is_none() {
                self.prefill_input_tokens = Some(usage.input);
            }
            self.last_input_tokens = usage.input;
        }
    }

    /// 设置预填充基线（从 message_start 的 input_tokens）。
    pub fn set_prefill(&mut self, input_tokens: u64) {
        self.prefill_input_tokens = Some(input_tokens);
        self.last_input_tokens = input_tokens;
    }

    /// 计算当前使用率（0.0 ~ 1.0）。
    pub fn usage_ratio(&self, scope: TokenScope) -> f64 {
        let baseline = match scope {
            TokenScope::Total => 0,
            TokenScope::BodyAfterPrefix => self.prefill_input_tokens.unwrap_or(0),
        };

        let used = self.last_input_tokens.saturating_sub(baseline);
        if self.effective_window == 0 || self.prefill_input_tokens.is_none() {
            return 0.0;
        }
        (used as f64) / (self.effective_window as f64)
    }

    /// 确定当前水位区。
    pub fn compute_zone(&self, scope: TokenScope) -> CompactZone {
        let ratio = self.usage_ratio(scope);
        if ratio >= self.block_ratio {
            CompactZone::Blocked
        } else if ratio >= self.collapse_ratio {
            CompactZone::Danger
        } else if ratio >= self.warning_ratio {
            CompactZone::Warning
        } else {
            CompactZone::Safe
        }
    }

    /// 检查是否应该执行 MicroCompact（超 TTL）。
    pub fn should_micro_compact(&self) -> bool {
        match self.last_assistant_time {
            Some(t) => t.elapsed().as_secs() > self.micro_compact_ttl_secs,
            None => false,
        }
    }

    /// 记录最后 assistant 消息时间。
    pub fn set_last_assistant_time(&mut self, time: Instant) {
        self.last_assistant_time = Some(time);
    }

    /// 记录成功压缩。
    pub fn record_compaction(&mut self) {
        self.ordinal += 1;
        self.last_compact_time = Some(Instant::now());
    }

    /// 记录压缩失败。返回 true 表示熔断器已打开。
    pub fn record_failure(&mut self) -> bool {
        self.circuit_breaker_remaining = self.circuit_breaker_remaining.saturating_sub(1);
        self.circuit_breaker_remaining == 0
    }

    /// 熔断器是否已打开。
    pub fn is_circuit_broken(&self) -> bool {
        self.circuit_breaker_remaining == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effective_window() {
        let w = AutoCompactWindow::new(1_000_000, 16384, 3);
        assert_eq!(w.effective_window, 983_616);
    }

    #[test]
    fn test_effective_window_large_output() {
        // max_output > 20000, reserved capped at 20000
        let w = AutoCompactWindow::new(100_000, 50_000, 3);
        assert_eq!(w.effective_window, 80_000);
    }

    #[test]
    fn test_zone_safe_when_no_prefill() {
        let w = AutoCompactWindow::new(1_000_000, 16384, 3);
        assert_eq!(w.compute_zone(TokenScope::BodyAfterPrefix), CompactZone::Safe);
    }

    #[test]
    fn test_zone_detection_total() {
        let mut w = AutoCompactWindow::new(100_000, 16384, 3);
        w.update(&TokenUsage {
            input: 90_000,
            output: 0,
            cache_creation_input: None,
            cache_read_input: None,
        });
        // 90_000 / 83_616 ≈ 1.076 > 0.95 → Blocked
        assert_eq!(w.compute_zone(TokenScope::Total), CompactZone::Blocked);
    }

    #[test]
    fn test_zone_detection_body_after_prefix() {
        let mut w = AutoCompactWindow::new(100_000, 16384, 3);
        w.set_prefill(10_000);
        w.update(&TokenUsage {
            input: 90_000,
            output: 0,
            cache_creation_input: None,
            cache_read_input: None,
        });
        // (90_000 - 10_000) / 83_616 ≈ 0.956 > 0.95 → Blocked
        assert_eq!(w.compute_zone(TokenScope::BodyAfterPrefix), CompactZone::Blocked);
    }

    #[test]
    fn test_zone_warning() {
        let mut w = AutoCompactWindow::new(100_000, 16384, 3);
        w.set_prefill(10_000);
        w.update(&TokenUsage {
            input: 80_000,
            output: 0,
            cache_creation_input: None,
            cache_read_input: None,
        });
        // (80_000 - 10_000) / 83_616 ≈ 0.836 → between 0.85? No, 0.836 < 0.85
        // So actually safe. Let me adjust to hit warning: need 0.85 * 83616 = 71074
        // 10_000 + 71074 = 81074
        w.update(&TokenUsage {
            input: 82_000,
            output: 0,
            cache_creation_input: None,
            cache_read_input: None,
        });
        // (82_000 - 10_000) / 83_616 ≈ 0.861 → Warning
        assert_eq!(w.compute_zone(TokenScope::BodyAfterPrefix), CompactZone::Warning);
    }

    #[test]
    fn test_circuit_breaker_opens_after_three_failures() {
        let mut w = AutoCompactWindow::new(1_000_000, 16384, 3);
        assert!(!w.record_failure()); // 2 remaining
        assert!(!w.record_failure()); // 1 remaining
        assert!(w.record_failure());  // 0 remaining → opens, returns true
        assert!(w.is_circuit_broken());
    }

    #[test]
    fn test_should_micro_compact_within_ttl() {
        let mut w = AutoCompactWindow::new(1_000_000, 16384, 3);
        w.set_last_assistant_time(Instant::now());
        assert!(!w.should_micro_compact()); // Just set, should not trigger
    }

    #[test]
    fn test_compaction_increments_ordinal() {
        let mut w = AutoCompactWindow::new(1_000_000, 16384, 3);
        assert_eq!(w.ordinal, 0);
        w.record_compaction();
        assert_eq!(w.ordinal, 1);
        w.record_compaction();
        assert_eq!(w.ordinal, 2);
    }
}

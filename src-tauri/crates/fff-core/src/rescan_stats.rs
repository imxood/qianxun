/// Rescan request accounting (disabled by default for faster compilation).
/// When disabled, all counters are zero and no atomic operations are performed.

/// Whether rescan accounting is compiled in.
#[allow(dead_code)]
pub const RESCAN_STATS_ENABLED: bool = false;

/// Simplified rescan stats snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub struct RescanStats {
    /// Total rescan count (always 0 when disabled).
    pub total: usize,
    /// Requests suppressed during cooldown (always 0 when disabled).
    pub throttled: usize,
}

/// Rescan counters (no-op when rescan_stats is not enabled).
#[derive(Default)]
#[allow(dead_code)]
pub(crate) struct RescanCounters {
    _phantom: std::marker::PhantomData<()>,
}

#[allow(dead_code)]
impl RescanCounters {
    pub(crate) fn record(&self, _reason: &str) {}

    pub(crate) fn record_throttled(&self, _reason: &str) {}

    pub(crate) fn snapshot(&self) -> RescanStats {
        RescanStats::default()
    }

    pub(crate) fn reset(&self) {}
}

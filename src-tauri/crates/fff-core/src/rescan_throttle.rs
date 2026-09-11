use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const NEVER: u64 = u64::MAX;

/// Drops watcher rescan requests inside the cooldown after the last scan.
/// A slightly stale index is fine: the next admitted event rescans everything.
#[allow(dead_code)]
pub(crate) struct RescanThrottle {
    epoch: Instant,
    last_admitted: AtomicU64,
}

impl Default for RescanThrottle {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            last_admitted: AtomicU64::new(NEVER),
        }
    }
}

impl RescanThrottle {
    /// Returns `true` if a rescan may start now and records it as the last scan.
    #[allow(dead_code)]
    pub(crate) fn admit(&self) -> bool {
        const MIN_INTERVAL_MS: u64 = 30_000; // 30s cooldown

        let now = self.elapsed_ms();

        loop {
            let last = self.last_admitted.load(Ordering::Acquire);
            if last != NEVER && now.saturating_sub(last) < MIN_INTERVAL_MS {
                return false;
            }
            // CAS so two concurrent requests cannot both start a walk.
            if self
                .last_admitted
                .compare_exchange(last, now, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Records an explicit (unthrottled) scan so watcher requests right after
    /// it are dropped: the index is already fresh.
    #[allow(dead_code)]
    pub(crate) fn note_explicit_scan(&self) {
        self.last_admitted
            .store(self.elapsed_ms(), Ordering::Release);
    }

    #[allow(dead_code)]
    fn elapsed_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

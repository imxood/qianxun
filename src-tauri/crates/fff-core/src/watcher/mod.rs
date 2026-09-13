mod background_watcher;
pub use background_watcher::*;

mod watch;
pub use watch::*;

// 注：上游的 rescan_tests（#[cfg(all(test, rescan_stats))]）随 rescan-stats
// 统计子系统一起在定制时移除；rescan_stats.rs 孤儿文件亦已删除（收编清理）。

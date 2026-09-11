//! Dedicated rayon pools for file search operations.

use std::sync::LazyLock;

/// Dedicated thread pool for background work (scan, walk).
pub static BACKGROUND_THREAD_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    let total = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);

    // Background work is mostly syscall-bound; halving parallelism leaves
    // cores for search/UI at negligible throughput cost.
    let bg_threads = (total / 2).max(2);
    rayon::ThreadPoolBuilder::new()
        .num_threads(bg_threads)
        .thread_name(|i| format!("fff-bg-{i}"))
        .build()
        .expect("failed to create background rayon pool")
});

/// Pool for grep content search: full parallelism.
pub static SEARCH_THREAD_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    let threads = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);

    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("fff-search-{i}"))
        .build()
        .expect("failed to create search rayon pool")
});

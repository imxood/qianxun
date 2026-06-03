// buf_writer: 部分方法 (push/is_empty) 和 Style 暂未用, 留 Phase 4.
#![allow(dead_code)]

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// 带超时刷盘的缓冲写入器。
///
/// 数据先写入内部 4KB buffer，满 capacity 时自动落盘。
/// 后台线程每 `flush_interval` 检查一次，如有缓存数据则写入文件。
///
/// 相比 `std::io::BufWriter`，在低频率写入场景下不会让数据长时间滞留在用户态 buffer 中。
pub struct TimedBufWriter<W: Write + Send + 'static> {
    inner: Arc<Mutex<BufInner<W>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    shutdown: Arc<AtomicBool>,
}

struct BufInner<W: Write> {
    file: W,
    buf: Vec<u8>,
    capacity: usize,
}

fn flush_buf<W: Write>(inner: &Mutex<BufInner<W>>) {
    if let Ok(mut guard) = inner.lock() {
        if !guard.buf.is_empty() {
            let cap = guard.capacity;
            let buf = std::mem::replace(&mut guard.buf, Vec::with_capacity(cap));
            let _ = guard.file.write_all(&buf);
            let _ = guard.file.flush();
        }
    }
}

impl<W: Write + Send + 'static> TimedBufWriter<W> {
    pub fn new(file: W, capacity: usize, flush_interval: Duration) -> Self {
        let inner = Arc::new(Mutex::new(BufInner {
            file,
            buf: Vec::with_capacity(capacity),
            capacity,
        }));

        let shutdown = Arc::new(AtomicBool::new(false));

        let bg_inner = inner.clone();
        let bg_shutdown = shutdown.clone();
        let handle = thread::spawn(move || loop {
            if bg_shutdown.load(Ordering::Relaxed) {
                flush_buf(&bg_inner);
                return;
            }
            thread::sleep(flush_interval);
            flush_buf(&bg_inner);
        });

        Self {
            inner,
            handle: Mutex::new(Some(handle)),
            shutdown,
        }
    }
}

impl<'a, W: Write + Send + 'static> tracing_subscriber::fmt::MakeWriter<'a> for TimedBufWriter<W> {
    type Writer = TimedBufHandle<W>;

    fn make_writer(&'a self) -> Self::Writer {
        TimedBufHandle {
            inner: self.inner.clone(),
        }
    }
}

/// `make_writer()` 创建的临时句柄，每次日志事件使用一个。
pub struct TimedBufHandle<W: Write> {
    inner: Arc<Mutex<BufInner<W>>>,
}

impl<W: Write> Write for TimedBufHandle<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut guard = self.inner.lock().unwrap();
        if guard.buf.len() + buf.len() >= guard.capacity {
            if !guard.buf.is_empty() {
                let cap = guard.capacity;
                let old_buf = std::mem::replace(&mut guard.buf, Vec::with_capacity(cap));
                guard.file.write_all(&old_buf)?;
            }
            guard.file.write_all(buf)?;
        } else {
            guard.buf.extend_from_slice(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<W: Write + Send + 'static> Drop for TimedBufWriter<W> {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Ok(mut handle_opt) = self.handle.lock() {
            if let Some(handle) = handle_opt.take() {
                let _ = handle.join();
            }
        }
    }
}

// ─── LogRing: in-memory ring buffer for last N log lines ──────────────
//
// Stage 7b: 用作 `/v1/system/logs?lines=N` endpoint 的数据源. 暂不接
// tracing-subscriber 真正写日志 (留给 Stage 7c 集成 make_writer);
// 当前 daemon 的 stderr 走 tracing 默认 fmt layer, LogRing 主要是
// 给 endpoint 一个可测试的 ring buffer 抽象.
//
// 设计: `Mutex<VecDeque<String>>` 简单实现, capacity 满时弹出最旧.
// 不考虑 lock-free 优化 (tail 调用频率低, 单线程够用).

use std::collections::VecDeque;

/// 默认容量 (最近 1000 行). LogRing::with_capacity 覆盖.
pub const DEFAULT_LOG_RING_CAPACITY: usize = 1000;

/// 内存中的最近 N 行日志环形缓冲.
pub struct LogRing {
    inner: Mutex<VecDeque<String>>,
    capacity: usize,
}

impl LogRing {
    /// 创建容量为 `DEFAULT_LOG_RING_CAPACITY` (1000) 的 ring.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_LOG_RING_CAPACITY)
    }

    /// 创建指定容量的 ring.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// 推入一行日志. 超过 capacity 时弹出最旧行.
    pub fn push(&self, line: impl Into<String>) {
        let line = line.into();
        if let Ok(mut guard) = self.inner.lock() {
            if guard.len() >= self.capacity {
                guard.pop_front();
            }
            guard.push_back(line);
        }
    }

    /// 取最后 `n` 行 (按时间顺序, 最早在前). 若 ring 实际长度 < n, 返全部.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let len = guard.len();
        if n == 0 || len == 0 {
            return Vec::new();
        }
        let take = n.min(len);
        // 拿最后 take 行
        let start = len - take;
        guard.iter().skip(start).cloned().collect()
    }

    /// ring 中当前行数.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// ring 是否为空.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for LogRing {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod log_ring_tests {
    use super::*;

    #[test]
    fn test_tail_empty_returns_empty() {
        let ring = LogRing::new();
        assert!(ring.tail(100).is_empty());
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn test_tail_n_zero_returns_empty() {
        let ring = LogRing::new();
        ring.push("line 1");
        ring.push("line 2");
        assert!(ring.tail(0).is_empty());
    }

    #[test]
    fn test_tail_returns_last_n_in_order() {
        let ring = LogRing::new();
        for i in 0..5 {
            ring.push(format!("line {i}"));
        }
        let tail = ring.tail(3);
        assert_eq!(tail, vec!["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn test_tail_n_larger_than_len_returns_all() {
        let ring = LogRing::new();
        ring.push("a");
        ring.push("b");
        let tail = ring.tail(10);
        assert_eq!(tail, vec!["a", "b"]);
    }

    #[test]
    fn test_capacity_drops_oldest() {
        let ring = LogRing::with_capacity(3);
        for i in 0..5 {
            ring.push(format!("line {i}"));
        }
        assert_eq!(ring.len(), 3, "ring should cap at capacity");
        let tail = ring.tail(10);
        assert_eq!(tail, vec!["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn test_default_capacity_is_1000() {
        let ring = LogRing::new();
        for i in 0..1500 {
            ring.push(format!("line {i}"));
        }
        assert_eq!(ring.len(), DEFAULT_LOG_RING_CAPACITY);
        let tail = ring.tail(5);
        // 1500 行, capacity 1000, 留下 500..=1499
        assert_eq!(tail[0], "line 1495");
        assert_eq!(tail[4], "line 1499");
    }
}

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

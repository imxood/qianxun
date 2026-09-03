//! 极简文件日志：单文件追加 + 大小上限轮转（改名 .old，不双写）。
//!
//! M0 只服务外壳自身（启动、设置变更、panic 证据）；DSH 的输出流是
//! 控制台的职责，不混进这里。日志内容不写 token 等敏感信息（编码规范 §8）。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// 触发轮转的大小上限。取一个「多到不至于常见、少到不会失控」的值。
const MAX_BYTES: u64 = 8 * 1024 * 1024;

struct Sink {
    path: PathBuf,
    file: Option<File>,
}

/// 进程级单例。init 之前的写入静默降级到 stderr——日志永远不该成为
/// 启动失败的原因。
static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();

pub fn init(path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let file = OpenOptions::new().create(true).append(true).open(&path);
    let _ = SINK.set(Mutex::new(Sink {
        path,
        file: file.ok(),
    }));
    install_panic_hook();
}

pub fn log(level: &str, message: &str) {
    let line = format!(
        "{} [{level}] {message}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    eprint!("{line}");
    let Some(sink) = SINK.get() else { return };
    let Ok(mut sink) = sink.lock() else { return };
    if let Some(file) = sink.file.as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
    rotate_if_large(&mut sink);
}

/// 轮转是 best-effort：失败只影响日志，不影响任何调用方。
fn rotate_if_large(sink: &mut Sink) {
    let oversized = fs::metadata(&sink.path)
        .map(|meta| meta.len() >= MAX_BYTES)
        .unwrap_or(false);
    if !oversized {
        return;
    }
    let rotated = sink.path.with_extension("log.old");
    let _ = fs::rename(&sink.path, &rotated);
    sink.file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&sink.path)
        .ok();
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // panic 证据进文件 + stderr；崩溃路径本身不允许再崩。
        log("panic", &info.to_string());
    }));
}

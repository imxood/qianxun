//! 工具管理域（R001）：Moli 无头浏览器的检测与安装，以及 Laya
//! 决策引擎侧车的启动/停止/健康探测。
//!
//! Moli 三源检测（优先级从高到低）：settings 自备路径 → 数据目录受管安装
//! （`<data>/tools/moli/`）→ 系统 PATH。检测即真实拉起 `--version`
//! 探测，不做任何存在性猜测。

pub mod laya;
pub mod moli;
pub mod qwen;

/// `env://log` 事件负载：侧车子进程的一行输出（Laya/QWen 通用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnvProcLogLine {
    /// 侧车标识："laya" | "qwen"。
    pub tool: &'static str,
    /// 输出流："stdout" | "stderr"。
    pub stream: &'static str,
    /// 单行文本（已去行尾换行；非 UTF-8 字节按 lossy 转换）。
    pub line: String,
}

/// 侧车进程 stdout/stderr 中继：piped 读取 → 每行**同时**落日志文件
/// （诊断事实源，UI 不在线时也有据可查）与 `env://log` 事件
/// （环境页右侧日志面板实时消费）。
/// 子进程退出（pipe 关闭）后读到 EOF，线程自然结束，无需显式回收。
pub fn relay_child_output(
    app: &tauri::AppHandle,
    tool: &'static str,
    stream: &'static str,
    pipe: impl std::io::Read + Send + 'static,
    mut file: std::fs::File,
) {
    use std::io::{BufRead, BufReader, Write};
    use tauri::Emitter;
    let app = app.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            match reader.read_until(b'\n', &mut raw) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\r', '\n']);
            let _ = writeln!(file, "{line}");
            let _ = app.emit(
                "env://log",
                EnvProcLogLine {
                    tool,
                    stream,
                    line: line.to_owned(),
                },
            );
        }
    });
}

/// Windows 下 spawn 控制台子进程（laya-server/llama-server 均是）时
/// 抑制新控制台窗口：release 千寻是 GUI 子系统，不加此标志每次启动
/// 都会闪一个黑色终端框（实测踩坑）。debug 千寻本身是控制台子系统，
/// 子进程继承其控制台，本就无窗口。
#[cfg(windows)]
pub fn suppress_console_window(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn suppress_console_window(_command: &mut std::process::Command) {}

//! 终端域 IPC 命令与会话管理。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use portable_pty::{CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::{Error, Result};

/// 事件通道（前端 listen 后按会话 id 过滤）。
const OUTPUT_EVENT: &str = "terminal://output";
const EXIT_EVENT: &str = "terminal://exit";

/// 输出事件负载。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OutputEvent<'a> {
    id: u64,
    data: &'a str,
}

/// 退出事件负载。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ExitEvent {
    id: u64,
    #[serde(rename = "exitCode")]
    exit_code: Option<u32>,
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Mutex<Box<dyn Write + Send>>,
}

#[derive(Default)]
pub struct TerminalState {
    sessions: Mutex<HashMap<u64, Session>>,
    next_id: AtomicU64,
}

/// 会话清单条目（标签条重建用）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: u64,
    pub shell: String,
}

/// 启动一个 PTY 会话，返回 id。输出/退出走事件。
#[tauri::command]
pub fn terminal_spawn(
    app: AppHandle,
    state: State<'_, TerminalState>,
    shell: Option<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<u64> {
    let shell = resolve_shell(shell.as_deref());
    let mut command = CommandBuilder::new(&shell);
    if let Some(dir) = cwd.filter(|text| !text.is_empty()) {
        command.cwd(dir);
    }

    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|cause| Error::Terminal(format!("打开 PTY 失败：{cause}")))?;
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|cause| Error::Terminal(format!("启动 {shell} 失败：{cause}")))?;
    // slave 侧在 spawn 后关闭，master 继续驱动。
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|cause| Error::Terminal(format!("克隆 PTY 读取端失败：{cause}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|cause| Error::Terminal(format!("取得 PTY 写入端失败：{cause}")))?;

    let id = state.next_id.fetch_add(1, Ordering::AcqRel);
    state.sessions.lock().unwrap().insert(
        id,
        Session {
            master: pair.master,
            writer: Mutex::new(writer),
        },
    );

    // 输出泵：PTY → 前端（lossy 解码，终端字节流容错）。
    {
        let app = app.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        let data = String::from_utf8_lossy(&buffer[..size]).into_owned();
                        let _ = app.emit(OUTPUT_EVENT, OutputEvent { id, data: &data });
                    }
                }
            }
        });
    }

    // 退出监视：回收子进程并通知前端（同时清理会话表）。
    {
        let app = app.clone();
        std::thread::spawn(move || {
            let status = child.wait();
            let code = status.as_ref().ok().map(|exit| exit.exit_code());
            // 先通知再清理：drop(master) 在 conpty 上可能阻塞在读端关闭上，
            // emit 前置保证无论收尾多慢，前端都能立刻收尾标签。
            let _ = app.emit(
                EXIT_EVENT,
                ExitEvent {
                    id,
                    exit_code: code,
                },
            );
            // State 借用闭包拥有的 app：块内取、块尾还。
            {
                let state = app.state::<TerminalState>();
                state.sessions.lock().unwrap().remove(&id);
            }
        });
    }

    Ok(id)
}

/// 前端键盘输入 → PTY。
#[tauri::command]
pub fn terminal_write(state: State<'_, TerminalState>, id: u64, data: String) -> Result<()> {
    let sessions = state.sessions.lock().unwrap();
    let Some(session) = sessions.get(&id) else {
        return Ok(()); // 会话已退出：静默（前端也收到 exit 事件收尾）。
    };
    let mut writer = session.writer.lock().unwrap();
    writer
        .write_all(data.as_bytes())
        .map_err(|cause| Error::Terminal(format!("写入 PTY 失败：{cause}")))?;
    writer
        .flush()
        .map_err(|cause| Error::Terminal(format!("flush PTY 失败：{cause}")))?;
    Ok(())
}

/// 视口尺寸变化 → PTY resize。
#[tauri::command]
pub fn terminal_resize(
    state: State<'_, TerminalState>,
    id: u64,
    cols: u16,
    rows: u16,
) -> Result<()> {
    let sessions = state.sessions.lock().unwrap();
    let Some(session) = sessions.get(&id) else {
        return Ok(());
    };
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|cause| Error::Terminal(format!("调整 PTY 尺寸失败：{cause}")))
}

/// 关闭标签：杀会话（exit 事件随后到达前端做 UI 收尾）。
#[tauri::command]
pub fn terminal_kill(state: State<'_, TerminalState>, id: u64) -> Result<()> {
    // drop master 会触发 reader EOF 与子进程收尾（Windows 上 conpty 同步关闭）。
    state.sessions.lock().unwrap().remove(&id);
    Ok(())
}

/// 存活会话清单。
#[tauri::command]
pub fn terminal_list(state: State<'_, TerminalState>) -> Result<Vec<TerminalInfo>> {
    let sessions = state.sessions.lock().unwrap();
    Ok(sessions
        .keys()
        .map(|id| TerminalInfo {
            id: *id,
            shell: String::new(),
        })
        .collect())
}

/// shell 解析：auto = pwsh 优先、powershell 兜底；显式路径直接用。
fn resolve_shell(shell: Option<&str>) -> String {
    match shell {
        Some(path) if !path.is_empty() && path != "auto" => path.to_owned(),
        _ => {
            let pwsh = which_in_path("pwsh.exe");
            if pwsh.is_some() {
                "pwsh.exe".to_owned()
            } else {
                "powershell.exe".to_owned()
            }
        }
    }
}

fn which_in_path(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell解析显式优先auto找pwsh() {
        assert_eq!(resolve_shell(Some("C:/bin/mysh.exe")), "C:/bin/mysh.exe");
        assert_eq!(resolve_shell(Some("auto")), {
            if which_in_path("pwsh.exe").is_some() {
                "pwsh.exe"
            } else {
                "powershell.exe"
            }
        });
        assert_eq!(resolve_shell(Some("")), {
            if which_in_path("pwsh.exe").is_some() {
                "pwsh.exe"
            } else {
                "powershell.exe"
            }
        });
    }

    /// 真实 PTY 链路（conpty）：spawn cmd /c echo → 带超时读回标记。
    /// conpty 的 read 是无限阻塞的，子进程退出后也不 EOF——
    /// 静默超时后 drop master（ClosePseudoConsole）强制唤醒读端。
    #[test]
    #[ignore = "真实 conpty，验收时手跑"]
    fn 真实pty回环() {
        use std::io::Write;
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let mut command = CommandBuilder::new("cmd.exe");
        command.args(["/C", "echo qx-pty-ok"]);
        let mut child = pair.slave.spawn_command(command).expect("spawn");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        // take_writer 只允许一次：writer 保留给 DSR 应答用。
        let writer = pair.master.take_writer().expect("writer");
        let dsr_writer = std::sync::Mutex::new(writer);
        let mut replied_cursor_query = false;

        // 读线程 → channel；主线程 recv_timeout 判静默。
        // conpty 握手会发 ESC[6n（光标位置查询），必须回 DSR 否则 cmd 挂起等应答。
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        let _reader_thread = std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        if sender.send(buffer[..size].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let mut collected = Vec::new();
        let mut found = false;
        while let Ok(chunk) = receiver.recv_timeout(Duration::from_secs(3)) {
            collected.extend_from_slice(&chunk);
            if !replied_cursor_query && collected.windows(4).any(|w| w == b"\x1b[6n") {
                replied_cursor_query = true;
                let _ = dsr_writer.lock().unwrap().write_all(b"\x1b[24;80R");
            }
            if collected.windows(9).any(|window| window == b"qx-pty-ok") {
                found = true;
                break;
            }
        }
        // 收尾：try_wait 轮询（cmd 收到 DSR 后 echo 并退出；万一没退，
        // 10s 后放弃——不无限阻塞测试进程）。不 drop master、不 join 读线程
        //（ClosePseudoConsole 会等 conpty client，client 又等 reader 持有的
        // 管道读端，强收必死锁；句柄随测试进程退出回收）。
        let mut status = None;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(exit) = child.try_wait().expect("try_wait") {
                status = Some(exit);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let text = String::from_utf8_lossy(&collected);
        assert!(found, "输出应包含标记：{text}");
        assert_eq!(status.expect("cmd 应在时限内退出").exit_code(), 0);
        println!(
            "conpty 回环耗时 {:.2?}，输出 {} 字节",
            started.elapsed(),
            collected.len()
        );
    }
}

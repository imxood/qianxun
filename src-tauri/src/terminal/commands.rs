//! 终端域 IPC 命令与会话管理。
//!
//! 会话三线程模型：读泵（PTY → 事件）/ 退出监视（先 emit 再清理）/
//! 前端 xterm 保活。v0.2 修正：spawn 直接返回 shell 名（标签默认标题）、
//! 输出带会话级回放缓冲（消除「监听注册晚于 banner」的竞态）、
//! kill 显式终止子进程（不赌 conpty 关闭传播）。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, OnceLock};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::{Error, Result};

/// 事件通道（前端 listen 后按会话 id 过滤）。
const OUTPUT_EVENT: &str = "terminal://output";
const EXIT_EVENT: &str = "terminal://exit";
/// 回放缓冲上限（字节）：够覆盖 shell 横幅 + 一屏历史，内存代价可忽略。
const REPLAY_CAP: usize = 64 * 1024;

/// pwsh / powershell 的 cwd 跟踪钩子：把默认 prompt 换成「先发 OSC 7
/// （file:// URL 报告当前目录）再渲染原 prompt」。无空格写法，规避
/// CreateProcess 命令行引号问题；profile 加载后运行，用户自定义 prompt
///（如 oh-my-posh）经由 $__qxp 保留。
const PWSH_CWD_HOOK: &str = "$__qxp=$function:prompt;function global:prompt{$d=$PWD.ProviderPath;$u=$d.Replace('\\','/');[Console]::Write(([char]27)+']7;file:///'+$u+([char]7));if($__qxp){&$__qxp}else{'PS '+$d+'> '}}";

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
    /// 显式终止句柄：kill 命令用它，不依赖 conpty 关闭传播。
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// 输出回放缓冲（尾部 64KB）：前端监听就绪后拉取，弥合 spawn→挂载窗口期。
    replay: Mutex<String>,
}

#[derive(Default)]
pub struct TerminalState {
    sessions: Mutex<HashMap<u64, Session>>,
    next_id: AtomicU64,
    /// 会话 id → 固定（PIN）记录 id。会话退出时据此刷新 PIN 回放。
    pins: Mutex<HashMap<u64, u64>>,
    /// PIN 记录 id 计数器：跨重启用毫秒时间戳初始化，避免覆盖旧文件。
    next_pin_id: OnceLock<AtomicU64>,
}

impl TerminalState {
    fn next_pin_id(&self) -> u64 {
        let counter = self
            .next_pin_id
            .get_or_init(|| {
                AtomicU64::new(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(1),
                )
            });
        counter.fetch_add(1, Ordering::AcqRel)
    }
}

/// 会话信息（spawn 的返回值：id + 实际 shell，标签默认标题用）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: u64,
    pub shell: String,
}

/// 一个固定（PIN）终端的元数据（不含回放正文）。
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PinnedTerminal {
    pub pin_id: u64,
    pub title: String,
    pub shell: String,
    pub cwd: Option<String>,
}

/// PIN 记录的完整落盘形态（元数据 + 回放正文）。
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PinnedRecord {
    #[serde(flatten)]
    meta: PinnedTerminal,
    replay: String,
}

/// 启动一个 PTY 会话，返回 id 与解析后的 shell。输出/退出走事件。
#[tauri::command]
pub fn terminal_spawn(
    app: AppHandle,
    state: State<'_, TerminalState>,
    shell: Option<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<TerminalInfo> {
    let shell = resolve_shell(shell.as_deref());
    let mut command = CommandBuilder::new(&shell);
    if let Some(dir) = cwd.filter(|text| !text.is_empty()) {
        command.cwd(dir);
    }
    // pwsh/powershell 注入 prompt 钩子：前端经 OSC 7 跟踪 cwd（PIN 恢复用）。
    let basename = shell
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(basename.as_str(), "pwsh.exe" | "powershell.exe") {
        command.arg("-Command");
        command.arg(PWSH_CWD_HOOK);
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
    let killer = child.clone_killer();

    let id = state.next_id.fetch_add(1, Ordering::AcqRel);
    state
        .sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            id,
            Session {
                master: pair.master,
                writer: Mutex::new(writer),
                killer: Mutex::new(killer),
                replay: Mutex::new(String::new()),
            },
        );

    // 输出泵：PTY → 前端。每块读即发（读是阻塞的，攒批只会徒增延迟）；
    // 同时写入回放缓冲，供监听晚到的前端补齐横幅与提示符。
    {
        let app = app.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            let mut decoder = DecodeState::new();
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        let text = decoder.feed(&buffer[..size]);
                        if text.is_empty() {
                            continue;
                        }
                        let state = app.state::<TerminalState>();
                        if let Some(session) = state
                            .sessions
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get(&id)
                        {
                            let mut replay = session
                                .replay
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner);
                            append_capped(&mut replay, &text, REPLAY_CAP);
                        }
                        let _ = app.emit(OUTPUT_EVENT, OutputEvent { id, data: &text });
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
            {
                let state = app.state::<TerminalState>();
                // PIN 会话退出：用最终回放刷新记录（下次启动恢复到此刻）。
                let pin_id = state
                    .pins
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&id)
                    .copied();
                if let Some(pin_id) = pin_id {
                    let replay = state
                        .sessions
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&id)
                        .map(|session| {
                            session
                                .replay
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .clone()
                        });
                    if let Some(replay) = replay {
                        if let Ok(mut record) = read_pinned_record(&app, pin_id) {
                            record.replay = replay;
                            let _ = write_pinned_record(&app, &record);
                        }
                    }
                    state
                        .pins
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                }
                state
                    .sessions
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
            }
        });
    }

    Ok(TerminalInfo { id, shell })
}

/// 回放：监听就绪的前端拉取 spawn 以来的累计输出（尾部 64KB）。
#[tauri::command]
pub fn terminal_replay(state: State<'_, TerminalState>, id: u64) -> Result<String> {
    let sessions = state
        .sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match sessions.get(&id) {
        Some(session) => Ok(session
            .replay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()),
        None => Ok(String::new()), // 会话已退出：空串，exit 事件负责收尾。
    }
}

/// 前端键盘输入 → PTY。
#[tauri::command]
pub fn terminal_write(state: State<'_, TerminalState>, id: u64, data: String) -> Result<()> {
    let sessions = state
        .sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(session) = sessions.get(&id) else {
        return Ok(()); // 会话已退出：静默（前端也收到 exit 事件收尾）。
    };
    let mut writer = session
        .writer
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
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
    let sessions = state
        .sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
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

/// 关闭标签：出表 + 显式杀子进程（exit 事件随后到达前端做 UI 收尾）。
#[tauri::command]
pub fn terminal_kill(state: State<'_, TerminalState>, id: u64) -> Result<()> {
    // 先出表再杀：kill 的耗时不可控，不持锁。
    let session = state
        .sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    // 会话没了，PIN 关联一并解除（记录文件保留与否由前端语义决定：
    // 活标签关闭=不要了 → 先 unpin 再 kill）。
    state
        .pins
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    if let Some(session) = session {
        let mut killer = session
            .killer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // 杀不动（已退出）不报错——目标状态本就是「进程结束」。
        let _ = killer.kill();
    }
    Ok(())
}

/// 清空终端：前端负责清 xterm 视口与滚动缓冲，这里清回放缓冲，
/// 避免下次挂载/重放把旧内容带回来。
#[tauri::command]
pub fn terminal_clear(state: State<'_, TerminalState>, id: u64) -> Result<()> {
    if let Some(session) = state
        .sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&id)
    {
        session
            .replay
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
    Ok(())
}

// ---- PIN：固定终端，跨启动恢复 cwd 与历史输出 ----

fn pinned_dir(app: &AppHandle) -> Result<PathBuf> {
    let dir = crate::paths::data_dir(app)?.join("terminals");
    std::fs::create_dir_all(&dir).map_err(|cause| {
        Error::Terminal(format!("创建终端数据目录失败：{cause}"))
    })?;
    Ok(dir)
}

fn pinned_path(app: &AppHandle, pin_id: u64) -> Result<PathBuf> {
    Ok(pinned_dir(app)?.join(format!("pin-{pin_id}.json")))
}

fn write_pinned_record(app: &AppHandle, record: &PinnedRecord) -> Result<()> {
    let path = pinned_path(app, record.meta.pin_id)?;
    let json = serde_json::to_string(record)
        .map_err(|cause| Error::Terminal(format!("PIN 记录序列化失败：{cause}")))?;
    crate::atomic::write(&path, json.as_bytes())
        .map_err(|cause| Error::Terminal(format!("写入 PIN 记录失败：{cause}")))
}

fn read_pinned_record(app: &AppHandle, pin_id: u64) -> Result<PinnedRecord> {
    let path = pinned_path(app, pin_id)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|cause| Error::Terminal(format!("读取 PIN 记录失败：{cause}")))?;
    serde_json::from_str(&text)
        .map_err(|cause| Error::Terminal(format!("PIN 记录解析失败：{cause}")))
}

/// 固定会话：把当前回放 + 元数据落盘。已有 PIN 的会话重复调用 = 刷新。
#[tauri::command]
pub fn terminal_pin(
    app: AppHandle,
    state: State<'_, TerminalState>,
    id: u64,
    title: String,
    shell: String,
    cwd: Option<String>,
) -> Result<u64> {
    let replay = {
        let sessions = state
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(session) = sessions.get(&id) else {
            return Err(Error::Terminal(format!("会话不存在或已退出：{id}")));
        };
        // 显式局部守卫：块尾的临时 MutexGuard 会把借用拖到 sessions 之后。
        let guard = session.replay.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    };
    let pin_id = match state
        .pins
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&id)
    {
        Some(existing) => *existing,
        None => state.next_pin_id(),
    };
    write_pinned_record(
        &app,
        &PinnedRecord {
            meta: PinnedTerminal {
                pin_id,
                title,
                shell,
                cwd,
            },
            replay,
        },
    )?;
    state
        .pins
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, pin_id);
    Ok(pin_id)
}

/// 已有 PIN 记录的会话在新进程里续上（启动恢复路径：不改写回放内容）。
#[tauri::command]
pub fn terminal_pin_resume(
    app: AppHandle,
    state: State<'_, TerminalState>,
    id: u64,
    pin_id: u64,
) -> Result<()> {
    read_pinned_record(&app, pin_id)?;
    state
        .pins
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id, pin_id);
    Ok(())
}

/// 取消固定：解除关联并删除记录文件。
#[tauri::command]
pub fn terminal_unpin(app: AppHandle, state: State<'_, TerminalState>, id: u64) -> Result<()> {
    let pin_id = state
        .pins
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    if let Some(pin_id) = pin_id {
        match pinned_path(&app, pin_id) {
            Ok(path) => {
                let _ = std::fs::remove_file(path);
            }
            Err(cause) => return Err(cause),
        }
    }
    Ok(())
}

/// 全部固定记录的元数据（启动恢复用；不含回放正文）。
#[tauri::command]
pub fn terminal_pinned_list(app: AppHandle) -> Result<Vec<PinnedTerminal>> {
    let dir = pinned_dir(&app)?;
    let mut list = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .map_err(|cause| Error::Terminal(format!("读取终端数据目录失败：{cause}")))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(record) = serde_json::from_str::<PinnedRecord>(&text) {
            list.push(record.meta);
        }
    }
    list.sort_by_key(|meta| meta.pin_id);
    Ok(list)
}

/// 某条固定记录的历史回放（启动恢复时写进新会话的初始内容）。
#[tauri::command]
pub fn terminal_pinned_replay(app: AppHandle, pin_id: u64) -> Result<String> {
    Ok(read_pinned_record(&app, pin_id)?.replay)
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

/// 增量 UTF-8 解码：跨块多字节字符保留到下一块，不再产出替换符；
/// 真正非法的字节序列以 U+FFFD 显式呈现（终端字节流容错语义）。
struct DecodeState {
    pending: Vec<u8>,
}

impl DecodeState {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        // 前缀已被 std 验证合法。
                        if let Ok(valid) = std::str::from_utf8(&self.pending[..valid_up_to]) {
                            text.push_str(valid);
                        }
                        self.pending.drain(..valid_up_to);
                    }
                    match error.error_len() {
                        None => break, // 尾部不完整：留给下一块。
                        Some(bad_len) => {
                            text.push('\u{FFFD}');
                            self.pending.drain(..bad_len);
                        }
                    }
                }
            }
        }
        text
    }
}

/// 追加并按字节上限裁掉头部（推进到字符边界，不切半个字）。
fn append_capped(buffer: &mut String, chunk: &str, cap: usize) {
    buffer.push_str(chunk);
    let total = buffer.len();
    if total <= cap {
        return;
    }
    let mut drop_bytes = total - cap;
    while drop_bytes < total && !buffer.is_char_boundary(drop_bytes) {
        drop_bytes += 1;
    }
    buffer.drain(..drop_bytes);
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

    #[test]
    fn 增量解码跨块汉字不产出替换符() {
        let mut decoder = DecodeState::new();
        let bytes = "千寻终端".as_bytes();
        // 在多字节字符中间切开：完整字符即时产出，拆开的留到下一块补齐。
        let (head, tail) = bytes.split_at(5);
        assert_eq!(decoder.feed(head), "千");
        assert_eq!(decoder.feed(tail), "寻终端");
        // 非法序列：显式替换符。
        assert_eq!(decoder.feed(&[0xff, b'a']), "\u{FFFD}a");
    }

    #[test]
    fn 回放缓冲按上限裁头部且不切字符() {
        let mut buffer = String::new();
        append_capped(&mut buffer, "千寻", 8);
        append_capped(&mut buffer, "abcdef", 8);
        // "千寻"=6 字节 + "abcdef"=6 字节 = 12 > 8：裁到字符边界，保留 "abcdef" 与可能的半个字。
        assert!(buffer.len() <= 8);
        assert!(buffer.is_char_boundary(0));
        assert_eq!(buffer, "abcdef");
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

//! mario/ 状态目录的读写命令。IPC 边界不做路径穿越的慈善:
//! 文件名走白名单(裸名或 runs/ 一层子目录),写入走原子替换,
//! 审计日志(evolution.jsonl)走只追加。

use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::error::{Error, Result};
use crate::{atomic, paths};

/// 单文件写入上限:手册/策略/簿记都是小文件,1 MiB 足够且防爆盘。
const MAX_WRITE_BYTES: usize = 1 << 20;
/// 单次追加上限:审计行是单行 JSON,64 KiB 已极宽。
const MAX_APPEND_BYTES: usize = 64 << 10;

fn mario_dir(app: &AppHandle) -> Result<PathBuf> {
    let base = paths::data_dir(app)?;
    let dir = base.join("games").join("mario");
    // 一次性迁移:旧 `mario/` 整体搬进 `games/mario/`(含 runs/ 归档),
    // UI 与闭环无感;rename 失败(跨卷等)退回递归复制。
    if !dir.exists() {
        let legacy = base.join("mario");
        if legacy.is_dir() {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|c| Error::Mario(format!("建目录失败:{c}")))?;
            }
            std::fs::rename(&legacy, &dir)
                .or_else(|_| copy_dir_recursive(&legacy, &dir))
                .map_err(|c| Error::Mario(format!("迁移 games/mario 失败:{c}")))?;
        }
    }
    std::fs::create_dir_all(&dir).map_err(|c| Error::Mario(format!("建目录失败:{c}")))?;
    Ok(dir)
}

/// 递归复制目录(迁移兜底;不跟随符号链接)。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::result::Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// 合法文件段:ASCII 字母数字 + `.-_`,非空,≤128 字符。
fn valid_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg.len() <= 128
        && seg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// 解析 `name` 到 mario/ 下的真实路径。只接受两种形状:
/// `file.ext` 或 `runs/file.ext`(一层子目录,留给逐局复盘归档)。
fn resolve(app: &AppHandle, name: &str) -> Result<PathBuf> {
    let mut parts = name.split('/');
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    if parts.next().is_some() {
        return Err(Error::Mario(format!("路径最多一层子目录:{name}")));
    }
    let dir = mario_dir(app)?;
    match (first, second) {
        ("runs", Some(file)) if valid_segment(file) => {
            let sub = dir.join("runs");
            std::fs::create_dir_all(&sub).map_err(|c| Error::Mario(format!("建目录失败:{c}")))?;
            Ok(sub.join(file))
        }
        (file, None) if valid_segment(file) => Ok(dir.join(file)),
        _ => Err(Error::Mario(format!(
            "非法文件名(仅允许 [A-Za-z0-9._-] 裸名或 runs/ 前缀):{name}"
        ))),
    }
}

/// 读文件;不存在返回 None(与"空内容"区分,前端据此走默认/迁移)。
#[tauri::command]
pub fn mario_state_read(app: AppHandle, name: String) -> Result<Option<String>> {
    let path = resolve(&app, &name)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(c) if c.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(c) => Err(Error::Mario(format!("读取失败:{c}"))),
    }
}

/// 整文件原子替换(tmp + fsync + rename,与笔记模块同一写入纪律)。
#[tauri::command]
pub fn mario_state_write(app: AppHandle, name: String, content: String) -> Result<()> {
    if content.len() > MAX_WRITE_BYTES {
        return Err(Error::Mario(format!(
            "内容超长(上限 {} 字节):{name}",
            MAX_WRITE_BYTES
        )));
    }
    let path = resolve(&app, &name)?;
    atomic::write(&path, content.as_bytes()).map_err(|c| Error::Mario(format!("写入失败:{c}")))
}

/// 只追加一行(自动补换行);evolution.jsonl 审计链专用——不改写历史。
#[tauri::command]
pub fn mario_state_append(app: AppHandle, name: String, line: String) -> Result<()> {
    if line.len() > MAX_APPEND_BYTES {
        return Err(Error::Mario(format!(
            "追加行超长(上限 {} 字节):{name}",
            MAX_APPEND_BYTES
        )));
    }
    let path = resolve(&app, &name)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|c| Error::Mario(format!("追加打开失败:{c}")))?;
    let mut bytes = line.into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    f.write_all(&bytes)
        .and_then(|()| f.sync_all())
        .map_err(|c| Error::Mario(format!("追加失败:{c}")))?;
    Ok(())
}

/// 列出 mario/ 下文件名(不含 runs/ 内容):进化面板/调试面板用。
#[tauri::command]
pub fn mario_state_list(app: AppHandle) -> Result<Vec<String>> {
    let dir = mario_dir(&app)?;
    let mut names: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|c| Error::Mario(format!("列目录失败:{c}")))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

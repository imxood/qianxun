//! 磁盘扫描域：目录占用扫描（下一层子项 + 递归大小）与回收站清理。
//!
//! 设计约束：
//! - 流式扫描由 fff-search 的 `scan_directory` 承担：rayon 并行遍历任意
//!   目录，`cancelled` 原子标志随时停止；扫描发现即时可见，事件按固定
//!   100ms 节流推给前端，避免 IPC 洪泛；
//! - 同步 `disk_scan` / `disk_clean` 保留：单层扫描走 spawn_blocking，
//!   清理走回收站（trash crate）可撤销；
//! - 旧 `disk_scan` 的约定继续有效：符号链接 / junction 不深入（防环），
//!   巨目录子项截断 top N 聚合占位，`size` 恒真。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::AppHandle;

use crate::error::{Error, Result};

/// 子项列表上限：超出部分聚合成「其余」占位，防止巨目录撑爆 IPC。
const CHILDREN_LIMIT: usize = 200;

/// fff 流式扫描条目 → 磁盘页数据形状（字段一一对应，递归转换）。
impl From<fff_search::DiskSpaceEntry> for DiskEntry {
    fn from(value: fff_search::DiskSpaceEntry) -> Self {
        DiskEntry {
            name: value.name,
            path: value.path,
            size: value.size,
            dir: value.is_dir,
            children: value.children.into_iter().map(Into::into).collect(),
        }
    }
}

/// 目录占用条目：磁盘扫描页一个方块的数据形状。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskEntry {
    /// 显示名（目录名 / 文件名 / 占位项文案）。
    pub name: String,
    /// 绝对路径；占位项为空串。
    pub path: String,
    /// 递归占用（字节）；无权限 / 链接记 0。
    pub size: u64,
    /// 是否目录（方块可下钻）。
    pub dir: bool,
    /// 子项（仅下一层，按占用降序）。
    pub children: Vec<DiskEntry>,
}

/// 扫描起点：千寻数据根 + 受管子目录的展示名。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskHome {
    pub root: String,
    /// 子目录名 → 展示名（dsh-runtime → DSH 运行时）。
    pub labels: HashMap<String, String>,
}

/// 磁盘清理页的初始视图：数据根路径 + 友好命名。
#[tauri::command]
pub fn disk_home(app: AppHandle) -> Result<DiskHome> {
    let root = crate::paths::data_dir(&app)?;
    let labels: HashMap<String, String> = [
        ("dsh-runtime", "DSH 运行时"),
        ("dsh-runtime-backup", "安装备份"),
        ("dsh-home", "DSH 数据"),
        ("node", "托管 Node"),
        ("pnpm-tool", "pnpm 工具"),
        ("logs", "日志"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    Ok(DiskHome {
        root: root.display().to_string(),
        labels,
    })
}

/// 扫描一个目录：返回自身递归占用与下一层子项（各自递归大小）。
#[tauri::command]
pub async fn disk_scan(path: String) -> Result<DiskEntry> {
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err(Error::Disk(format!("目录不存在：{}", path.display())));
    }
    tauri::async_runtime::spawn_blocking(move || scan_dir(&path))
        .await
        .map_err(|cause| Error::Disk(format!("扫描没有完成：{cause}")))
}

/// 清理 = 整个目录（或文件）移入回收站。盘根与千寻数据根本身拒绝清理。
#[tauri::command]
pub async fn disk_clean(app: AppHandle, path: String) -> Result<()> {
    let target = PathBuf::from(&path);
    if !target.exists() {
        return Err(Error::Disk(format!("路径不存在：{path}")));
    }
    if target.parent().is_none() {
        return Err(Error::Disk("盘根不能清理".to_owned()));
    }
    let data_root = crate::paths::data_dir(&app)?;
    let root_resolved = std::fs::canonicalize(&data_root).unwrap_or(data_root);
    let same_as_root = std::fs::canonicalize(&target)
        .map(|resolved| resolved == root_resolved)
        .unwrap_or(false);
    if same_as_root {
        return Err(Error::Disk("千寻数据目录不能整体清理".to_owned()));
    }
    tauri::async_runtime::spawn_blocking(move || trash::delete(&target))
        .await
        .map_err(|cause| Error::Disk(format!("清理没有完成：{cause}")))?
        .map_err(|cause| Error::Disk(format!("移入回收站失败：{cause}")))
}

// ---- 流式扫描：fff 并行遍历 + 固定频率事件 ----

/// 流式扫描事件：进度按 100ms 节流推送，结束时给完整树快照。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum DiskScanEvent {
    /// 周期进度（固定 ~100ms 一帧）：当前扫描目标 + 实时计数。
    Progress {
        /// 正在遍历的根目录。
        root: String,
        files: u64,
        dirs: u64,
        bytes: u64,
    },
    /// 扫描结束（完成或被停止）：完整树快照，前端整体替换。
    Done {
        root: String,
        cancelled: bool,
        tree: DiskEntry,
    },
}

/// 进程内流式扫描句柄：换目标 / 停止时置位旧的 `cancelled`，
/// `scan_active` 防止同一时刻两条扫描并存。内层 Arc 化让句柄可以
/// 跨进 spawn_blocking 的 'static 闭包。
#[derive(Default)]
pub struct DiskScanManager {
    current: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

impl DiskScanManager {
    /// 取出（并作废）当前扫描，返回新扫描应使用的取消标志。
    fn rotate(&self) -> Arc<AtomicBool> {
        let mut guard = self.current.lock().unwrap();
        if let Some(old) = guard.take() {
            old.store(true, Ordering::Relaxed);
        }
        let fresh = Arc::new(AtomicBool::new(false));
        *guard = Some(fresh.clone());
        fresh
    }

    /// 克隆内层共享槽（供 spawn_blocking 的 'static 闭包收尾用）。
    fn slot(&self) -> Arc<Mutex<Option<Arc<AtomicBool>>>> {
        self.current.clone()
    }
}

/// 停止当前流式扫描：置位取消标志，walker 在下个条目即退出，
/// 前端照常收到 Done{cancelled:true} 收尾帧。无进行中扫描时空操作。
#[tauri::command]
pub fn disk_scan_stop(manager: tauri::State<'_, DiskScanManager>) -> Result<()> {
    if let Some(current) = manager.current.lock().unwrap().as_ref() {
        current.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// 流式扫描任意目录：fff 并行遍历，事件走 Channel 推送。
/// 用户换目标 / 点停止时旧的扫描会被静默作废（事件流以新扫描为准）。
#[tauri::command]
pub async fn disk_scan_stream(
    manager: tauri::State<'_, DiskScanManager>,
    path: String,
    on_event: Channel<DiskScanEvent>,
) -> Result<()> {
    let target = PathBuf::from(&path);
    if !target.is_dir() {
        return Err(Error::Disk(format!("目录不存在：{}", path)));
    }
    let cancelled = manager.rotate();
    let slot = manager.slot();

    tauri::async_runtime::spawn_blocking(move || {
        let root_display = target.display().to_string();
        let mut last_tick = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_millis(100))
            .unwrap_or_else(std::time::Instant::now);
        let result = fff_search::scan_directory(&target, &cancelled, |files, dirs, bytes| {
            let now = std::time::Instant::now();
            if now.duration_since(last_tick) >= std::time::Duration::from_millis(100) {
                last_tick = now;
                let _ = on_event.send(DiskScanEvent::Progress {
                    root: root_display.clone(),
                    files,
                    dirs,
                    bytes,
                });
            }
        });
        // 扫描结束后清空句柄（仅当还指向本次扫描时）。
        {
            let mut guard = slot.lock().unwrap();
            if let Some(current) = guard.as_ref() {
                if Arc::ptr_eq(current, &cancelled) {
                    *guard = None;
                }
            }
        }
        let event = match result {
            Ok(done) => DiskScanEvent::Done {
                root: done.root,
                cancelled: done.cancelled,
                tree: done.tree_root.into(),
            },
            Err(cause) => {
                // 出错也要给前端收尾事件，UI 才能脱离「扫描中」状态。
                crate::logging::log("warn", &format!("磁盘扫描 {root_display} 失败：{cause}"));
                DiskScanEvent::Done {
                    root: root_display,
                    cancelled: true,
                    tree: DiskEntry {
                        name: String::new(),
                        path: String::new(),
                        size: 0,
                        dir: true,
                        children: Vec::new(),
                    },
                }
            }
        };
        let _ = on_event.send(event);
    });
    Ok(())
}

// ---- 内部：只读遍历 ----

/// 扫描一层：子项各自递归求大小，父项大小 = 子项之和（单次遍历求全树）。
fn scan_dir(path: &Path) -> DiskEntry {
    let mut children: Vec<DiskEntry> = std::fs::read_dir(path)
        .map(|readings| {
            readings
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    entry_info(&entry.path(), name)
                })
                .collect()
        })
        .unwrap_or_default();
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));

    let total = children.iter().map(|entry| entry.size).sum::<u64>();
    if children.len() > CHILDREN_LIMIT {
        let rest: Vec<DiskEntry> = children.split_off(CHILDREN_LIMIT);
        let rest_size = rest.iter().map(|entry| entry.size).sum::<u64>();
        children.push(DiskEntry {
            name: format!("其余 {} 项", rest.len()),
            path: String::new(),
            size: rest_size,
            dir: false,
            children: Vec::new(),
        });
    }

    DiskEntry {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
        path: path.display().to_string(),
        size: total,
        dir: true,
        children,
    }
}

/// 单个子项：目录递归求和，符号链接 / junction 不深入（防环）。
fn entry_info(path: &Path, name: String) -> DiskEntry {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return DiskEntry {
                name,
                path: path.display().to_string(),
                size: 0,
                dir: false,
                children: Vec::new(),
            }
        }
    };
    let dir = metadata.is_dir();
    let size = if metadata.is_symlink() {
        0
    } else if dir {
        walk_size(path)
    } else {
        metadata.len()
    };
    DiskEntry {
        name,
        path: path.display().to_string(),
        size,
        dir,
        children: Vec::new(),
    }
}

fn walk_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(readings) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in readings.flatten() {
        let child = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&child) else {
            continue;
        };
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            total += walk_size(&child);
        } else {
            total += metadata.len();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 扫描聚合与占位项() {
        let dir = std::env::temp_dir().join(format!("qx-disk-{}", std::process::id()));
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.join("a.txt"), [0u8; 10]).unwrap();
        std::fs::write(nested.join("b.bin"), [0u8; 100]).unwrap();

        let entry = scan_dir(&dir);
        assert_eq!(entry.size, 110);
        assert_eq!(entry.children.len(), 2);
        // 降序：nested(100) 在 a.txt(10) 前。
        assert_eq!(entry.children[0].name, "nested");
        assert!(entry.children[0].dir);

        // 清理后的目录也不残留。
        std::fs::remove_dir_all(&dir).ok();
    }
}

//! 磁盘清理域：目录占用扫描（下一层子项 + 递归大小）与回收站清理。
//!
//! 设计约束：
//! - 扫描是纯只读遍历，spawn_blocking 里跑，不占异步线程池的同步线程；
//! - 符号链接 / junction 不深入（防环、不重复计容），链接本身记 0；
//! - 巨目录的子项列表截断为 top N，尾部聚合为一个占位项——`size` 恒真，
//!   占位项 path 为空，UI 据此禁用下钻与清理；
//! - 清理走回收站（trash crate）：可撤销，不做物理直删。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::AppHandle;

use crate::error::{Error, Result};

/// 子项列表上限：超出部分聚合成「其余」占位，防止巨目录撑爆 IPC。
const CHILDREN_LIMIT: usize = 200;

/// 目录占用条目：磁盘清理页一个方块的数据形状。
#[derive(Serialize)]
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

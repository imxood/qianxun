//! 原子写：同目录临时文件写入 + fsync + rename 替换。
//!
//! 为什么不用「写完再 rename 回原位」之外的方案：崩溃可能发生在任一时刻，
//! 两个 rename 语义之间的窗口里磁盘上要么是完整旧文件、要么是完整新文件，
//! 不存在半截 JSON。Windows 的 rename 不覆盖已存在目标，所以正式文件先删后换；
//! 若删除后、换名前崩溃，会留下 .tmp 残迹，但读取方只认正式文件名，无影响。

use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // 注意：不带目录成分的相对路径（如 "settings.json"）的 parent() 是
    // 空路径而非 None——空目录会被当作「写到当前目录」，这里显式拒绝，
    // 防止调用方笔误把文件写进进程工作目录。
    let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty());
    let parent = parent
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "路径必须包含目录部分"))?;
    fs::create_dir_all(parent)?;

    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        // fsync 保证内容落盘后才进入替换阶段；没有它，掉电时 rename 可能
        // 先于数据到达磁盘。
        file.sync_all()?;
    }

    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 写入并读回内容一致() {
        let dir = std::env::temp_dir().join(format!("qx-atomic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        write(&path, b"{\"ok\":true}").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"ok\":true}");
        // 覆盖写同样成立，且不留临时文件。
        write(&path, b"{\"ok\":false}").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"ok\":false}");
        assert!(!dir.join("settings.json.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 根路径这类无父目录的输入被显式拒绝() {
        let err = write(Path::new("settings.json"), b"{}").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}

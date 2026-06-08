// vault 快照文件路径: <app_local_data_dir>/stronghold-snapshot.bin
// 选 iota_stronghold 默认文件位置之外的自定义路径, 避免和插件默认值冲突
// (Stage 7 加 plugin 时可统一).

use iota_stronghold::SnapshotPath;
use tauri::{AppHandle, Manager};

pub const VAULT_CLIENT_PATH: &[u8] = b"main";

pub fn vault_snapshot_path(app: &AppHandle) -> Result<SnapshotPath, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("resolve app_local_data_dir failed: {e}"))?;
    // 确保目录存在 (首次启动时)
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create app_local_data_dir {:?} failed: {e}", dir))?;
    let path = dir.join("stronghold-snapshot.bin");
    Ok(SnapshotPath::from_path(path))
}
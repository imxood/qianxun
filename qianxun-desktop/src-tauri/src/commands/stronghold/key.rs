// 2 个 Tauri command: set_secret / get_secret
// (delete_secret TS 端有 mock, Rust 端等 4a 后续补, bridge.ts 走 isTauri 守卫 web fallback)

use tauri::AppHandle;

use super::snapshot::vault_snapshot_path;
use super::vault;

/// 加密存到 stronghold vault. 密码用于快照加密, 重启后需重新输入.
#[tauri::command]
pub async fn set_secret(
    app: AppHandle,
    key: String,
    value: String,
    password: String,
) -> Result<(), String> {
    let snapshot_path = vault_snapshot_path(&app)?;
    vault::set(&snapshot_path, &password, &key, &value)?;
    tracing::info!(key = %key, "stronghold: secret stored");
    Ok(())
}

/// 从 stronghold vault 解密读取. 密码错误或 key 不存在时返回 Ok(None), 不抛异常.
#[tauri::command]
pub async fn get_secret(
    app: AppHandle,
    key: String,
    password: String,
) -> Result<Option<String>, String> {
    let snapshot_path = vault_snapshot_path(&app)?;
    vault::get(&snapshot_path, &password, &key)
}

/// 2026-06-12 (Phase B.2): 删除指定 key.
/// 返 Ok(true) = 真删了, Ok(false) = vault 不存在或 key 不存在. 密码错误才返 Err.
#[tauri::command]
pub async fn delete_secret(
    app: AppHandle,
    key: String,
    password: String,
) -> Result<bool, String> {
    let snapshot_path = vault_snapshot_path(&app)?;
    vault::delete(&snapshot_path, &password, &key)
}
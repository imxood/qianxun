// 3 个 Tauri command: set_secret / get_secret / delete_secret.

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

/// 2026-06-12 (Phase B.2, 批次 2.7 升级): 删除指定 key.
/// 返 DeleteOutcome 结构化枚举 (snapshot_missing / client_missing / key_missing / deleted),
/// 不再合并成 Ok(false). 密码错误才返 Err.
/// 2026-06-12 (批次 2.8): commit 失败时回滚, 不留"内存改了磁盘没改"半态.
#[tauri::command]
pub async fn delete_secret(
    app: AppHandle,
    key: String,
    password: String,
) -> Result<vault::DeleteOutcome, String> {
    let snapshot_path = vault_snapshot_path(&app)?;
    vault::delete(&snapshot_path, &password, &key)
}
//! 设置域 IPC 命令。命令函数只做「取状态 → 调域逻辑 → 返回」，
//! 校验与合并规则全部在 settings 模块（编码规范 §2：域内自治）。

use serde_json::Value;
use tauri::State;

use super::{apply_patch, save, Settings};
use crate::error::Result;
use crate::paths;
use crate::AppState;

/// 中毒恢复：临界区只含纯内存操作与落盘，恢复旧值继续用，
/// 比让整个设置系统从此死锁更合理（编码规范 §6：锁持有最小化）。
fn locked<'a>(state: &'a State<'_, AppState>) -> std::sync::MutexGuard<'a, Settings> {
    state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Result<Settings> {
    Ok(locked(&state).clone())
}

#[tauri::command]
pub fn settings_update(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    patch: Value,
) -> Result<Settings> {
    let path = paths::settings_path(&app)?;

    let mut guard = locked(&state);
    let next = apply_patch(&guard, &patch)?;
    // 先落盘再更新内存：失败时内存仍是旧值，UI 与磁盘保持一致。
    save(&path, &next)?;
    *guard = next.clone();
    drop(guard);
    // 远程网关随 remote 域变化自动重启（幂等：无变化则原地保留）。
    tauri::async_runtime::spawn(crate::remote::commands::sync(app.clone()));
    Ok(next)
}

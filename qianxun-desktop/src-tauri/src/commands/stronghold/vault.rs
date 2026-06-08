// set / get 实际逻辑 (调 KeyProvider + Stronghold + SnapshotPath)
// 业务跟 command 解耦, 方便单测 + 后续复用 (比如 desktop 内的 background task).
//
// 修 Finding 3 (verifier 报告): 第二次 set 时, "main" client 已存在,
// create_client 会返 "already exists". 先 try-load, 失败再 create.
//
// 修 Finding 2 (verifier 报告): get_client 拿不到刚 load 的 snapshot 里的 client
// (ClientDataNotPresent). 改用 load_client (从 snapshot 加载到 session).

use iota_stronghold::{SnapshotPath, Stronghold};

use super::keyprovider::make_keyprovider;
use super::snapshot::VAULT_CLIENT_PATH;

pub fn set(
    snapshot_path: &SnapshotPath,
    password: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let keyprovider = make_keyprovider(password)?;

    let stronghold = Stronghold::default();
    // 快照存在 → 先加载 (让 stronghold 拿到所有已存 client + store 数据).
    // 快照不存在 → 全新 vault, 跳过 load.
    if snapshot_path.exists() {
        stronghold
            .load_snapshot(&keyprovider, snapshot_path)
            .map_err(|e| format!("stronghold load_snapshot failed: {e}"))?;
    }

    let client = match stronghold.load_client(VAULT_CLIENT_PATH) {
        Ok(c) => c,
        Err(_) => stronghold
            .create_client(VAULT_CLIENT_PATH)
            .map_err(|e| format!("stronghold create_client failed: {e}"))?,
    };
    let store = client.store();
    // lifetime = None → 永不过期 (Stage 7 可加 token refresh 逻辑)
    store
        .insert(key.as_bytes().to_vec(), value.as_bytes().to_vec(), None)
        .map_err(|e| format!("stronghold insert failed: {e}"))?;

    stronghold
        .commit_with_keyprovider(snapshot_path, &keyprovider)
        .map_err(|e| format!("stronghold commit failed: {e}"))?;

    Ok(())
}

pub fn get(
    snapshot_path: &SnapshotPath,
    password: &str,
    key: &str,
) -> Result<Option<String>, String> {
    if !snapshot_path.exists() {
        // vault 从未初始化 → 没有 key
        return Ok(None);
    }
    let keyprovider = make_keyprovider(password)?;

    let stronghold = Stronghold::default();
    stronghold
        .load_snapshot(&keyprovider, snapshot_path)
        .map_err(|e| format!("stronghold load_snapshot failed (wrong password?): {e}"))?;

    let client = stronghold
        .load_client(VAULT_CLIENT_PATH)
        .map_err(|e| format!("stronghold load_client failed: {e}"))?;
    let store = client.store();

    match store
        .get(key.as_bytes())
        .map_err(|e| format!("stronghold get failed: {e}"))?
    {
        Some(bytes) => {
            // stronghold 返回 Vec<u8>, 强转 String (API key 永远 UTF-8 合法)
            let s = String::from_utf8(bytes)
                .map_err(|e| format!("stronghold returned non-UTF8 secret: {e}"))?;
            Ok(Some(s))
        }
        None => Ok(None),
    }
}
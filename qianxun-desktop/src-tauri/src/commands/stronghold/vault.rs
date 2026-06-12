// set / get 实际逻辑 (调 KeyProvider + Stronghold + SnapshotPath)
// 业务跟 command 解耦, 方便单测 + 后续复用 (比如 desktop 内的 background task).
//
// 修 Finding 3 (verifier 报告): 第二次 set 时, "main" client 已存在,
// create_client 会返 "already exists". 先 try-load, 失败再 create.
//
// 修 Finding 2 (verifier 报告): get_client 拿不到刚 load 的 snapshot 里的 client
// (ClientDataNotPresent). 改用 load_client (从 snapshot 加载到 session).
//
// 2026-06-12 (批次 2.7): delete 函数返 DeleteOutcome 4 态结构化枚举, 不再合并成 Ok(bool).
// 2026-06-12 (批次 2.8): set / delete commit 失败时回滚内存 store, 不留"内存改了磁盘没改"半态.

use iota_stronghold::{SnapshotPath, Stronghold};

use super::keyprovider::make_keyprovider;
use super::snapshot::VAULT_CLIENT_PATH;

/// 2026-06-12 (批次 2.7): delete 操作的结构化结果.
/// 规范 10 命名准确: 不再用 Ok(bool) 混淆 "snapshot 不存在" / "key 不存在" / "vault 损坏" 三态.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteOutcome {
    /// 快照文件不存在, vault 从未初始化
    SnapshotMissing,
    /// snapshot 存在, 但 client 未创建 (理论上不应发生, 但快照可能从旧版本继承)
    ClientMissing,
    /// client 存在, 但该 key 没存过
    KeyMissing,
    /// 真删了
    Deleted,
}

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
    // 2026-06-12 (批次 2.8): 留旧值, commit 失败时回滚 (新发现 C).
    // 内存 store 操作是 infallible (除非 OOM), 失败是 commit 阶段的磁盘 IO 错误.
    let old_value = store
        .get(key.as_bytes())
        .map_err(|e| format!("stronghold get old value failed: {e}"))?
        .map(|b| b.to_vec());
    // lifetime = None → 永不过期 (Stage 7 可加 token refresh 逻辑)
    store
        .insert(key.as_bytes().to_vec(), value.as_bytes().to_vec(), None)
        .map_err(|e| format!("stronghold insert failed: {e}"))?;

    if let Err(e) = stronghold.commit_with_keyprovider(snapshot_path, &keyprovider) {
        // 回滚: 把旧值重新 insert (没旧值就 delete), 内存状态跟磁盘一致.
        // 内存 store 操作一般不会失败, 若失败错误信息明确告知.
        let rollback_result: Result<(), String> = match old_value {
            Some(v) => store
                .insert(key.as_bytes().to_vec(), v, None)
                .map(|_| ())
                .map_err(|re| format!("{re}; rollback also failed")),
            None => store
                .delete(key.as_bytes())
                .map(|_| ())
                .map_err(|re| format!("{re}; rollback also failed")),
        };
        return Err(match rollback_result {
            Ok(()) => format!("stronghold commit failed: {e} (rolled back)"),
            Err(rollback_err) => {
                format!("stronghold commit failed: {e}; AND rollback failed: {rollback_err}; in-memory state unknown, please restart app")
            }
        });
    }

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

/// 2026-06-12 (Phase B.2; 批次 2.7 升级): 删除指定 key, 返 DeleteOutcome 结构化枚举.
/// 2026-06-12 (批次 2.8): commit 失败时回滚 (新发现 C).
pub fn delete(
    snapshot_path: &SnapshotPath,
    password: &str,
    key: &str,
) -> Result<DeleteOutcome, String> {
    if !snapshot_path.exists() {
        return Ok(DeleteOutcome::SnapshotMissing);
    }
    let keyprovider = make_keyprovider(password)?;
    let stronghold = Stronghold::default();
    stronghold
        .load_snapshot(&keyprovider, snapshot_path)
        .map_err(|e| format!("stronghold load_snapshot failed (wrong password?): {e}"))?;
    let client = match stronghold.load_client(VAULT_CLIENT_PATH) {
        Ok(c) => c,
        Err(_) => return Ok(DeleteOutcome::ClientMissing), // 不抛, 视为 missing
    };
    let store = client.store();
    let removed = store
        .delete(key.as_bytes())
        .map_err(|e| format!("stronghold delete failed: {e}"))?;
    // stronghold store().delete() 返回 Option<Vec<u8>>: Some(old_value)=存在并删除, None=本就不存在
    let old_value = match removed {
        Some(v) => v.to_vec(),
        None => return Ok(DeleteOutcome::KeyMissing),
    };
    if let Err(e) = stronghold.commit_with_keyprovider(snapshot_path, &keyprovider) {
        // 回滚: 重新 insert 旧值, 内存状态恢复.
        let rollback = store
            .insert(key.as_bytes().to_vec(), old_value, None)
            .map(|_| ())
            .map_err(|re| format!("{re}; rollback also failed"));
        return Err(match rollback {
            Ok(()) => format!("stronghold commit failed: {e} (rolled back)"),
            Err(rollback_err) => {
                format!("stronghold commit failed: {e}; AND rollback failed: {rollback_err}; in-memory state unknown, please restart app")
            }
        });
    }
    Ok(DeleteOutcome::Deleted)
}
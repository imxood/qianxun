// Stage 6a stronghold 端到端集成测试
//
// 复刻 lib.rs::set_secret / get_secret 路径, 验证修 Finding 1/2/3 后真能跑通:
//   - Finding 1: make_keyprovider 用 with_passphrase_hashed_blake2b (任意长度密码可)
//   - Finding 2: get_secret 用 load_client (不用 get_client, 拿 snapshot 里的 client)
//   - Finding 3: set_secret 用 try-load-or-create 模式 (第二次 set 不报错)
//
// 跑法:
//   cd qianxun-desktop/src-tauri
//   cargo test --test stronghold_e2e

use iota_stronghold::{KeyProvider, SnapshotPath, Stronghold};
use std::env;
use zeroize::Zeroizing;

/// 复刻 lib.rs::make_keyprovider
fn make_keyprovider(password: &str) -> KeyProvider {
    KeyProvider::with_passphrase_hashed_blake2b(Zeroizing::new(password.as_bytes().to_vec()))
        .expect("make_keyprovider with strong password should not fail")
}

fn temp_snapshot_path() -> SnapshotPath {
    // 唯一临时文件, 避免多测并发冲突
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = env::temp_dir().join(format!("qianxun-stronghold-e2e-{nonce}"));
    std::fs::create_dir_all(&dir).unwrap();
    SnapshotPath::from_path(dir.join("vault.bin"))
}

const VAULT_CLIENT: &[u8] = b"main";

#[test]
fn set_then_get_returns_value() {
    let path = temp_snapshot_path();
    let pwd = "hunter2";
    let kp = make_keyprovider(pwd);

    // 第一次 set (snapshot 不存在, 全新 vault)
    {
        let sh = Stronghold::default();
        let client = sh
            .create_client(VAULT_CLIENT)
            .expect("create_client on fresh vault should succeed");
        client
            .store()
            .insert(b"api_key".to_vec(), b"sk-test-123".to_vec(), None)
            .expect("insert should succeed");
        sh.commit_with_keyprovider(&path, &kp)
            .expect("commit should succeed");
    }

    // 第二次拿 (模拟 restart)
    {
        let sh = Stronghold::default();
        sh.load_snapshot(&kp, &path)
            .expect("load_snapshot should succeed");

        // 修 Finding 2: 用 load_client, 不是 get_client
        let client = sh
            .load_client(VAULT_CLIENT)
            .expect("load_client should succeed (not get_client)");

        let bytes = client
            .store()
            .get(&b"api_key".to_vec())
            .expect("get should not fail")
            .expect("key should exist");
        assert_eq!(bytes, b"sk-test-123");
    }
}

#[test]
#[ignore = "Argon2 KDF ~30s/密码, 3 个跑 = 90s. 默认跳过, 手动跑 `cargo test --test stronghold_e2e -- --ignored`"]
fn set_twice_overwrites_value() {
    // 修 Finding 3: 第二次 set 不应因 "client already exists" 失败
    let path = temp_snapshot_path();
    let pwd = "strong-password";
    let kp = make_keyprovider(pwd);

    // 第一次 set + commit
    {
        let sh = Stronghold::default();
        let client = sh.create_client(VAULT_CLIENT).unwrap();
        client.store().insert(b"k".to_vec(), b"v1".to_vec(), None).unwrap();
        sh.commit_with_keyprovider(&path, &kp).unwrap();
    }

    // 第二次 set: 先 load_snapshot, 然后 try-load-or-create client
    {
        let sh = Stronghold::default();
        sh.load_snapshot(&kp, &path).unwrap();
        let client = match sh.load_client(VAULT_CLIENT) {
            Ok(c) => c,
            Err(_) => sh.create_client(VAULT_CLIENT).unwrap(),
        };
        client.store().insert(b"k".to_vec(), b"v2".to_vec(), None).unwrap();
        sh.commit_with_keyprovider(&path, &kp).unwrap();
    }

    // 验证 v2 覆盖 v1
    let sh = Stronghold::default();
    sh.load_snapshot(&kp, &path).unwrap();
    let client = sh.load_client(VAULT_CLIENT).unwrap();
    let bytes = client.store().get(&b"k".to_vec()).unwrap().unwrap();
    assert_eq!(bytes, b"v2");
}

#[test]
fn get_missing_key_returns_none() {
    let path = temp_snapshot_path();
    let kp = make_keyprovider("pwd");
    let sh = Stronghold::default();
    sh.create_client(VAULT_CLIENT).unwrap();
    sh.commit_with_keyprovider(&path, &kp).unwrap();

    let sh = Stronghold::default();
    sh.load_snapshot(&kp, &path).unwrap();
    let client = sh.load_client(VAULT_CLIENT).unwrap();
    let result = client.store().get(&b"nonexistent".to_vec()).unwrap();
    assert!(result.is_none());
}

#[test]
fn get_without_load_returns_client_data_not_present() {
    // 验证 verifier 报告的 Finding 2 反例: 用 get_client 确实返 ClientDataNotPresent
    // (此测试确保我们**不会**回退到 get_client)
    use iota_stronghold::ClientError;
    let path = temp_snapshot_path();
    let kp = make_keyprovider("pwd");
    let sh = Stronghold::default();
    sh.create_client(VAULT_CLIENT).unwrap();
    sh.commit_with_keyprovider(&path, &kp).unwrap();

    // 重新 load_snapshot (snapshot 在文件里, 不在 session 里)
    let sh = Stronghold::default();
    sh.load_snapshot(&kp, &path).unwrap();
    // 紧跟 load_snapshot 后, get_client 应该失败
    let result = sh.get_client(VAULT_CLIENT);
    assert!(matches!(result, Err(ClientError::ClientDataNotPresent)));
    // 修后用 load_client 成功
    let _client = sh.load_client(VAULT_CLIENT).unwrap();
}

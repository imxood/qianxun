// stronghold domain — iota_stronghold 凭据加密 vault (Argon2 + ChaCha20, §11.3)
//
//   key.rs:        2 个 Tauri command (set_secret / get_secret) — 参数 + 调 vault
//   vault.rs:      set/get/delete 实际逻辑 (Stronghold + SnapshotPath + KeyProvider)
//   keyprovider.rs: 密码 → KeyProvider (blake2b KDF, 任意长度密码可)
//   snapshot.rs:   vault_snapshot_path + VAULT_CLIENT_PATH 常量
//
// 实现细节 (Stage 6a 沿用):
//   tauri-plugin-stronghold v2.3.1 没有公开的 Rust API (只有 JS-bound invoke
//   handlers, state 是私有的). 直接用 iota-stronghold (plugin 的底层) 实现
//   set/get, 同一加密引擎, 跳过 plugin wrapper. 详见 deliverable.md.
//
// 不做 (Stage 7 留):
//   - 自动密码派生 / 强度校验
//   - 启动时自动解锁 (需重新输密码)
//   - key rotation / 双向加密协议
//   - keyring (OS) 兜底 — 留 §11.4 P1 升级
//   - delete_secret (TS 端有 mock, Rust 端等 4a 后续)

pub mod key;
mod keyprovider;
mod snapshot;
mod vault;
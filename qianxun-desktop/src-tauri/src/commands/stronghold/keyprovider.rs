// 密码 → KeyProvider (Argon2 + blake2b KDF)
// 任意长度密码可, Zeroizing 包裹明文.
//
// 修 Finding 1 (verifier 报告): 原 `try_from` 把 data 当 32 字节 NaCl 密钥,
// 用户典型密码 (7-20 字节) 直接 panic (NCSizeNotAllowed). 改用
// `with_passphrase_hashed_blake2b` 走 KDF, 任意长度密码可, Zeroizing 包裹.

use iota_stronghold::KeyProvider;
use zeroize::Zeroizing;

pub fn make_keyprovider(password: &str) -> Result<KeyProvider, String> {
    KeyProvider::with_passphrase_hashed_blake2b(Zeroizing::new(password.as_bytes().to_vec()))
        .map_err(|e| format!("KeyProvider failed: {e}"))
}
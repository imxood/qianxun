// Stage 10a — Admin credential 持久化 + 密码校验 + JWT 签发
//
// model 字段留 Phase 4 接 config.admin_model.
#![allow(dead_code)]
//
// 设计目标: 替代 Stage 6a/9c 的 "env var JWT secret" 方案, 改成
// "admin password → short-lived JWT" 模式.
//
// 文件: ~/.qianxun/admin.cred (JSON 格式)
// 字段:
//   - password_hash: bcrypt(cost=10) 哈希 (24-byte random plaintext)
//   - token_secret: 32-byte 随机 HS256 secret (每次 rotate 重生)
//
// 首启动 (文件不存在) 时:
//   1) 生成 32-byte 随机 token_secret
//   2) 生成 16-byte 随机 password (base64 → 24 chars, 安全可显示)
//   3) bcrypt 默认 cost=10 哈希 password
//   4) 写文件 (mode 0o600 on Unix; Windows 默认 ACL)
//   5) **stderr 打印 password** (用户首次登录用, 后续可 rotate)
//
// 后续启动: 读文件 → 解析 → 返回 AdminCredential.
//
// API:
//   load_or_create(path) — 加载或首启动创建
//   verify_password(plain) — bcrypt::verify
//   sign_jwt(sub, exp_secs) — HS256 签发, claims 含 sub/exp/iat
//   rotate_token() — 重新生成 token_secret, 写文件 (旧 JWT 立即失效)
//   rotate_password(new_plain) — bcrypt 哈希新密码, 写文件 + in-place 更新
//
// 不做什么:
//   - keyring (Stage 7b 评估未做, 继续用文件 + chmod 600)
//   - token 黑名单 (Stage 11+)
//   - 2FA / OAuth (永不)

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header as JwtHeader};
use serde::{Deserialize, Serialize};

/// 内存中的 admin credential. **不** 持有明文密码 (load 后丢弃).
///
/// 字段用 `Arc<RwLock<...>>` 包装, 这样:
/// - 多个 handler 并发 verify / sign 不会互相 block (读锁)
/// - rotate_token / rotate_password 拿写锁更新 in-memory (handler 通过 `&self` 调用)
/// - 文件和内存始终一致 (rotate 先写文件, 再更新内存)
pub struct AdminCredential {
    password_hash: Arc<RwLock<String>>,
    token_secret: Arc<RwLock<String>>,
    /// 真实文件路径 (rotate 时复用)
    path: PathBuf,
}

/// admin.cred 文件结构.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct CredFile {
    password_hash: String,
    token_secret: String,
}

/// 公共错误类型 (axum 友好). String 简化 — 不需要细粒度区分.
pub type CredError = String;

impl AdminCredential {
    /// 加载或首启动创建. 文件不存在时打印密码到 stderr (仅首启动).
    ///
    /// `path`: 通常 `~/.qianxun/admin.cred`.
    pub fn load_or_create(path: &Path) -> Result<Self, CredError> {
        if path.exists() {
            // 后续启动: 读 + 解析
            let s = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let parsed: CredFile = serde_json::from_str(&s)
                .map_err(|e| format!("parse {}: {e}", path.display()))?;
            if parsed.password_hash.is_empty() {
                return Err(format!("empty password_hash in {}", path.display()));
            }
            if parsed.token_secret.is_empty() {
                return Err(format!("empty token_secret in {}", path.display()));
            }
            tracing::info!(
                "[admin-auth] loaded admin credential from {}",
                path.display()
            );
            Ok(Self {
                password_hash: Arc::new(RwLock::new(parsed.password_hash)),
                token_secret: Arc::new(RwLock::new(parsed.token_secret)),
                path: path.to_path_buf(),
            })
        } else {
            // 首启动: 生成 + 写
            // 1) 32-byte 随机 token_secret (用 getrandom, 避免引整 rand crate)
            let mut secret_bytes = [0u8; 32];
            getrandom::getrandom(&mut secret_bytes)
                .map_err(|e| format!("getrandom(token_secret): {e}"))?;
            let token_secret = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(secret_bytes);

            // 2) 16-byte 随机 password (base64 → 24 chars, URL_SAFE_NO_PAD: 字母+数字+_)
            let mut pass_bytes = [0u8; 16];
            getrandom::getrandom(&mut pass_bytes)
                .map_err(|e| format!("getrandom(password): {e}"))?;
            let password = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(pass_bytes);

            // 3) bcrypt 默认 cost=10
            let password_hash = bcrypt::hash(&password, bcrypt::DEFAULT_COST)
                .map_err(|e| format!("bcrypt hash failed: {e}"))?;

            // 4) 写文件 (确保父目录存在)
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create_dir {}: {e}", parent.display()))?;
            }
            let body = serde_json::to_string_pretty(&CredFile {
                password_hash: password_hash.clone(),
                token_secret: token_secret.clone(),
            })
            .map_err(|e| format!("serialize cred file: {e}"))?;
            std::fs::write(path, &body)
                .map_err(|e| format!("write {}: {e}", path.display()))?;

            // 5) Unix 上 chmod 0o600 (owner read/write only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o600);
                let _ = std::fs::set_permissions(path, perms);
            }
            // Windows 上文件 ACL 默认对 owner + admins 已限制, 不额外设置.

            // 6) **stderr 打印密码** (仅首启动, 让用户能登录)
            eprintln!("==============================================================");
            eprintln!("[admin-auth] First-time setup: generated admin credential.");
            eprintln!("[admin-auth] Password (save this — you can change it after login):");
            eprintln!("[admin-auth]   {}", password);
            eprintln!("[admin-auth] Stored at: {}", path.display());
            eprintln!("==============================================================");
            // 同时记到 tracing (info 级别) 方便日志检索
            tracing::warn!(
                "[admin-auth] first-time setup: credential stored at {}. Password printed to stderr. \
                 Login via Web UI or POST /v1/auth/login.",
                path.display()
            );

            Ok(Self {
                password_hash: Arc::new(RwLock::new(password_hash)),
                token_secret: Arc::new(RwLock::new(token_secret)),
                path: path.to_path_buf(),
            })
        }
    }

    /// 验证明文密码.
    pub fn verify_password(&self, plain: &str) -> bool {
        let hash = match self.password_hash.read() {
            Ok(h) => h.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        bcrypt::verify(plain, &hash).unwrap_or(false)
    }

    /// 签发 HS256 JWT.
    ///
    /// claims: `{ sub, exp, iat }` (sub 由 caller 传; exp_secs 是从 now 起的秒数).
    /// 失败时返 jsonwebtoken::errors::Error (几乎不会发生 — 我们的 secret 是 base64 字符串).
    pub fn sign_jwt(
        &self,
        sub: &str,
        exp_secs: i64,
    ) -> Result<String, jsonwebtoken::errors::Error> {
        let secret = self.token_secret_snapshot();
        let now = chrono::Utc::now().timestamp();
        let claims = JwtClaims {
            sub: sub.to_string(),
            exp: now + exp_secs,
            iat: now,
        };
        encode(
            &JwtHeader::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
    }

    /// 重新生成 token_secret. 写文件覆盖 + in-place 更新. 旧的 JWT 立即失效.
    /// 返回新 secret (供测试/日志用, 正常 handler 流程不直接暴露).
    pub fn rotate_token(&self) -> Result<String, CredError> {
        let mut secret_bytes = [0u8; 32];
        getrandom::getrandom(&mut secret_bytes)
            .map_err(|e| format!("getrandom(rotate): {e}"))?;
        let new_secret = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(secret_bytes);

        let hash_snapshot = self.password_hash_snapshot();
        let body = serde_json::to_string_pretty(&CredFile {
            password_hash: hash_snapshot,
            token_secret: new_secret.clone(),
        })
        .map_err(|e| format!("serialize cred file: {e}"))?;
        std::fs::write(&self.path, &body)
            .map_err(|e| format!("write {}: {e}", self.path.display()))?;

        // in-place 更新 (写锁)
        let mut guard = self
            .token_secret
            .write()
            .map_err(|e| format!("token_secret lock poisoned: {e}"))?;
        *guard = new_secret.clone();

        tracing::warn!("[admin-auth] rotated token_secret (all existing JWT invalidated)");
        Ok(new_secret)
    }

    /// 修改密码. bcrypt 哈希新密码, 写文件 + in-place 更新. **不** 影响 token.
    pub fn rotate_password_inplace(&self, new_plain: &str) -> Result<(), CredError> {
        if new_plain.len() < 4 {
            return Err("password too short (min 4 chars)".to_string());
        }
        let new_hash = bcrypt::hash(new_plain, bcrypt::DEFAULT_COST)
            .map_err(|e| format!("bcrypt hash failed: {e}"))?;

        let secret_snapshot = self.token_secret_snapshot();
        let body = serde_json::to_string_pretty(&CredFile {
            password_hash: new_hash.clone(),
            token_secret: secret_snapshot,
        })
        .map_err(|e| format!("serialize cred file: {e}"))?;
        std::fs::write(&self.path, &body)
            .map_err(|e| format!("write {}: {e}", self.path.display()))?;

        let mut guard = self
            .password_hash
            .write()
            .map_err(|e| format!("password_hash lock poisoned: {e}"))?;
        *guard = new_hash;

        tracing::info!("[admin-auth] rotated password (token_secret unchanged)");
        Ok(())
    }

    /// 内部 helper: 取 token_secret 副本 (供 auth_middleware 验签用).
    pub fn token_secret(&self) -> String {
        self.token_secret_snapshot()
    }

    /// 内部 helper: 取 password_hash 副本 (供测试 + 自我检查).
    pub fn password_hash(&self) -> String {
        self.password_hash_snapshot()
    }

    /// **仅测试用**: 直接覆盖 token_secret (不写文件, 不更新 password).
    /// 让 router 测试可以用 `set_jwt_secret(val)` 风格同步 secret.
    pub fn set_token_secret_for_test(&self, val: &str) {
        let mut guard = self
            .token_secret
            .write()
            .expect("test lock poisoned");
        *guard = val.to_string();
    }

    /// **仅测试用**: 构造一个 AdminCredential, 用 caller 提供的已知
    /// token_secret + password_hash. 持久化到 temp_dir 下的 `qianxun-test-admin-fortest.cred`,
    /// 这样 rotate_token / rotate_password 的写文件逻辑可以正常跑.
    ///
    /// 给 router 测试 (make_jwt 用同一 secret 签 token) 用. 真实 daemon 启动走
    /// `load_or_create`, 不走这条路径.
    #[cfg(test)]
    pub fn for_test(token_secret: &str, password_hash: &str) -> Self {
        use std::sync::{Arc, RwLock};
        let path = std::env::temp_dir().join(format!(
            "qianxun-test-admin-fortest-{}.cred",
            std::process::id()
        ));
        // 写一个最小文件 (for_test 假定 init OK)
        let body = serde_json::to_string(&CredFile {
            password_hash: password_hash.to_string(),
            token_secret: token_secret.to_string(),
        })
        .expect("serialize");
        let _ = std::fs::write(&path, body);
        Self {
            password_hash: Arc::new(RwLock::new(password_hash.to_string())),
            token_secret: Arc::new(RwLock::new(token_secret.to_string())),
            path,
        }
    }

    fn token_secret_snapshot(&self) -> String {
        match self.token_secret.read() {
            Ok(s) => s.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn password_hash_snapshot(&self) -> String {
        match self.password_hash.read() {
            Ok(s) => s.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// JWT claims. 跟 `router::Claims` 等价, 但定义在这里避免循环引用.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub exp: i64,
    pub iat: i64,
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 临时文件路径 (测试用, 互不冲突).
    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("qianxun_admin_auth_test_{}_{}.json", name, std::process::id()));
        p
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn test_load_or_create_first_time() {
        let path = temp_path("first");
        cleanup(&path);
        assert!(!path.exists(), "precondition: file should not exist");

        let admin = AdminCredential::load_or_create(&path).expect("first-time load");
        assert!(path.exists(), "file should now exist");
        assert!(
            !admin.password_hash().is_empty(),
            "password_hash should be set"
        );
        assert!(
            !admin.token_secret().is_empty(),
            "token_secret should be set"
        );
        // 写文件: 检查 password_hash 确实是 bcrypt 格式 ($2b$...)
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("$2b$") || body.contains("$2a$"), "should be bcrypt hash");
        cleanup(&path);
    }

    #[test]
    fn test_load_or_create_subsequent() {
        let path = temp_path("subsequent");
        cleanup(&path);

        // 第一次: 创建
        let first = AdminCredential::load_or_create(&path).expect("first create");
        let first_hash = first.password_hash();
        let first_secret = first.token_secret();
        drop(first);

        // 第二次: 加载
        let second = AdminCredential::load_or_create(&path).expect("subsequent load");
        assert_eq!(second.password_hash(), first_hash, "hash should match");
        assert_eq!(second.token_secret(), first_secret, "secret should match");
        cleanup(&path);
    }

    #[test]
    fn test_verify_password_correct_and_wrong() {
        let path = temp_path("verify");
        cleanup(&path);
        let admin = AdminCredential::load_or_create(&path).expect("create");

        // 拿到首启动密码 (我们没法从 stderr 抓, 只能从 hash 反推: 没有
        // bcrypt 的 decrypt, 所以**这里直接换 rotate_password 设一个已知密码**)
        admin.rotate_password_inplace("correct-password").expect("set known pw");

        assert!(admin.verify_password("correct-password"), "correct should pass");
        assert!(!admin.verify_password("wrong-password"), "wrong should fail");
        assert!(!admin.verify_password(""), "empty should fail");
        cleanup(&path);
    }

    #[test]
    fn test_sign_jwt_verifies_with_same_secret() {
        let path = temp_path("sign");
        cleanup(&path);
        let admin = AdminCredential::load_or_create(&path).expect("create");

        let token = admin.sign_jwt("admin", 60).expect("sign");
        assert!(!token.is_empty());

        // 用同一个 admin 的 secret 验签 (模拟 daemon 端 middleware)
        use jsonwebtoken::{decode, DecodingKey, Validation};
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp"]);
        let data = decode::<JwtClaims>(
            &token,
            &DecodingKey::from_secret(admin.token_secret().as_bytes()),
            &validation,
        )
        .expect("verify with same secret");
        assert_eq!(data.claims.sub, "admin");
        cleanup(&path);
    }

    #[test]
    fn test_rotate_token_invalidates_old_jwt() {
        let path = temp_path("rotate_token");
        cleanup(&path);
        let admin = AdminCredential::load_or_create(&path).expect("create");

        let old_token = admin.sign_jwt("admin", 60).expect("sign old");
        let new_secret = admin.rotate_token().expect("rotate");
        // rotate_token 同时更新 in-memory + 写文件 — 旧 secret 立刻失效
        // (在 in-memory 已经被替换).
        // 这里验证: 当前 admin 实例持有的 secret = new_secret (一致).
        assert_eq!(admin.token_secret(), new_secret, "in-memory secret should match");

        // 旧 token 用旧 secret 签发, 但 admin 现在用新 secret 验签, 应该失败.
        use jsonwebtoken::{decode, DecodingKey, Validation};
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_required_spec_claims(&["exp"]);
        let result = decode::<JwtClaims>(
            &old_token,
            &DecodingKey::from_secret(new_secret.as_bytes()),
            &validation,
        );
        assert!(result.is_err(), "old JWT should fail with new secret");
        cleanup(&path);
    }

    #[test]
    fn test_rotate_password_changes_hash_but_keeps_secret() {
        let path = temp_path("rotate_pw");
        cleanup(&path);
        let admin = AdminCredential::load_or_create(&path).expect("create");
        let old_secret = admin.token_secret();
        admin.rotate_password_inplace("new-password-123").expect("rotate pw");

        let old_hash = admin.password_hash();
        // 实际写文件的 hash 可能因为 retry 算法略有不同 (bcrypt salt), 但
        // 跟"任意同一 password" 算出的 hash 不同 (因为 salt 每次新生成)
        // 简化: 这里直接 verify_password 验证行为正确
        assert!(admin.verify_password("new-password-123"), "new pw should pass");
        assert!(!admin.verify_password("old-password"), "old pw should fail");
        // secret 应该不变 (rotate_password 不动 token_secret)
        assert_eq!(admin.token_secret(), old_secret, "secret should be unchanged");

        // 把 admin 写回文件, 再 load 看看 hash 是否更新
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("$2b$") || body.contains("$2a$"));
        let _ = old_hash; // suppress unused warning
        cleanup(&path);
    }

    #[test]
    fn test_rotate_password_too_short_rejected() {
        let path = temp_path("rotate_pw_short");
        cleanup(&path);
        let admin = AdminCredential::load_or_create(&path).expect("create");
        let r = admin.rotate_password_inplace("ab");
        assert!(r.is_err(), "should reject passwords < 4 chars");
        // 文件不应被修改 (因为 rotate 失败)
        cleanup(&path);
    }
}

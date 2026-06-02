//! WebSocket auth 验证 (Stage 3: SQLite-backed).
//!
//! ## Stage 2 → Stage 3 变化
//!
//! - **Stage 2**: 静态白名单 (`test_token_dt_xxx` / `_yyy`).
//! - **Stage 3**: 改用 `TeamDb::lookup_device` 查 SQLite `devices` 表.
//!   内部用 FNV-1a 64-bit 计算 token hash (Stage 4 升级到 SHA256).
//!
//! ## 设计原则 (不变)
//!
//! - **错误类型明确**: `AuthError` 三态 (InvalidToken / Expired / InternalError),
//!   对应 `_shared-contract.md` §3.3 `auth_error.code` 三种 code.
//! - **轻量级纯函数 + DB**: 不依赖 Hub 状态, 单测用 in-memory TeamDb 跑.

use chrono::{DateTime, Utc};
use rusqlite::Error as SqlError;

use super::team_db::TeamDb;

// ─── 公共类型 ──────────────────────────────────────────────

/// 设备验证成功后的元信息.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// 设备唯一标识, 来自 `WsFrame::Auth::machine_id` (DB 查出来的).
    pub machine_id: String,
    /// 设备在 VPS 注册时间 (来自 `devices.created_at`).
    pub registered_at: DateTime<Utc>,
}

/// Auth 验证失败原因.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// token 格式错误 / 不在 DB / 已禁用.
    InvalidToken,
    /// token 存在但已过期 (Stage 4 才区分, Stage 3 schema 无 `expires_at`).
    Expired,
    /// 内部错误 (DB 连接失败等).
    InternalError,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidToken => write!(f, "invalid device token"),
            Self::Expired => write!(f, "device token expired"),
            Self::InternalError => write!(f, "internal auth error"),
        }
    }
}

impl std::error::Error for AuthError {}

/// 把 `AuthError` 映射到 `_shared-contract.md` §3.3 `auth_error.code` 字符串.
pub fn auth_error_code(e: &AuthError) -> &'static str {
    match e {
        AuthError::InvalidToken => "invalid_token",
        AuthError::Expired => "expired",
        AuthError::InternalError => "internal_error",
    }
}

// ─── SQLite-backed 验证 ────────────────────────────────────

/// 验证 device_token 是否有效.
///
/// **Stage 3 实现**:
/// 1. token 为空 → `InvalidToken`.
/// 2. 计算 `token_hash = FNV-1a-64(token)` (Stage 4 升级 SHA256).
/// 3. 查 `TeamDb::lookup_device`:
///    - `Ok(Some(dev))` → `status == "active"` 才返回 `Ok(DeviceInfo)`, 否则 `InvalidToken`.
///    - `Ok(None)` → `InvalidToken`.
///    - `Err(_)` → `InternalError`.
///
/// # Errors
/// - `AuthError::InvalidToken`: 空 token / DB 查不到 / device 已禁用.
/// - `AuthError::Expired`: Stage 4 schema 加 `expires_at` 后才区分.
/// - `AuthError::InternalError`: DB 查询错误 (lock poisoned / IO 失败).
pub fn validate_device_token(db: &TeamDb, token: &str) -> Result<DeviceInfo, AuthError> {
    if token.is_empty() {
        return Err(AuthError::InvalidToken);
    }
    match db.lookup_device(token) {
        Ok(Some(dev)) => {
            if dev.status != "active" {
                return Err(AuthError::InvalidToken);
            }
            // created_at 形如 "2026-06-02T09:30:00.123456789+00:00"
            // chrono::DateTime::parse_from_rfc3339 可解析, 失败则用 now() 兜底.
            let registered_at = DateTime::parse_from_rfc3339(&dev.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            Ok(DeviceInfo {
                machine_id: dev.machine_id,
                registered_at,
            })
        }
        Ok(None) => Err(AuthError::InvalidToken),
        Err(e) => {
            tracing::warn!(error = %e, "TeamDb::lookup_device failed");
            Err(map_sql_error(e))
        }
    }
}

fn map_sql_error(e: SqlError) -> AuthError {
    // rusqlite 0.34 error variants: SqliteFailure / QueryReturnedNoRows / InvalidQuery /
    // InvalidColumnIndex / InvalidColumnName / InvalidColumnType / IntegralValueOutOfRange /
    // Utf8Error / FromSqlConversionFailure / ToSqlConversionFailure / UnwindingPanic /
    // InvalidPath / ... 多数是 tuple variant, 用 `..` 忽略内部字段.
    match e {
        SqlError::QueryReturnedNoRows => AuthError::InvalidToken, // 不会触发, 走 Ok(None) 路径
        SqlError::InvalidQuery
        | SqlError::InvalidColumnIndex(_)
        | SqlError::InvalidColumnName(_)
        | SqlError::InvalidColumnType(..)
        | SqlError::SqliteFailure(..)
        | SqlError::InvalidPath(_)
        | SqlError::FromSqlConversionFailure(..)
        | SqlError::ToSqlConversionFailure(..)
        | SqlError::UnwindingPanic
        | SqlError::SqliteSingleThreadedMode
        | SqlError::ExecuteReturnedResults
        | SqlError::StatementChangedRows(_) => AuthError::InternalError,
        _ => AuthError::InternalError, // 兜底
    }
}

// ─── 单测 ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::team_db::TeamDb;
    use rusqlite::Connection;

    /// 构造 in-memory TeamDb (不写文件, 预建 users 表满足 FK).
    fn test_db() -> TeamDb {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'user',
                created_at TEXT NOT NULL,
                disabled INTEGER NOT NULL DEFAULT 0
            )",
        )
        .expect("create users");
        TeamDb::from_connection(conn).expect("from_connection")
    }

    /// 预注册一个 device token.
    fn register(db: &TeamDb, token: &str, machine_id: &str, name: &str) {
        db.register_device(token, machine_id, name)
            .expect("register_device");
    }

    /// 测试 1: 合法 token → DeviceInfo, machine_id 正确, registered_at ~ now.
    #[test]
    fn test_valid_token_returns_device_info() {
        let db = test_db();
        register(&db, "dt_real_xxx", "machine_1", "office-pc");

        let info = validate_device_token(&db, "dt_real_xxx")
            .expect("valid token should pass");
        assert_eq!(info.machine_id, "machine_1");
        let now = Utc::now();
        let drift = (now - info.registered_at).num_seconds().abs();
        assert!(drift < 5, "registered_at should be ~now, drift={drift}s");
    }

    /// 测试 2: 不存在 token → InvalidToken, code 映射正确.
    #[test]
    fn test_invalid_token_returns_auth_error() {
        let db = test_db();
        let err = validate_device_token(&db, "wrong_token").unwrap_err();
        assert_eq!(err, AuthError::InvalidToken);
        assert_eq!(auth_error_code(&err), "invalid_token");
    }

    /// 测试 3: 空 token → InvalidToken.
    #[test]
    fn test_empty_token_returns_auth_error() {
        let db = test_db();
        let err = validate_device_token(&db, "").unwrap_err();
        assert_eq!(err, AuthError::InvalidToken);
        assert_eq!(auth_error_code(&err), "invalid_token");
    }

    /// 测试 4: 第二个注册的 token 也能命中.
    #[test]
    fn test_second_registered_token_works() {
        let db = test_db();
        register(&db, "dt_alpha", "machine_A", "alpha-pc");
        register(&db, "dt_beta", "machine_B", "beta-pc");

        let info_alpha = validate_device_token(&db, "dt_alpha").expect("alpha");
        assert_eq!(info_alpha.machine_id, "machine_A");
        let info_beta = validate_device_token(&db, "dt_beta").expect("beta");
        assert_eq!(info_beta.machine_id, "machine_B");
    }

    /// 测试 5: Display 错误信息可读.
    #[test]
    fn test_auth_error_display_messages() {
        assert_eq!(AuthError::InvalidToken.to_string(), "invalid device token");
        assert_eq!(AuthError::Expired.to_string(), "device token expired");
        assert_eq!(AuthError::InternalError.to_string(), "internal auth error");
    }
}

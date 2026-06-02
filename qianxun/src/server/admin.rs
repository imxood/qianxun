//! VPS Server Admin auth guard (Stage 4).
//!
//! ## Stage 4 简化范围
//!
//! Admin auth = JWT 中 `role` 字段是 `"admin"` 或 `"owner"`.
//! 从 `Authorization: Bearer <jwt>` 提取 JWT, 解析 claims, 检查 role.
//!
//! ## 与 Stage 1 关系
//!
//! - 复用 `qianxun/src/server/auth.rs::Claims` 结构 (已扩展 `Deserialize`).
//! - 复用同一个 `JWT_SECRET` 临时硬编码 (Stage 5 接 env var `QXVPS_JWT_SECRET`).
//!
//! ## 与 Stage 5+ 关系
//!
//! - 完整 admin 端点 (用户禁用, 角色修改, JWT 密钥轮换) 在 `02-vps-server.md` §11.2.4.
//! - 当前 Stage 4 只做最薄一层: 提取 + 验签 + role 检查.
//!
//! ## 错误模型
//!
//! 三态 `AdminError` 对应 HTTP 状态码:
//! - `Missing` → 401 Unauthorized (无 header)
//! - `Invalid` → 401 Unauthorized (签名错 / 格式错 / 过期)
//! - `Forbidden` → 403 Forbidden (role 不够)
//!
//! 调用方负责把 `AdminError` 映射成 `(StatusCode, String)` 响应.

use axum::http::HeaderMap;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

use crate::server::auth::{Claims, JWT_SECRET};

/// Admin auth 失败原因.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminError {
    /// 缺少 `Authorization` header.
    Missing,
    /// header 格式错 / JWT 解析失败 / 签名错 / 过期.
    Invalid,
    /// token 有效但 role 不足 (非 admin/owner).
    Forbidden,
}

impl std::fmt::Display for AdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "missing authorization header"),
            Self::Invalid => write!(f, "invalid or expired token"),
            Self::Forbidden => write!(f, "admin role required"),
        }
    }
}

impl std::error::Error for AdminError {}

/// Admin auth guard: 提取 JWT, 解码 claims, 检查 role.
///
/// ## 调用约定
///
/// 由 axum handler 作为第一个调用:
///
/// ```ignore
/// async fn admin_handler(
///     State(state): State<Arc<VpsState>>,
///     headers: axum::http::HeaderMap,
///     ...
/// ) -> impl IntoResponse {
///     let claims = match admin::require_admin(&headers) {
///         Ok(c) => c,
///         Err(AdminError::Missing | AdminError::Invalid) =>
///             return (StatusCode::UNAUTHORIZED, "auth required").into_response(),
///         Err(AdminError::Forbidden) =>
///             return (StatusCode::FORBIDDEN, "admin role required").into_response(),
///     };
///     // ... 业务逻辑
/// }
/// ```
///
/// ## Returns
///
/// - `Ok(Claims)` — 合法 admin/owner token, 返回完整 claims (`sub` = user_id).
/// - `Err(AdminError::Missing)` — 无 `Authorization` header.
/// - `Err(AdminError::Invalid)` — header 不是 `Bearer <token>` 格式 / token 为空 / 签名错 / 过期.
/// - `Err(AdminError::Forbidden)` — claims 解析成功但 `role` 不是 `"admin"` 或 `"owner"`.
pub fn require_admin(headers: &HeaderMap) -> Result<Claims, AdminError> {
    let auth_value = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(AdminError::Missing)?;

    let token = auth_value
        .strip_prefix("Bearer ")
        .or_else(|| auth_value.strip_prefix("bearer "))
        .ok_or(AdminError::Invalid)?
        .trim();

    if token.is_empty() {
        return Err(AdminError::Invalid);
    }

    // HS256 验签, 要求 exp claim 必须存在
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp"]);

    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(JWT_SECRET.as_bytes()),
        &validation,
    )
    .map_err(|_| AdminError::Invalid)?;

    let claims = data.claims;

    if claims.role == "admin" || claims.role == "owner" {
        Ok(claims)
    } else {
        Err(AdminError::Forbidden)
    }
}

// ─── 单测 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use jsonwebtoken::{encode, EncodingKey, Header};

    /// 签发测试用 JWT (同步用相同密钥, 1h 有效期).
    fn make_token_with_role(role: &str) -> String {
        let claims = Claims {
            sub: "user_alice".into(),
            role: role.into(),
            exp: (Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
        )
        .expect("encode test jwt")
    }

    /// 测试 1: 合法 admin JWT → Ok(Claims with role=admin).
    ///
    /// 验证完整路径: header 提取 → Bearer 剥离 → JWT 验签 → role 检查 → 返回 claims.
    #[test]
    fn test_admin_role_passes() {
        let mut headers = HeaderMap::new();
        let token = make_token_with_role("admin");
        headers.insert(
            "authorization",
            format!("Bearer {token}").parse().unwrap(),
        );

        let claims = require_admin(&headers).expect("admin should pass");
        assert_eq!(claims.role, "admin", "role should be admin");
        assert_eq!(claims.sub, "user_alice", "sub should be user_alice");
    }

    /// 测试 2: 合法普通 user JWT → Err(Forbidden).
    ///
    /// 验签成功 (token 合法) 但 role 不足, 必须返回 Forbidden 而非 Invalid
    /// (区分 401 vs 403).
    #[test]
    fn test_user_role_forbidden() {
        let mut headers = HeaderMap::new();
        let token = make_token_with_role("user");
        headers.insert(
            "authorization",
            format!("Bearer {token}").parse().unwrap(),
        );

        let err = require_admin(&headers).expect_err("user should be forbidden");
        assert_eq!(err, AdminError::Forbidden, "should be Forbidden, not Invalid");
    }
}

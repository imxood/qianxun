//! 内嵌控制台资产的 HTTP 层（docs/20 §7.4 决策 3）。
//!
//! 三条纪律：
//! - **缓存**：Vite 产物文件名带内容哈希 → `immutable` 一年；`index.html`
//!   不带哈希 → `no-cache`，否则升级后用户永远拿旧壳配新 API。
//! - **压缩**：构建期已生成 .gz（`pnpm build:console`），这里按 Accept-Encoding
//!   直接发压缩件，不在运行时压。
//! - **CSP**：无 CDN、无外链字体，所以 `default-src 'self'` 就够——二维码
//!   是内联 SVG，不需要 img-src 之外的放宽。
//!
//! `/qx-pair` 与 `/qx-fleet` 不走这里：它们永久保持零构建手写页（bootloader
//! 纪律，构建链坏了也不能阻断重新配对）。

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

include!(concat!(env!("OUT_DIR"), "/console_assets.rs"));

/// 单个内嵌文件（build.rs 生成）。
pub struct Asset {
    pub path: &'static str,
    pub mime: &'static str,
    pub raw: &'static [u8],
    pub gz: Option<&'static [u8]>,
}

fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|asset| asset.path == path)
}

/// 是否构建过前端（main.rs 用它在启动日志里提示，而不是让人对着空控制台发呆）。
pub fn embedded() -> bool {
    !ASSETS.is_empty()
}

const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
                    script-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; \
                    frame-ancestors 'none'";

fn response(asset: &'static Asset, headers: &HeaderMap, immutable: bool) -> Response {
    let accepts_gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("gzip"));
    let (body, encoding): (&'static [u8], Option<&'static str>) = match (accepts_gzip, asset.gz) {
        (true, Some(gz)) => (gz, Some("gzip")),
        _ => (asset.raw, None),
    };

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, asset.mime)
        .header(header::CONTENT_SECURITY_POLICY, CSP)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "same-origin")
        .header(
            header::CACHE_CONTROL,
            if immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            },
        );
    if let Some(encoding) = encoding {
        builder = builder.header(header::CONTENT_ENCODING, encoding);
    }
    builder
        .body(axum::body::Body::from(body))
        .expect("静态响应可构造")
}

/// 未构建前端时的 503：说清怎么修，而不是回一个空白 200。
fn missing() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "控制台资产未嵌入：请在仓库根运行 `pnpm build:console` 后重新编译 qx-server",
    )
        .into_response()
}

/// `/console`、`/console/`、`/login` —— 同一个 SPA 的三种入口（docs/20 §4.1）。
pub async fn serve_index(headers: HeaderMap) -> Response {
    match find("index.html") {
        Some(index) => response(index, &headers, false),
        None => missing(),
    }
}

/// `/console/assets/*` —— 带哈希的静态产物，长缓存。
pub async fn serve_asset(
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    // 路由是 `/console/assets/{*path}`，所以 path 里没有 `assets/` 前缀；
    // 表里的键是相对 dist/ 的路径，要补回去。
    match find(&format!("assets/{path}")) {
        Some(asset) => response(asset, &headers, true),
        // 未知路径按 404 而不是回 index.html：少一层 SPA fallback 的歧义。
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::HeaderValue;

    #[test]
    fn 查不存在的路径返回_none() {
        assert!(find("assets/不存在.js").is_none());
    }

    /// 路由剥掉了 `assets/` 前缀，补回去必须命中真产物（这条曾经错过，
    /// 症状是页面 HTML 出来了但 404 一片空白控制台）。
    #[tokio::test]
    async fn 带前缀路由能取到产物() {
        if !embedded() {
            return;
        }
        let hashed = ASSETS
            .iter()
            .find(|asset| asset.path.starts_with("assets/"))
            .expect("有产物就必有 assets/ 下的文件");
        let name = hashed.path.trim_start_matches("assets/");
        let response = serve_asset(axum::extract::Path(name.to_owned()), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1 << 20).await.expect("可读");
        assert_eq!(body.len(), hashed.raw.len());
    }

    #[test]
    fn 构建过前端时索引一定在() {
        // 空表（没跑 pnpm build:console）时 embedded() 必须是 false，
        // 否则启动提示会骗人。
        assert_eq!(embedded(), !ASSETS.is_empty());
        if embedded() {
            assert!(find("index.html").is_some(), "有资产就必须有 index.html");
        }
    }

    #[test]
    fn 哈希产物长缓存_壳不缓存() {
        let asset = match find("index.html") {
            Some(asset) => asset,
            None => return,
        };
        let shell = response(asset, &HeaderMap::new(), false);
        assert_eq!(
            shell
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-cache")
        );
        let hashed = response(asset, &HeaderMap::new(), true);
        assert!(hashed
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|value| value.contains("immutable")));
    }

    #[test]
    fn 带_gzip_的响应带编码头() {
        let asset = match ASSETS.iter().find(|a| a.gz.is_some()) {
            Some(asset) => asset,
            None => return,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate"),
        );
        let gzip = response(asset, &headers, true);
        assert_eq!(
            gzip.headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
            Some("gzip")
        );
        // 不接受压缩时必须回明文，否则浏览器拿到的是压不动的字节。
        let plain = response(asset, &HeaderMap::new(), true);
        assert!(plain.headers().get(header::CONTENT_ENCODING).is_none());
    }
}

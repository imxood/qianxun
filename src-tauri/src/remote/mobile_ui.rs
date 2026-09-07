//! 移动定制层（方案 A）：网关自有路由 `/qx-mobile/*` + 定制文件服务。
//!
//! 借鉴 dsh-mobile 的「文件即 UI」：`<DSH_HOME>/mobile-access/` 下的
//! `custom.css` / `custom.js` 由网关直接服务（不转发 DSH），文件保存后
//! 手机端数秒内自动生效——页面里的 bootstrap.js 轮询 `version` 的内容
//! hash，变化即热替换（见 assets/qx-mobile-bootstrap.js）。
//!
//! 路由一览（除 unknown 外全部要求已配对，cookie 或 query token）：
//! - `GET info` 电脑端身份（主机名/版本/DSH 就绪），供 App 端命名连接与在线探测；
//!   额外带 CORS `*`——Capacitor 壳在配对前（无 cookie）用 query token 探测。
//! - `GET version` `{v,css,js}` 内容 hash（css/js 各 16 位 hex），热刷新依据。
//! - `GET bootstrap.js` 定制层引导脚本（内嵌二进制，随千寻升级）。
//! - `GET custom.css` 定制样式，文件缺失回退注释占位，ETag = sha256。
//! - `GET custom.js` 定制脚本，同上。
//! - 其余 `/qx-mobile/*` 一律 404（不转发 DSH，避免把内部面漏给上游）。
//!
//! 未来电脑端能力（监控/文件/终端/通知……）都在本模块按 `/qx-mobile/<能力>` 扩展。

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, Response, StatusCode};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::remote::gateway::{authorized, plain, GatewayState};

/// 单文件上限（对齐 dsh-mobile：css 512KB / js 1MB），超出按 413 拒绝。
const CSS_LIMIT: usize = 512 * 1024;
const JS_LIMIT: usize = 1024 * 1024;

const CSS_FALLBACK: &str =
    "/* 在 mobile-access/custom.css 写手机端样式覆盖，保存后手机端数秒内自动生效。 */\n";
const JS_FALLBACK: &str =
    "/* 在 mobile-access/custom.js 写手机端脚本：window.qxMobile.register(({ root }) => { ... })，返回清理函数。 */\n";

/// 引导脚本内嵌二进制：随千寻发布，网关以 no-cache 下发。
const BOOTSTRAP_JS: &str = include_str!("assets/qx-mobile-bootstrap.js");

/// 定制层状态：两个定制文件的绝对路径（网关启动时解析一次）。
pub struct MobileUi {
    pub css_path: PathBuf,
    pub js_path: PathBuf,
}

impl MobileUi {
    pub fn new(mobile_access_dir: PathBuf) -> Self {
        Self {
            css_path: mobile_access_dir.join("custom.css"),
            js_path: mobile_access_dir.join("custom.js"),
        }
    }
}

// ---- 路由 handler ----

/// 电脑端身份：App 端「添加连接」时读取主机名自动命名 + 列表页在线探测。
pub async fn info(
    State(state): State<GatewayState>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response<Body> {
    if !authorized(&state, uri.query().unwrap_or(""), &headers) {
        return plain(StatusCode::UNAUTHORIZED, "未配对设备");
    }
    let hostname = hostname::get()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let body = json!({
        "app": "qianxun",
        "version": env!("CARGO_PKG_VERSION"),
        "hostname": hostname,
        "dshReady": state.upstream_ready(),
    });
    let mut response = json_response(&body);
    // CORS 放开：凭据是 query 里的配对 token 本身，cookie 不跨源，无凭据泄漏面。
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    response
}

/// 定制文件内容 hash：bootstrap.js 每 5s 轮询（页面可见时）。
pub async fn version(
    State(state): State<GatewayState>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response<Body> {
    if !authorized(&state, uri.query().unwrap_or(""), &headers) {
        return plain(StatusCode::UNAUTHORIZED, "未配对设备");
    }
    let css = short_hash_of(&state.mobile.css_path, CSS_FALLBACK).await;
    let js = short_hash_of(&state.mobile.js_path, JS_FALLBACK).await;
    json_response(&json!({ "v": 1, "css": css, "js": js }))
}

/// 定制层引导脚本（内嵌，no-cache：千寻升级后页面立取新版）。
pub async fn bootstrap(
    State(state): State<GatewayState>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response<Body> {
    if !authorized(&state, uri.query().unwrap_or(""), &headers) {
        return plain(StatusCode::UNAUTHORIZED, "未配对设备");
    }
    text_response(BOOTSTRAP_JS, "text/javascript; charset=utf-8", "no-cache")
}

pub async fn custom_css(
    State(state): State<GatewayState>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response<Body> {
    serve_custom(
        &state,
        uri.query().unwrap_or(""),
        &headers,
        &state.mobile.css_path,
        CSS_LIMIT,
        CSS_FALLBACK,
        "text/css; charset=utf-8",
    )
    .await
}

pub async fn custom_js(
    State(state): State<GatewayState>,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response<Body> {
    serve_custom(
        &state,
        uri.query().unwrap_or(""),
        &headers,
        &state.mobile.js_path,
        JS_LIMIT,
        JS_FALLBACK,
        "text/javascript; charset=utf-8",
    )
    .await
}

/// 未知 `/qx-mobile/*`：明确 404，不转发 DSH（防止内部路径误漏到上游）。
pub async fn unknown() -> Response<Body> {
    plain(StatusCode::NOT_FOUND, "未知 qx-mobile 路由")
}

// ---- 共用实现 ----

/// 定制文件服务：ENOENT 回退占位注释、超限 413、ETag/304。bootstrap 取文件
/// 时追加 `?v=<hash>` 版本参数，浏览器按 URL 长缓存，内容变化即换 URL。
async fn serve_custom(
    state: &GatewayState,
    query: &str,
    headers: &HeaderMap,
    path: &Path,
    limit: usize,
    fallback: &str,
    content_type: &str,
) -> Response<Body> {
    if !authorized(state, query, headers) {
        return plain(StatusCode::UNAUTHORIZED, "未配对设备");
    }
    let (body, from_file) = match tokio::fs::read(path).await {
        Ok(bytes) => (bytes, true),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            (fallback.as_bytes().to_vec(), false)
        }
        Err(cause) => {
            return plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("定制文件读取失败：{cause}"),
            )
        }
    };
    if body.len() > limit {
        return plain(StatusCode::PAYLOAD_TOO_LARGE, "定制文件超出大小上限");
    }
    let etag = format!("\"{}\"", full_hash(&body));
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    if if_none_match.is_some_and(|present| present.contains(etag.as_str())) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::CACHE_CONTROL, CUSTOM_CACHE_CONTROL)
            .body(Body::empty())
            .unwrap_or_else(|_| Response::new(Body::empty()));
    }
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, CUSTOM_CACHE_CONTROL)
        .header(header::ETAG, etag)
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    if !from_file {
        // 占位注释带提示头，方便真机排查「为什么样式没生效」。
        response.headers_mut().insert(
            header::HeaderName::from_static("x-qx-mobile-custom"),
            header::HeaderValue::from_static("fallback"),
        );
    }
    response
}

fn json_response(value: &serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(value.to_string()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn text_response(text: &str, content_type: &str, cache: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache)
        .header(header::ETAG, format!("\"{}\"", full_hash(text.as_bytes())))
        .body(Body::from(text.to_owned()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn full_hash(bytes: &[u8]) -> String {
    bytes_to_hex(&Sha256::digest(bytes))
}

/// 文件内容（缺失用 fallback）sha256 前 16 位 hex——version 端点与 URL 版本参数同源。
async fn short_hash_of(path: &Path, fallback: &str) -> String {
    let bytes = tokio::fs::read(path)
        .await
        .unwrap_or_else(|_| fallback.as_bytes().to_vec());
    full_hash(&bytes)[..16].to_owned()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 定制资源按 URL 版本参数失效，可放心长缓存。
const CUSTOM_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 定制文件路径拼接() {
        let ui = MobileUi::new(PathBuf::from("C:/tmp/mobile-access"));
        assert_eq!(
            ui.css_path,
            PathBuf::from("C:/tmp/mobile-access/custom.css")
        );
        assert_eq!(ui.js_path, PathBuf::from("C:/tmp/mobile-access/custom.js"));
    }

    #[test]
    fn hash_稳定性与长度() {
        assert_eq!(full_hash(b"hello"), full_hash(b"hello"));
        assert_ne!(full_hash(b"hello"), full_hash(b"world"));
        assert_eq!(full_hash(b"hello").len(), 64);
        assert_eq!(full_hash(b"hello")[..16].len(), 16);
    }

    #[test]
    fn 未知路由返回404() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("测试用单线程 runtime");
        let response = runtime.block_on(unknown());
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

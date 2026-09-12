//! npm registry 上的插件目录：搜索与详情。
//!
//! 插件就是普通的 npm 包（声明了 `dsh.bundle.patch` 的是插件，否则是库）。
//! 目录不维护精选清单：搜索直接走用户配置的 registry（默认 npmmirror），
//! 装得到才搜得到。空搜索词给生态发现词 `dsh`，让首屏有东西可看。

use std::time::Duration;

use serde::Serialize;

use crate::error::{Error, Result};
use crate::harness::install::PINNED_VERSION;

/// 空搜索框的发现词：不精选、不运营，registry 搜到什么看什么。
const DISCOVERY: &str = "dsh";

/// 单次搜索条数：够逛，不多到拖慢镜像。
const SEARCH_SIZE: &str = "100";

/// 宽裕些：镜像可能在美国之外的慢链路上。
const BUDGET: Duration = Duration::from_secs(20);

/// 一条搜索结果（列表行）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub name: String,
    pub version: String,
    pub description: String,
    pub publisher: String,
    /// ISO 时间戳（前端格式化）。
    pub updated: String,
    pub weekly_downloads: u64,
    pub link: Option<String>,
    /// registry base，详情/安装共用一个源，避免搜与装对不上。
    pub registry: String,
}

/// 一个包的发布详情（展开行）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    /// 声明了 `dsh.bundle.patch` = 插件；否则只是提及 DSH 的普通包。
    pub bundle: bool,
    /// 与千寻锁定的 DSH 版本的兼容结论。
    pub compatibility: Compatibility,
    /// 包管理器生命周期脚本（会执行本地代码），展示用。
    pub lifecycle_scripts: Vec<String>,
    /// 发布者标记的弃用说明；存在即拒绝从市场安装。
    pub deprecated: Option<String>,
    pub unpacked_bytes: Option<u64>,
    /// 确认按钮将安装的精确说明符。
    pub install_spec: String,
}

/// 兼容结论：pkg 声明的 `@deepseek-ai/dsh` 依赖范围 vs 千寻 PINNED_VERSION。
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Compatibility {
    Compatible { requirement: String },
    Unknown,
    Incompatible { requirement: String, reason: String },
}

/// 搜索 registry。空查询给发现词。
pub async fn search(registry: &str, query: &str) -> Result<Vec<Listing>> {
    let text = match query.trim() {
        "" => DISCOVERY,
        typed => typed,
    };
    let base = registry.trim_end_matches('/');
    let endpoint = format!(
        "{base}/-/v1/search?text={}&size={SEARCH_SIZE}",
        urlencode(text)
    );
    let body = fetch_json(&endpoint).await?;
    let objects = body
        .get("objects")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(objects
        .iter()
        .filter_map(|entry| listing(entry, base))
        .collect())
}

/// 读一个包的最新发布 manifest。
pub async fn detail(registry: &str, name: &str) -> Result<Detail> {
    if !is_package_name(name) {
        return Err(Error::Market(format!("{name} 不是合法的包名")));
    }
    let base = registry.trim_end_matches('/');
    let manifest = fetch_json(&format!("{base}/{name}/latest")).await?;
    Ok(detail_from_manifest(name, base, &manifest))
}

fn listing(entry: &serde_json::Value, base: &str) -> Option<Listing> {
    let package = entry.get("package")?;
    Some(Listing {
        name: string(package, "name")?,
        version: string(package, "version").unwrap_or_default(),
        description: string(package, "description").unwrap_or_default(),
        publisher: package
            .pointer("/publisher/username")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        updated: string(package, "date").unwrap_or_default(),
        weekly_downloads: entry
            .pointer("/downloads/weekly")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
        link: package.get("links").and_then(|links| {
            ["homepage", "repository", "npm"]
                .into_iter()
                .find_map(|key| string(links, key))
        }),
        registry: base.to_owned(),
    })
}

fn detail_from_manifest(name: &str, _base: &str, manifest: &serde_json::Value) -> Detail {
    let version = string(manifest, "version").unwrap_or_else(|| name.to_owned());
    let lifecycle_scripts = ["preinstall", "install", "postinstall", "prepare"]
        .into_iter()
        .filter(|script| manifest.pointer(&format!("/scripts/{script}")).is_some())
        .map(str::to_owned)
        .collect();
    Detail {
        name: string(manifest, "name").unwrap_or_else(|| name.to_owned()),
        install_spec: format!("{name}@{version}"),
        version,
        description: string(manifest, "description").unwrap_or_default(),
        license: string(manifest, "license").unwrap_or_default(),
        homepage: string(manifest, "homepage"),
        repository: manifest
            .pointer("/repository/url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        bundle: manifest.pointer("/dsh/bundle/patch").is_some(),
        compatibility: compatibility(manifest),
        lifecycle_scripts,
        deprecated: manifest.get("deprecated").and_then(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        }),
        unpacked_bytes: manifest
            .pointer("/dist/unpackedSize")
            .and_then(serde_json::Value::as_u64),
    }
}

/// 兼容判定（宽松）：rc 钉版的生态现实是包声明 `^0.1.x` 这类宽范围——
/// 直接按 semver 预发布规则比对会大量误杀，所以对当前版本同时试
/// 原样与去预发布两种形态，任一命中即兼容；解析不了的范围按未知处理。
fn compatibility(manifest: &serde_json::Value) -> Compatibility {
    let requirement = ["peerDependencies", "dependencies", "optionalDependencies"]
        .into_iter()
        .find_map(|field| {
            manifest
                .pointer(&format!("/{field}/@deepseek-ai~1dsh"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
    let Some(requirement) = requirement else {
        return Compatibility::Unknown;
    };
    let Ok(parsed) = semver::VersionReq::parse(&requirement) else {
        return Compatibility::Incompatible {
            reason: "声明了无法解析的版本范围".to_owned(),
            requirement: requirement.clone(),
        };
    };
    let Ok(current) = semver::Version::parse(PINNED_VERSION) else {
        return Compatibility::Unknown;
    };
    let release = semver::Version {
        pre: semver::Prerelease::EMPTY,
        ..current.clone()
    };
    if parsed.matches(&current) || parsed.matches(&release) {
        Compatibility::Compatible { requirement }
    } else {
        Compatibility::Incompatible {
            reason: format!("它要求 {requirement}"),
            requirement,
        }
    }
}

/// 安装前的硬门槛：弃用与不兼容拒绝安装。
pub(crate) fn validate(detail: &Detail) -> Result<()> {
    if let Some(reason) = &detail.deprecated {
        return Err(Error::Market(format!("该包已弃用：{reason}")));
    }
    if let Compatibility::Incompatible { reason, .. } = &detail.compatibility {
        return Err(Error::Market(format!(
            "与千寻适配的 DSH {PINNED_VERSION} 不兼容：{reason}"
        )));
    }
    Ok(())
}

async fn fetch_json(endpoint: &str) -> Result<serde_json::Value> {
    let response = reqwest::Client::new()
        .get(endpoint)
        .timeout(BUDGET)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|cause| Error::Market(format!("registry 不可达：{cause}")))?;
    if !response.status().is_success() {
        return Err(Error::Market(format!(
            "registry 返回 {}（{endpoint}）",
            response.status()
        )));
    }
    response
        .json()
        .await
        .map_err(|cause| Error::Market(format!("registry 响应不是 JSON：{cause}")))
}

fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// npm 包名（含 scope）。
fn is_package_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 214 {
        return false;
    }
    match name.strip_prefix('@') {
        Some(rest) => match rest.split_once('/') {
            Some((scope, pkg)) => is_segment(scope) && is_segment(pkg),
            None => false,
        },
        None => is_segment(name),
    }
}

fn is_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.' | '~')
        })
}

fn string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{compatibility, detail_from_manifest, is_package_name, urlencode, Compatibility};

    #[test]
    fn 兼容判定对rc钉版宽松() {
        let compatible = compatibility(&serde_json::json!({
            "peerDependencies": { "@deepseek-ai/dsh": "^0.1.2" }
        }));
        assert!(matches!(compatible, Compatibility::Compatible { .. }));

        let declared_rc = compatibility(&serde_json::json!({
            "peerDependencies": { "@deepseek-ai/dsh": "^0.1.5-rc.1" }
        }));
        assert!(matches!(declared_rc, Compatibility::Compatible { .. }));

        let unknown = compatibility(&serde_json::json!({ "name": "x" }));
        assert!(matches!(unknown, Compatibility::Unknown));

        let incompatible = compatibility(&serde_json::json!({
            "peerDependencies": { "@deepseek-ai/dsh": "^9.0.0" }
        }));
        assert!(matches!(incompatible, Compatibility::Incompatible { .. }));
    }

    #[test]
    fn 详情提取bundle与门槛字段() {
        let detail = detail_from_manifest(
            "sample",
            "https://registry.npmmirror.com",
            &serde_json::json!({
                "name": "sample",
                "version": "1.2.3",
                "license": "MIT",
                "deprecated": "用新包",
                "scripts": { "postinstall": "node setup.js" },
                "dsh": { "bundle": { "patch": [] } },
                "dist": { "unpackedSize": 4096 }
            }),
        );
        assert!(detail.bundle);
        assert_eq!(detail.install_spec, "sample@1.2.3");
        assert_eq!(detail.lifecycle_scripts, ["postinstall"]);
        assert_eq!(detail.unpacked_bytes, Some(4096));
        assert_eq!(detail.deprecated.as_deref(), Some("用新包"));
    }

    #[test]
    fn 包名校验覆盖scope() {
        assert!(is_package_name("dshmarket"));
        assert!(is_package_name("@xmanrui/dsh-im"));
        assert!(!is_package_name(""));
        assert!(!is_package_name("@only-scope"));
        assert!(!is_package_name("Bad/Name"));
    }

    #[test]
    fn 搜索词按查询串转义() {
        assert_eq!(urlencode("dsh bundle"), "dsh%20bundle");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
    }
}

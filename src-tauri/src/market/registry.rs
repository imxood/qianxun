//! 安装预检：读 npm registry 的包 manifest 做硬门槛（弃用 / 不兼容 / 版本一致）。
//!
//! 浏览与搜索在前端：webview 的 fetch 走系统代理、npmmirror 全端点带 CORS，
//! 目录（dsh-plugin-catalog）的 tar.gz 解包也由前端完成。Rust 只保留变更面
//! 的信任边界——安装前在这里复核 registry 最新状态，不信任前端传来的结论。

use std::time::Duration;

use crate::error::{Error, Result};
use crate::harness::install::PINNED_VERSION;

/// 宽裕些：镜像可能在美国之外的慢链路上。
const BUDGET: Duration = Duration::from_secs(20);

/// 一个包的发布详情（安装预检的输入）。
#[derive(Clone, Debug)]
pub struct Detail {
    pub version: String,
    /// 声明了 `dsh.bundle.patch` = 插件；否则只是提及 DSH 的普通包。
    #[allow(dead_code)]
    pub bundle: bool,
    /// 与千寻锁定的 DSH 版本的兼容结论。
    pub compatibility: Compatibility,
    #[allow(dead_code)]
    pub lifecycle_scripts: Vec<String>,
    /// 发布者标记的弃用说明；存在即拒绝安装。
    pub deprecated: Option<String>,
}

/// 兼容门槛的结论：放行，或拦截并给出理由。
/// （「未声明兼容范围」「范围无法解析」都按放行处理——门槛只拦确凿的不兼容；
/// 展示层的兼容结论由前端计算，这里不重复。）
#[derive(Clone, Debug)]
pub enum Compatibility {
    Pass,
    /// reason 已包含「它要求 X」，可直接进错误消息。
    Block {
        reason: String,
    },
}

/// 读一个包的最新发布 manifest。
pub async fn detail(registry: &str, name: &str) -> Result<Detail> {
    if !is_package_name(name) {
        return Err(Error::Market(format!("{name} 不是合法的包名")));
    }
    let base = registry.trim_end_matches('/');
    let manifest = fetch_json(&format!("{base}/{name}/latest")).await?;
    Ok(detail_from_manifest(name, &manifest))
}

/// 安装前的硬门槛：弃用与不兼容拒绝安装。
pub fn validate(detail: &Detail) -> Result<()> {
    if let Some(reason) = &detail.deprecated {
        return Err(Error::Market(format!("该包已弃用：{reason}")));
    }
    if let Compatibility::Block { reason, .. } = &detail.compatibility {
        return Err(Error::Market(format!(
            "与千寻适配的 DSH {PINNED_VERSION} 不兼容：{reason}"
        )));
    }
    Ok(())
}

fn detail_from_manifest(name: &str, manifest: &serde_json::Value) -> Detail {
    let version = string(manifest, "version").unwrap_or_else(|| name.to_owned());
    let lifecycle_scripts = ["preinstall", "install", "postinstall", "prepare"]
        .into_iter()
        .filter(|script| manifest.pointer(&format!("/scripts/{script}")).is_some())
        .map(str::to_owned)
        .collect();
    Detail {
        bundle: manifest.pointer("/dsh/bundle/patch").is_some(),
        version,
        compatibility: compatibility(manifest),
        lifecycle_scripts,
        deprecated: manifest.get("deprecated").and_then(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        }),
    }
}

/// 兼容判定（宽松）：rc 钉版的生态现实是包声明 `^0.1.x` 这类宽范围——
/// 直接按 semver 预发布规则比对会大量误杀，所以对当前版本同时试
/// 原样与去预发布两种形态，任一命中即放行；未声明、解析不了都放行——
/// 门槛只拦确凿的不兼容。
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
        return Compatibility::Pass;
    };
    let Ok(parsed) = semver::VersionReq::parse(&requirement) else {
        return Compatibility::Pass;
    };
    let Ok(current) = semver::Version::parse(PINNED_VERSION) else {
        return Compatibility::Pass;
    };
    let release = semver::Version {
        pre: semver::Prerelease::EMPTY,
        ..current.clone()
    };
    if parsed.matches(&current) || parsed.matches(&release) {
        Compatibility::Pass
    } else {
        Compatibility::Block {
            reason: format!("它要求 {requirement}"),
        }
    }
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
    use super::{compatibility, detail_from_manifest, is_package_name, validate, Compatibility};

    #[test]
    fn 兼容判定对rc钉版宽松() {
        // 宽范围与显式 rc 声明都放行；只拦确凿的不兼容。
        let compatible = compatibility(&serde_json::json!({
            "peerDependencies": { "@deepseek-ai/dsh": "^0.1.2" }
        }));
        assert!(matches!(compatible, Compatibility::Pass));

        let declared_rc = compatibility(&serde_json::json!({
            "peerDependencies": { "@deepseek-ai/dsh": "^0.1.5-rc.1" }
        }));
        assert!(matches!(declared_rc, Compatibility::Pass));

        let undeclared = compatibility(&serde_json::json!({ "name": "x" }));
        assert!(matches!(undeclared, Compatibility::Pass));

        let incompatible = compatibility(&serde_json::json!({
            "peerDependencies": { "@deepseek-ai/dsh": "^9.0.0" }
        }));
        assert!(matches!(incompatible, Compatibility::Block { .. }));
    }

    #[test]
    fn 详情提取门槛字段且validate拦截() {
        let detail = detail_from_manifest(
            "sample",
            &serde_json::json!({
                "name": "sample",
                "version": "1.2.3",
                "deprecated": "用新包",
                "scripts": { "postinstall": "node setup.js" },
                "dsh": { "bundle": { "patch": [] } }
            }),
        );
        assert!(detail.bundle);
        assert_eq!(detail.version, "1.2.3");
        assert_eq!(detail.lifecycle_scripts, ["postinstall"]);
        assert!(validate(&detail).is_err());

        let clean = detail_from_manifest(
            "clean",
            &serde_json::json!({ "name": "clean", "version": "0.1.0" }),
        );
        assert!(validate(&clean).is_ok());
    }

    #[test]
    fn 包名校验覆盖scope() {
        assert!(is_package_name("dshmarket"));
        assert!(is_package_name("@xmanrui/dsh-im"));
        assert!(!is_package_name(""));
        assert!(!is_package_name("@only-scope"));
        assert!(!is_package_name("Bad/Name"));
    }
}

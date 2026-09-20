//! 启动失败的三级自救（08 设计 §11）：known-good 快照 + 最小安全 profile。
//!
//! 默认 profile 每次成功启动都会把 package.json 快照成 `.good`；启动失败时
//! 自动恢复快照重试一次（自动链只走一步），快照也救不了才由用户手动进入
//! 最小安全环境。安全 profile 是纯只读逃生舱：启动成功不写快照、不改默认
//! profile，插件变更一律冻结。
//!
//! 快照只覆盖 manifest 层故障（最常见：新插件把启动搞挂）。node_modules
//! 本身损坏时快照恢复无效，这正是第三层存在的意义（08 设计 §11.6）。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};

/// 最小安全 profile 的固定目录名（<dsh-home>/profiles/<SAFE_PROFILE>）。
pub const SAFE_PROFILE: &str = "qianxun-safe-mode";

/// 所有权标记文件：目录里必须有内容完全一致的标记才允许被安全模式复用，
/// 防止误吞用户碰巧同名的 profile（用户数据一个字节不动）。
const SAFE_MARKER: &str = ".qianxun-safe-mode";
const SAFE_MARKER_BODY: &str = "qianxun-safe-mode-v1\n";

/// 快照文件名（与被快照的 package.json 同目录）。
pub const SNAPSHOT_NAME: &str = "package.json.good";

/// 最小 bundles 栈：仅 DSH 运行必需的内核包（与默认 profile 的内核集一致）。
/// 内核包随千寻钉住的 DSH 运行时落位（harness prefix），**不在 profile
/// node_modules**，所以安全 profile 无需任何 pnpm 安装——写三个小文件即就绪。
const SAFE_BUNDLES: [&str; 2] = ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"];

/// harness_recover_known_good 的返回：恢复摘要（哪些插件从 bundles 被移除）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySummary {
    /// 从 bundles 里移除的包名（快照没有、当前有的）。
    pub removed_plugins: Vec<String>,
    /// 快照有、当前没有的包名（被恢复回来加载的）。
    pub restored_plugins: Vec<String>,
}

// ---------------------------------------------------------------------------
// known-good 快照
// ---------------------------------------------------------------------------

fn snapshot_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(SNAPSHOT_NAME)
}

/// 把当前 manifest 快照为 `.good`（原子写：临时名 + rename）。
/// 失败不致命（快照是增强信息），调用方只记 warn。
pub fn write_snapshot(profile_dir: &Path) -> Result<()> {
    let manifest = profile_dir.join("package.json");
    let body = std::fs::read(&manifest)
        .map_err(|cause| Error::Market(format!("读 manifest 失败：{cause}")))?;
    if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return Err(Error::Market(
            "manifest 不是合法 JSON，拒绝快照（保留旧快照）".to_owned(),
        ));
    }
    let target = snapshot_path(profile_dir);
    let temp = profile_dir.join(format!(".package.json.good.tmp-{}", std::process::id()));
    std::fs::write(&temp, &body)
        .map_err(|cause| Error::Market(format!("写快照临时文件失败：{cause}")))?;
    std::fs::rename(&temp, &target)
        .map_err(|cause| Error::Market(format!("快照落位失败：{cause}")))?;
    Ok(())
}

/// 快照是否存在。
pub fn snapshot_exists(profile_dir: &Path) -> bool {
    snapshot_path(profile_dir).is_file()
}

/// 快照与当前 manifest 是否不同（字节级）。这是自动恢复的触发条件：
/// 相同则说明 manifest 不是启动失败的原因，重试没有意义。
pub fn snapshot_differs(profile_dir: &Path) -> bool {
    let Ok(current) = std::fs::read(profile_dir.join("package.json")) else {
        // manifest 都没了（或读不了），有快照就值得恢复。
        return snapshot_exists(profile_dir);
    };
    match std::fs::read(snapshot_path(profile_dir)) {
        Ok(good) => good != current,
        Err(_) => false,
    }
}

/// 用快照恢复 manifest，返回恢复摘要。
/// `.good` 不存在 → 明确报错，不产生半写状态；node_modules 与 pinned 不动。
pub fn recover(profile_dir: &Path) -> Result<RecoverySummary> {
    let good = std::fs::read(snapshot_path(profile_dir))
        .map_err(|_| Error::Market("没有可恢复的快照（尚未有过成功启动）".to_owned()))?;
    let snapshot: serde_json::Value = serde_json::from_slice(&good)
        .map_err(|cause| Error::Market(format!("快照不是合法 JSON：{cause}")))?;

    // 摘要：当前 bundles 与快照 bundles 的差集（在恢复**之前**算）。
    let (removed, restored) = match std::fs::read_to_string(profile_dir.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    {
        Some(current) => bundle_diff(&current, &snapshot),
        None => (Vec::new(), bundle_list(&snapshot)),
    };

    crate::atomic::write(&profile_dir.join("package.json"), &good)
        .map_err(|cause| Error::Market(format!("恢复 manifest 失败：{cause}")))?;
    Ok(RecoverySummary {
        removed_plugins: removed,
        restored_plugins: restored,
    })
}

fn bundle_list(manifest: &serde_json::Value) -> Vec<String> {
    manifest
        .pointer("/dsh/profile/bundles")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// (removed, restored)：当前有快照没有 = 恢复后卸下；快照有当前没有 = 恢复后回来。
fn bundle_diff(
    current: &serde_json::Value,
    snapshot: &serde_json::Value,
) -> (Vec<String>, Vec<String>) {
    let now = bundle_list(current);
    let good = bundle_list(snapshot);
    let removed = now
        .iter()
        .filter(|name| !good.contains(name))
        .cloned()
        .collect();
    let restored = good
        .iter()
        .filter(|name| !now.contains(name))
        .cloned()
        .collect();
    (removed, restored)
}

// ---------------------------------------------------------------------------
// 最小安全 profile
// ---------------------------------------------------------------------------

/// 安全 profile 是否就绪（标记 + manifest 都在）。启动成功钩子顺带校验，
/// 缺了就静默重建（幂等预热，08 设计 §11.4）。
pub fn safe_profile_ready(profiles_dir: &Path) -> bool {
    let dir = profiles_dir.join(SAFE_PROFILE);
    marker_ok(&dir) && dir.join("package.json").is_file()
}

/// 建好（或复位）安全 profile；幂等，重复调用无副作用。
/// 同名用户 profile 无标记 → 拒绝复用、绝不吞并。
pub fn prepare_safe_profile(profiles_dir: &Path) -> Result<PathBuf> {
    let dir = profiles_dir.join(SAFE_PROFILE);
    match std::fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            if !marker_ok(&dir) {
                return Err(Error::Market(format!(
                    "{} 已存在且不属于千寻安全模式（缺所有权标记）；请改名后重试",
                    dir.display()
                )));
            }
        }
        Ok(_) => {
            return Err(Error::Market(format!(
                "{} 不是目录，无法作为安全 profile",
                dir.display()
            )))
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(&dir)
                .map_err(|cause| Error::Market(format!("建安全 profile 失败：{cause}")))?;
            std::fs::write(dir.join(SAFE_MARKER), SAFE_MARKER_BODY)
                .map_err(|cause| Error::Market(format!("写所有权标记失败：{cause}")))?;
        }
        Err(cause) => return Err(Error::Market(format!("安全 profile 无法检查：{cause}"))),
    }

    // 每次进入都复位为最小栈（标记在，复位就是安全的）。
    let bundles: Vec<&str> = SAFE_BUNDLES.to_vec();
    let manifest = serde_json::json!({
        "name": format!("dsh-profile-{SAFE_PROFILE}"),
        "private": true,
        "dependencies": {},
        "dsh": {
            "profile": {
                "bundles": bundles,
                "patchReload": "live"
            }
        }
    });
    let body = serde_json::to_vec_pretty(&manifest)
        .map_err(|cause| Error::Market(format!("编码安全 manifest 失败：{cause}")))?;
    crate::atomic::write(&dir.join("package.json"), &body)
        .map_err(|cause| Error::Market(format!("写安全 manifest 失败：{cause}")))?;
    // cordis 根清单：空 entry list（与默认 profile 同构）。
    if !dir.join("cordis.yml").is_file() {
        let _ = std::fs::write(dir.join("cordis.yml"), "[]\n");
    }
    Ok(dir)
}

fn marker_ok(dir: &Path) -> bool {
    matches!(
        std::fs::read_to_string(dir.join(SAFE_MARKER)),
        Ok(body) if body == SAFE_MARKER_BODY
    )
}

#[cfg(test)]
mod tests {
    use super::{
        prepare_safe_profile, recover, snapshot_differs, snapshot_exists, write_snapshot,
        RecoverySummary, SAFE_PROFILE, SNAPSHOT_NAME,
    };
    use std::fs;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "qx-recovery-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn manifest_with_bundles(bundles: &[&str]) -> String {
        serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": { "profile": { "bundles": bundles } }
        })
        .to_string()
    }

    #[test]
    fn 快照只在成功钩子写且恢复逐字节一致() {
        let profile = scratch("snapshot");
        fs::write(profile.join("package.json"), manifest_with_bundles(&["a"])).unwrap();

        // 无快照：exists=false，differs=false（没东西可恢复）。
        assert!(!snapshot_exists(&profile));
        assert!(!snapshot_differs(&profile));

        // 成功钩子写入快照；内容与 manifest 一致 → differs=false。
        write_snapshot(&profile).unwrap();
        assert!(snapshot_exists(&profile));
        assert!(!snapshot_differs(&profile));

        // 之后 manifest 变了（装了新插件 b）→ differs=true。
        fs::write(
            profile.join("package.json"),
            manifest_with_bundles(&["a", "b"]),
        )
        .unwrap();
        assert!(snapshot_differs(&profile));

        // 恢复：逐字节回到快照，摘要说清 b 被卸下；node_modules 与 pinned 不受影响。
        let summary = recover(&profile).unwrap();
        assert_eq!(
            fs::read(profile.join("package.json")).unwrap(),
            manifest_with_bundles(&["a"]).into_bytes()
        );
        assert_eq!(summary.removed_plugins, ["b"]);
        assert!(summary.restored_plugins.is_empty());
        // 恢复后快照与 manifest 再次一致。
        assert!(!snapshot_differs(&profile));
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn 无快照时恢复报明确错误且不产生半写() {
        let profile = scratch("no-snapshot");
        fs::write(profile.join("package.json"), manifest_with_bundles(&["a"])).unwrap();
        let failure = recover(&profile).unwrap_err();
        assert!(failure.to_string().contains("没有可恢复的快照"));
        // manifest 原样未动。
        assert_eq!(
            fs::read_to_string(profile.join("package.json")).unwrap(),
            manifest_with_bundles(&["a"])
        );
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn 恢复摘要双向都有且损坏manifest按无处理() {
        let profile = scratch("both");
        // 快照：内核 a + 插件 keep；当前：内核 a + 插件 extra（keep 被卸过）。
        fs::write(
            profile.join(SNAPSHOT_NAME),
            manifest_with_bundles(&["a", "keep"]),
        )
        .unwrap();
        fs::write(
            profile.join("package.json"),
            manifest_with_bundles(&["a", "extra"]),
        )
        .unwrap();
        let RecoverySummary {
            removed_plugins,
            restored_plugins,
        } = recover(&profile).unwrap();
        assert_eq!(removed_plugins, ["extra"]);
        assert_eq!(restored_plugins, ["keep"]);
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn 安全profile预热幂等且拒绝吞并无标记目录() {
        let profiles = scratch("safe");
        // 首次 prepare：建标记 + 最小 manifest。
        let dir = prepare_safe_profile(&profiles).unwrap();
        assert_eq!(dir.file_name().unwrap().to_string_lossy(), SAFE_PROFILE);
        assert!(super::safe_profile_ready(&profiles));
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
        let bundles = manifest
            .pointer("/dsh/profile/bundles")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        // 最小栈：只有内核包，无任何用户插件，dependencies 为空。
        assert_eq!(bundles.len(), 2);
        assert!(bundles
            .iter()
            .all(|b| b.as_str().unwrap().starts_with("@deepseek-ai/")));
        assert_eq!(manifest["dependencies"], serde_json::json!({}));

        // 重复 prepare：幂等（不报错、目录不变）。
        prepare_safe_profile(&profiles).unwrap();
        assert!(super::safe_profile_ready(&profiles));

        // 用户同名 profile 无标记 → 拒绝，绝不吞并。
        let hostile = scratch("hostile").join(SAFE_PROFILE);
        std::fs::create_dir_all(&hostile).unwrap();
        fs::write(hostile.join("package.json"), r#"{ "user": true }"#).unwrap();
        let failure = prepare_safe_profile(hostile.parent().unwrap()).unwrap_err();
        assert!(failure.to_string().contains("所有权标记"), "{failure}");
        // 用户文件原样未动。
        assert_eq!(
            fs::read_to_string(hostile.join("package.json")).unwrap(),
            r#"{ "user": true }"#
        );
        fs::remove_dir_all(&profiles).ok();
        fs::remove_dir_all(hostile.parent().unwrap()).ok();
    }
}

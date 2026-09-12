//! 插件市场域：搜索、安装、卸载、已装清单。
//!
//! 插件 = 装进 DSH profile 的 npm 包 + `dsh.profile.bundles` 里的一行。
//! 搜索/详情在 registry.rs（只读网络），本文件负责变更：
//! 用千寻自带的 pnpm 往 profile 装精确版本，再把包名并进 bundles 数组。
//! 变更即时落盘，DSH 下次启动生效；输出行进 supervisor 日志（环境页日志面）。

pub mod registry;

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::{self, install, supervisor::Stream, DEFAULT_PROFILE};
use crate::paths;
use crate::settings::Settings;

const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

/// profile 模板自带的内核包：不在市场里管理、不可卸载。
const BUILTIN_PREFIX: &str = "@deepseek-ai/";

/// 已装清单里的一项（profile package.json 事实）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledBundle {
    pub name: String,
    /// profile package.json 记录的依赖范围；内核包为空串。
    pub spec: String,
    /// 在 bundles 数组里 = 启动加载。
    pub active: bool,
    /// node_modules 下已落盘。
    pub deployed: bool,
    pub builtin: bool,
}

/// 搜索市场（透传 registry 搜索端点）。
#[tauri::command]
pub async fn market_search(app: AppHandle, query: String) -> Result<Vec<registry::Listing>> {
    let registry = registry_base(&app)?;
    registry::search(&registry, &query).await
}

/// 读一个包的发布详情。
#[tauri::command]
pub async fn market_detail(app: AppHandle, name: String) -> Result<registry::Detail> {
    let registry = registry_base(&app)?;
    registry::detail(&registry, &name).await
}

/// 已装清单：profile manifest 的 dependencies × bundles × node_modules。
#[tauri::command]
pub fn market_installed(app: AppHandle) -> Result<Vec<InstalledBundle>> {
    let settings = crate::settings_snapshot(&app)?;
    let Ok(profile) = profile_dir(&app, &settings) else {
        return Ok(Vec::new());
    };
    Ok(installed(&profile))
}

/// 安装精确版本并加入 bundles。输出逐行进 supervisor 日志。
#[tauri::command]
pub async fn market_install(app: AppHandle, name: String, version: String) -> Result<()> {
    let settings = crate::settings_snapshot(&app)?;
    let registry = settings.mirrors.registry_url();
    let profile = profile_dir(&app, &settings)?;
    // 安装前用详情做硬门槛（弃用/不兼容），不让 registry 之外的状态混进 profile。
    let detail = registry::detail(&registry, &name).await?;
    if detail.version != version {
        return Err(Error::Market(format!(
            "registry 最新版本是 {}，与请求的 {version} 不一致；刷新后重试",
            detail.version
        )));
    }
    registry::validate(&detail)?;

    let supervisor = app.state::<crate::AppState>().harness.supervisor.clone();
    supervisor.note(Stream::Stdout, format!("安装插件 {name}@{version}"));
    ensure_build_allowlist(&profile, &supervisor)?;
    run_pnpm(
        &app,
        &settings,
        &supervisor,
        ["add".into(), format!("{name}@{version}")],
        "插件安装",
    )
    .await?;
    mutate_bundles(&profile, |bundles| {
        if !bundles.iter().any(|entry| entry == &name) {
            bundles.push(name.clone());
        }
    })?;
    supervisor.note(
        Stream::Stdout,
        format!("{name}@{version} 已装好，DSH 下次启动生效"),
    );
    Ok(())
}

/// 卸载：移出 bundles + pnpm rm。
#[tauri::command]
pub async fn market_remove(app: AppHandle, name: String) -> Result<()> {
    let settings = crate::settings_snapshot(&app)?;
    let profile = profile_dir(&app, &settings)?;
    if name.starts_with(BUILTIN_PREFIX) {
        return Err(Error::Market("内核组件不可卸载".to_owned()));
    }
    let supervisor = app.state::<crate::AppState>().harness.supervisor.clone();
    supervisor.note(Stream::Stdout, format!("卸载插件 {name}"));
    mutate_bundles(&profile, |bundles| bundles.retain(|entry| entry != &name))?;
    let outcome = run_pnpm(
        &app,
        &settings,
        &supervisor,
        ["remove".into(), name.clone()],
        "插件卸载",
    )
    .await;
    supervisor.note(Stream::Stdout, format!("{name} 已卸载，DSH 下次启动生效"));
    outcome
}

// ---- 内部 ----

fn registry_base(app: &AppHandle) -> Result<String> {
    Ok(crate::settings_snapshot(app)?.mirrors.registry_url())
}

fn profile_dir(app: &AppHandle, settings: &Settings) -> Result<std::path::PathBuf> {
    let dir = harness::dsh_home(app, settings)
        .join("profiles")
        .join(DEFAULT_PROFILE);
    if !dir.is_dir() {
        return Err(Error::Market(format!(
            "DSH profile 不存在：{}。请先安装并启动一次 DSH。",
            dir.display()
        )));
    }
    Ok(dir)
}

/// 从 profile manifest 读已装事实；profile 还没有 package.json 时为空。
fn installed(profile: &std::path::Path) -> Vec<InstalledBundle> {
    let Ok(text) = std::fs::read_to_string(profile.join("package.json")) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let dependencies = manifest
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    let bundles = bundles_of(&manifest);
    let node_modules = profile.join("node_modules");

    let mut names: Vec<String> = bundles.iter().chain(dependencies.keys()).cloned().collect();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let builtin = name.starts_with(BUILTIN_PREFIX);
            let spec = dependencies
                .get(&name)
                .and_then(|value| value.as_str())
                .unwrap_or("");
            InstalledBundle {
                builtin,
                active: bundles.iter().any(|entry| entry == &name),
                // 内核包随千寻钉住的 DSH 运行时落位（harness prefix，不在
                // profile node_modules）——运行时能跑它就在；只有用户装的
                // 包才按 profile node_modules 判定落盘。
                deployed: builtin || node_modules.join(&name).exists(),
                spec: spec.to_owned(),
                name,
            }
        })
        .collect()
}

fn bundles_of(manifest: &serde_json::Value) -> Vec<String> {
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

/// 原子改写 profile manifest 里的 bundles 数组（其余字段原样保留）。
fn mutate_bundles(profile: &std::path::Path, change: impl FnOnce(&mut Vec<String>)) -> Result<()> {
    let manifest_path = profile.join("package.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|cause| Error::Market(format!("读 profile manifest 失败：{cause}")))?;
    let mut manifest: serde_json::Value = serde_json::from_str(&text)
        .map_err(|cause| Error::Market(format!("profile manifest 不是合法 JSON：{cause}")))?;
    let mut bundles = bundles_of(&manifest);
    change(&mut bundles);
    if let Some(object) = manifest.as_object_mut() {
        let profile_node = object.entry("dsh").or_insert_with(|| serde_json::json!({}));
        let profile_object = profile_node
            .as_object_mut()
            .ok_or_else(|| Error::Market("profile manifest 的 dsh 字段不是对象".to_owned()))?;
        let profile_node = profile_object
            .entry("profile")
            .or_insert_with(|| serde_json::json!({}));
        let profile_object = profile_node.as_object_mut().ok_or_else(|| {
            Error::Market("profile manifest 的 dsh.profile 字段不是对象".to_owned())
        })?;
        profile_object.insert(
            "bundles".to_owned(),
            serde_json::Value::Array(bundles.into_iter().map(serde_json::Value::String).collect()),
        );
    }
    let body = serde_json::to_vec_pretty(&manifest)
        .map_err(|cause| Error::Market(format!("编码 profile manifest 失败：{cause}")))?;
    crate::atomic::write(&manifest_path, &body)
        .map_err(|cause| Error::Market(format!("写 profile manifest 失败：{cause}")))
}

/// profile 的构建脚本白名单：与 DSH 运行时同一份已知原生依赖清单。
/// 没有它 pnpm 静默跳过原生构建，插件运行时才炸。
fn ensure_build_allowlist(
    profile: &std::path::Path,
    supervisor: &std::sync::Arc<harness::supervisor::Supervisor>,
) -> Result<()> {
    let path = profile.join("pnpm-workspace.yaml");
    if path.is_file() {
        return Ok(());
    }
    std::fs::write(&path, install::PNPM_WORKSPACE_YAML)
        .map_err(|cause| Error::Market(format!("写 pnpm 构建白名单失败：{cause}")))?;
    supervisor.note(
        Stream::Stdout,
        "已写入 profile 构建白名单（pnpm-workspace.yaml）".to_owned(),
    );
    Ok(())
}

/// 在 profile 目录里跑一条 pnpm（千寻自带的那份），输出进 supervisor 日志。
async fn run_pnpm(
    app: &AppHandle,
    settings: &Settings,
    supervisor: &std::sync::Arc<harness::supervisor::Supervisor>,
    args: impl IntoIterator<Item = String>,
    label: &'static str,
) -> Result<()> {
    // Node + npm 必须可用（安装器探测过的同一套规则）；pnpm 用千寻装好的。
    let environment = harness::environment(app, settings);
    let node = environment.node.clone().ok_or(Error::NoNodeRuntime {
        minimum: node_runtime::MINIMUM_SUPPORTED,
    })?;
    let npm_cli = install::npm_cli(&node.path).ok_or(Error::NpmMissing)?;
    let plan = install::InstallPlan {
        node: node.path.clone(),
        npm_cli,
        target: paths::harness_dir(app)?,
        pnpm_tool_dir: paths::pnpm_tool_dir(app)?,
        spec: install::install_spec(),
        registry: settings.mirrors.registry_url(),
    };
    if !plan.pnpm_cli().is_file() {
        return Err(Error::Market(
            "pnpm 工具缺失：先到环境页完成 DSH 安装".to_owned(),
        ));
    }

    let mut path_entries: Vec<_> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    if let Some(dir) = node.path.parent() {
        path_entries.insert(0, dir.to_path_buf());
    }

    let mut command = tokio::process::Command::new(&plan.node);
    command.arg(plan.pnpm_cli());
    for arg in args {
        command.arg(&arg);
    }
    command
        .arg("--dir")
        .arg(profile_dir(app, settings)?)
        .arg("--registry")
        .arg(plan.registry.clone())
        .arg("--reporter=append-only")
        .arg("--config.auto-install-peers=true")
        .current_dir(&plan.target)
        .env(
            "PATH",
            std::env::join_paths(path_entries)
                .map_err(|cause| Error::Market(format!("PATH 组装失败：{cause}")))?,
        )
        .env("npm_config_update_notifier", "false")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    install::hide_console_window(&mut command);

    let reporter = supervisor.clone();
    install::run_with_limits(
        command,
        move |stream, line| reporter.note(stream, line),
        label,
        IDLE_TIMEOUT,
        TOTAL_TIMEOUT,
        DRAIN_TIMEOUT,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{bundles_of, installed, mutate_bundles};
    use std::fs;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "qx-market-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("node_modules/dshmarket")).unwrap();
        root
    }

    #[test]
    fn 已装清单区分内核与用户包() {
        let profile = scratch("installed");
        fs::write(
            profile.join("package.json"),
            r#"{
                "name": "dsh-profile-web",
                "dependencies": { "dshmarket": "^1.44.0" },
                "dsh": { "profile": { "bundles": [
                    "@deepseek-ai/dsh-base",
                    "dshmarket"
                ] } }
            }"#,
        )
        .unwrap();

        let list = installed(&profile);
        assert_eq!(list.len(), 2);
        let builtin = list
            .iter()
            .find(|entry| entry.name == "@deepseek-ai/dsh-base")
            .unwrap();
        // 内核包随运行时落位，不按 profile node_modules 判定。
        assert!(builtin.builtin && builtin.active && builtin.deployed);
        assert_eq!(builtin.spec, "");
        let market = list.iter().find(|entry| entry.name == "dshmarket").unwrap();
        assert!(!market.builtin && market.active && market.deployed);
        assert_eq!(market.spec, "^1.44.0");
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn bundles维护可增删且保留其他字段() {
        let profile = scratch("bundles");
        fs::write(
            profile.join("package.json"),
            r#"{ "dependencies": {}, "dsh": { "profile": { "bundles": ["a"] }, "patchReload": "live" } }"#,
        )
        .unwrap();

        mutate_bundles(&profile, |bundles| bundles.push("b".to_owned())).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(profile.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(bundles_of(&manifest), ["a", "b"]);
        // 无关字段不丢。
        assert_eq!(
            manifest
                .pointer("/dsh/patchReload")
                .and_then(|v| v.as_str()),
            Some("live")
        );

        mutate_bundles(&profile, |bundles| bundles.retain(|name| name != "a")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(profile.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(bundles_of(&manifest), ["b"]);
        fs::remove_dir_all(&profile).ok();
    }
}

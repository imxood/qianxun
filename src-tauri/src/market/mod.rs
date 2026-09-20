//! 插件市场域：安装、卸载、已装清单（变更面）。
//!
//! 浏览与搜索在前端：webview fetch 走系统代理，npmmirror 全端点带 CORS，
//! 推荐目录（dsh-plugin-catalog）的 tar.gz 解包由前端完成，Rust 不掺和。
//! 本文件是唯一的变更面：安装前用 registry 详情复核（registry.rs，
//! 不信任前端的结论），pnpm 精确版本装入 profile，包名并进 bundles 数组。
//! 变更即时落盘，DSH 下次启动生效；输出行进 supervisor 日志（环境页日志面）。

pub mod registry;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::{self, install, supervisor::Stream, DEFAULT_PROFILE};
use crate::paths;
use crate::settings::{PinnedPlugin, Settings};

const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

/// profile 模板自带的内核包：不在市场里管理、不可卸载。
pub const BUILTIN_PREFIX: &str = "@deepseek-ai/";

/// market 域并发位（08 设计 §7）：挡住「同步进行中又点单个安装」。
/// 只挡本域，不做跨窗队列；进程级即可（千寻单实例，ADR 见 lib.rs）。
static MARKET_BUSY: AtomicBool = AtomicBool::new(false);

fn claim_busy() -> Result<BusyGuard> {
    if MARKET_BUSY.swap(true, Ordering::SeqCst) {
        return Err(Error::Market(
            "已有插件变更在进行中，请等它完成再操作".to_owned(),
        ));
    }
    Ok(BusyGuard)
}

struct BusyGuard;

impl Drop for BusyGuard {
    fn drop(&mut self) {
        MARKET_BUSY.store(false, Ordering::SeqCst);
    }
}

/// 安全模式激活时冻结一切插件变更（08 设计 §11.3-4）。
fn ensure_not_safe_mode(app: &AppHandle) -> Result<()> {
    if harness::commands::safe_mode_active(app) {
        return Err(Error::Market(
            "安全模式中不进行插件变更：请先恢复默认 profile 再操作".to_owned(),
        ));
    }
    Ok(())
}

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

/// 已装清单：profile manifest 的 dependencies × bundles × node_modules。
#[tauri::command]
pub fn market_installed(app: AppHandle) -> Result<Vec<InstalledBundle>> {
    let settings = crate::settings_snapshot(&app)?;
    let Ok(profile) = profile_dir(&app, &settings) else {
        return Ok(Vec::new());
    };
    Ok(installed(&profile))
}

/// 安装精确版本并加入 bundles。每一步与 pnpm 输出都进 supervisor 日志。
#[tauri::command]
pub async fn market_install(app: AppHandle, name: String, version: String) -> Result<()> {
    let _busy = claim_busy()?;
    ensure_not_safe_mode(&app)?;
    let settings = crate::settings_snapshot(&app)?;
    let registry = settings.mirrors.registry_url();
    let profile = profile_dir(&app, &settings)?;
    let supervisor = app.state::<crate::AppState>().harness.supervisor.clone();
    supervisor.note(
        Stream::Stdout,
        format!("[插件] 安装 {name}@{version}（pnpm add → profile）"),
    );
    // 安装前按请求的版本做硬门槛（存在性 / 弃用 / 不兼容），不让 registry
    // 之外的状态混进 profile。刻意不比对 dist-tags.latest：前端列表陈旧
    // 或用户刻意装旧版都是合理场景，pnpm 精确版本安装本来就支持。
    let detail = match registry::detail(&registry, &name, &version).await {
        Ok(detail) => detail,
        Err(failure) => {
            supervisor.note(Stream::Stderr, format!("[插件] 安装失败：{failure}"));
            return Err(failure);
        }
    };
    if let Err(failure) = registry::validate(&detail) {
        supervisor.note(Stream::Stderr, format!("[插件] 安装失败：{failure}"));
        return Err(failure);
    }

    if let Err(failure) = ensure_build_allowlist(&profile, &supervisor) {
        supervisor.note(Stream::Stderr, format!("[插件] 安装失败：{failure}"));
        return Err(failure);
    }
    let outcome = run_pnpm(
        &app,
        &settings,
        &supervisor,
        ["add".into(), format!("{name}@{version}")],
        "插件安装",
        true,
    )
    .await;
    if let Err(failure) = &outcome {
        supervisor.note(Stream::Stderr, format!("[插件] 安装失败：{failure}"));
        return outcome;
    }
    if let Err(failure) = mutate_bundles(&profile, |bundles| {
        if !bundles.iter().any(|entry| entry == &name) {
            bundles.push(name.clone());
        }
    }) {
        supervisor.note(Stream::Stderr, format!("[插件] 安装失败：{failure}"));
        return Err(failure);
    }
    supervisor.note(
        Stream::Stdout,
        "[插件] 已写入装配清单（dsh.profile.bundles）".to_owned(),
    );
    // 反向校验（08 设计 §3）：pnpm 退出码 0 不够，node_modules 里得真有
    // 这个版本才算装成功。不一致时报错但保留现场（bundles 已含该包），
    // 让用户用卸载清理——不做自动回滚。
    if let Some(actual) = deployed_version(&profile, &name) {
        if actual != detail.version {
            let failure = Error::Market(format!(
                "安装完成但落盘校验失败：期望 {name}@{}，实际 {actual}；请在已安装列表卸载后重试",
                detail.version
            ));
            supervisor.note(Stream::Stderr, format!("[插件] 安装失败：{failure}"));
            return Err(failure);
        }
    } else {
        let failure = Error::Market(format!(
            "安装完成但 {name} 未落盘（node_modules 缺失）；请在已安装列表卸载后重试"
        ));
        supervisor.note(Stream::Stderr, format!("[插件] 安装失败：{failure}"));
        return Err(failure);
    }
    // 记入插件清单（08 设计 §2）：失败只记 warn 不回滚安装——清单是增强信息。
    if let Err(failure) = crate::settings::update(&app, |settings| {
        settings
            .plugins
            .pin(PinnedPlugin::new(&name, &detail.version));
    }) {
        supervisor.note(
            Stream::Stderr,
            format!("[插件] 警告：清单未更新（{failure}），不影响本次安装"),
        );
    } else {
        supervisor.note(Stream::Stdout, "[插件] 已记入插件清单".to_owned());
    }
    supervisor.note(
        Stream::Stdout,
        // 用 registry versions 条目里复核到的版本（= 请求版本），形成闭环。
        format!(
            "[插件] {name}@{} 安装完成，DSH 下次启动生效",
            detail.version
        ),
    );
    Ok(())
}

/// 卸载：移出 bundles + pnpm rm。每一步与 pnpm 输出都进 supervisor 日志。
#[tauri::command]
pub async fn market_remove(app: AppHandle, name: String) -> Result<()> {
    let _busy = claim_busy()?;
    ensure_not_safe_mode(&app)?;
    let settings = crate::settings_snapshot(&app)?;
    let profile = profile_dir(&app, &settings)?;
    if name.starts_with(BUILTIN_PREFIX) {
        return Err(Error::Market("内核组件不可卸载".to_owned()));
    }
    let supervisor = app.state::<crate::AppState>().harness.supervisor.clone();
    supervisor.note(Stream::Stdout, format!("[插件] 卸载 {name}"));
    if let Err(failure) = mutate_bundles(&profile, |bundles| bundles.retain(|entry| entry != &name))
    {
        supervisor.note(Stream::Stderr, format!("[插件] 卸载失败：{failure}"));
        return Err(failure);
    }
    supervisor.note(
        Stream::Stdout,
        "[插件] 已从装配清单移除（dsh.profile.bundles）".to_owned(),
    );
    let outcome = run_pnpm(
        &app,
        &settings,
        &supervisor,
        ["remove".into(), name.clone()],
        "插件卸载",
        false,
    )
    .await;
    if let Err(failure) = &outcome {
        supervisor.note(Stream::Stderr, format!("[插件] 卸载失败：{failure}"));
        return outcome;
    }
    // 从清单移除（08 设计 §3）：失败只记 warn 不回滚卸载。
    if let Err(failure) = crate::settings::update(&app, |settings| settings.plugins.unpin(&name)) {
        supervisor.note(
            Stream::Stderr,
            format!("[插件] 警告：清单未更新（{failure}），不影响本次卸载"),
        );
    }
    supervisor.note(
        Stream::Stdout,
        format!("[插件] {name} 卸载完成，DSH 下次启动生效"),
    );
    outcome
}

// ---- 内部 ----

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

/// 反向校验（08 设计 §3）：node_modules/<name>/package.json 存在且读出
/// version 字段。None = 未落盘或 manifest 不合法。
pub(crate) fn deployed_version(profile: &std::path::Path, name: &str) -> Option<String> {
    let manifest_path = profile.join("node_modules").join(name).join("package.json");
    let body = std::fs::read_to_string(manifest_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;
    value.get("version")?.as_str().map(str::to_owned)
}

/// 清单里当前 profile 未落盘的插件个数（备份还原后的对账，08 设计 §5.2）。
pub fn plugins_missing(app: &AppHandle, settings: &Settings) -> usize {
    let Ok(profile) = profile_dir(app, settings) else {
        return settings.plugins.pinned.len();
    };
    settings
        .plugins
        .pinned
        .iter()
        .filter(|entry| {
            deployed_version(&profile, &entry.name).as_deref() != Some(entry.version.as_str())
        })
        .count()
}

/// 单项同步动作（纯数据，便于单测）：sync 前先规划，再逐项执行。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SyncAction {
    /// 已落盘且版本一致 → 跳过（幂等）。
    Skip,
    /// 需要执行安装（含 registry 复核 + pnpm + bundles + 反向校验）。
    Install,
    /// 条目本身有问题（内核包 / 名字或版本非法）→ 跳过并报告原因。
    Reject(&'static str),
}

/// 规划一个清单条目（08 设计 §4.1 步骤 1 的纯函数形态）。
pub(crate) fn plan_sync_entry(entry: &PinnedPlugin, deployed: Option<&str>) -> SyncAction {
    use crate::settings::PluginsSettings;
    if PluginsSettings::is_builtin(&entry.name) {
        return SyncAction::Reject("内核组件不走清单");
    }
    if entry.name.trim().is_empty() || entry.version.trim().is_empty() {
        return SyncAction::Reject("清单条目无效");
    }
    if deployed == Some(entry.version.as_str()) {
        return SyncAction::Skip;
    }
    SyncAction::Install
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
        "[插件] 已写入构建白名单（pnpm-workspace.yaml）".to_owned(),
    );
    Ok(())
}

/// 在 profile 目录里跑一条 pnpm（千寻自带的那份），输出进 supervisor 日志。
/// `with_registry`：只有 add 联网解析接受 `--registry`，remove 传了会报
/// 「Unknown option」。
async fn run_pnpm(
    app: &AppHandle,
    settings: &Settings,
    supervisor: &std::sync::Arc<harness::supervisor::Supervisor>,
    args: impl IntoIterator<Item = String>,
    label: &'static str,
    with_registry: bool,
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

    // pnpm 是原生 exe（@pnpm/exe），直接跑，不经 node。
    let mut command = tokio::process::Command::new(plan.pnpm_cli());
    for arg in args {
        command.arg(&arg);
    }
    command.arg("--dir").arg(profile_dir(app, settings)?);
    if with_registry {
        command.arg("--registry").arg(plan.registry.clone());
    }
    command
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
    .map_err(market_error)
}

/// `run_with_limits` 的错误是 Install 域形态（「安装失败：…」前缀），
/// 市场域剥掉前缀重新包装，避免「卸载失败：安装失败：…」叠罗汉。
fn market_error(failure: crate::error::Error) -> Error {
    let text = failure.to_string();
    let text = text.strip_prefix("安装失败：").unwrap_or(&text);
    Error::Market(text.to_owned())
}

// ---- 清单同步（08 设计 §4）-------------------------------------------------

/// 逐项结果：前端逐行渲染。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncItemResult {
    pub name: String,
    pub ok: bool,
    /// 已落盘且版本一致 → 跳过（幂等，不算失败）。
    pub skipped: bool,
    /// 成功：版本；跳过：「已是 x.y.z」；失败：错误摘要。
    pub detail: String,
}

/// 按清单逐个补装。顺序执行不并发（pnpm 对同一 profile 的并发写会互踩）；
/// 单项失败记录后**继续下一项**（G4），外层 Err 只留给整体性故障。
#[tauri::command]
pub async fn market_sync_pinned(app: AppHandle) -> Result<Vec<SyncItemResult>> {
    let _busy = claim_busy()?;
    ensure_not_safe_mode(&app)?;
    let settings = crate::settings_snapshot(&app)?;
    let profile = profile_dir(&app, &settings)?;
    let supervisor = app.state::<crate::AppState>().harness.supervisor.clone();
    let total = settings.plugins.pinned.len();
    supervisor.note(Stream::Stdout, format!("[插件] 开始同步清单（{total} 项）"));

    let mut results = Vec::new();
    for entry in &settings.plugins.pinned {
        let deployed = deployed_version(&profile, &entry.name);
        // 规划与执行分离：纯函数部分可单测（plan_sync_entry）。
        match plan_sync_entry(entry, deployed.as_deref()) {
            SyncAction::Skip => {
                results.push(SyncItemResult {
                    name: entry.name.clone(),
                    ok: true,
                    skipped: true,
                    detail: format!("已是 {}", entry.version),
                });
            }
            SyncAction::Reject(reason) => {
                results.push(SyncItemResult {
                    name: entry.name.clone(),
                    ok: false,
                    skipped: true,
                    detail: reason.to_owned(),
                });
            }
            SyncAction::Install => {
                let item = install_pinned_item(&app, &settings, &supervisor, entry).await;
                results.push(item);
            }
        }
    }

    let failed = results.iter().filter(|item| !item.ok).count();
    let installed = results
        .iter()
        .filter(|item| item.ok && !item.skipped)
        .count();
    supervisor.note(
        Stream::Stdout,
        format!(
            "[插件] 清单同步完成：安装 {installed}，跳过 {}，失败 {failed}",
            results.len() - installed - failed
        ),
    );
    Ok(results)
}

/// 执行一个清单条目的安装（registry 复核 → pnpm → bundles → 反向校验 → 记清单）。
/// 失败不影响调用方继续下一项（G4）。
async fn install_pinned_item(
    app: &AppHandle,
    settings: &Settings,
    supervisor: &std::sync::Arc<harness::supervisor::Supervisor>,
    entry: &PinnedPlugin,
) -> SyncItemResult {
    let name = &entry.name;
    let registry = settings.mirrors.registry_url();
    let mut failure_text = String::new();
    let mut ok = false;

    let detail = registry::detail(&registry, name, &entry.version).await;
    let detail = match detail {
        Ok(detail) => Some(detail),
        Err(failure) => {
            failure_text = failure.to_string();
            None
        }
    };
    if let Some(detail) = &detail {
        if let Err(failure) = registry::validate(detail) {
            failure_text = failure.to_string();
        }
    }
    if failure_text.is_empty() {
        if let Err(failure) = run_pnpm(
            app,
            settings,
            supervisor,
            ["add".into(), format!("{name}@{}", entry.version)],
            "清单补装",
            true,
        )
        .await
        {
            failure_text = failure.to_string();
        }
    }
    if failure_text.is_empty() {
        if let Err(failure) =
            mutate_bundles(&profile_dir(app, settings).unwrap_or_default(), |bundles| {
                if !bundles.iter().any(|existing| existing == name) {
                    bundles.push(name.clone());
                }
            })
        {
            failure_text = failure.to_string();
        }
    }
    if failure_text.is_empty() {
        let profile = profile_dir(app, settings).unwrap_or_default();
        if deployed_version(&profile, name).as_deref() != Some(entry.version.as_str()) {
            failure_text = format!("落盘校验失败：node_modules 里不是 {}", entry.version);
        }
    }
    if failure_text.is_empty() {
        // 记清单失败不视为该项失败（与 market_install 同口径：清单是增强信息）。
        if let Err(failure) = crate::settings::update(app, |settings| {
            settings
                .plugins
                .pin(PinnedPlugin::new(name, &entry.version));
        }) {
            supervisor.note(
                Stream::Stderr,
                format!("[插件] 警告：{name} 清单未更新（{failure}）"),
            );
        }
        supervisor.note(
            Stream::Stdout,
            format!("[插件] {}@{} 补装完成", name, entry.version),
        );
        ok = true;
    } else {
        supervisor.note(
            Stream::Stderr,
            format!("[插件] {}@{} 补装失败：{failure_text}", name, entry.version),
        );
    }
    SyncItemResult {
        name: name.clone(),
        ok,
        skipped: false,
        detail: if ok {
            format!("已安装 {}", entry.version)
        } else {
            failure_text
        },
    }
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

    // ---- 清单与同步（08 设计 §2/§3/§4）------------------------------------

    use super::{deployed_version, plan_sync_entry, SyncAction};
    use crate::settings::{PinnedPlugin, PluginsSettings};

    #[test]
    fn pinned清单记入更新去重排序() {
        let mut plugins = PluginsSettings::default();
        plugins.pin(PinnedPlugin::new("zeta", "1.0.0"));
        plugins.pin(PinnedPlugin::new("alpha", "0.2.0"));
        plugins.pin(PinnedPlugin::new("zeta", "1.1.0"));
        // 更新不重复（同包新版本覆盖）。
        assert_eq!(plugins.pinned.len(), 2);
        // 按包名小写排序。
        assert_eq!(plugins.pinned[0].name, "alpha");
        assert_eq!(plugins.pinned[1].version, "1.1.0");

        plugins.unpin("alpha");
        assert_eq!(plugins.pinned.len(), 1);
        // 不存在的移除是 no-op。
        plugins.unpin("ghost");
        assert_eq!(plugins.pinned.len(), 1);
    }

    #[test]
    fn settings老文件无plugins字段时清单为空() {
        let text = r#"{
            "schemaVersion": 1,
            "theme": "system",
            "window": { "closeToTray": true, "startMinimized": false, "geometry": null },
            "dsh": { "port": 23090, "allowRandomFallback": false, "versionStrategy": "pinned", "autostart": true, "home": "isolated" },
            "mirrors": { "nodeBinary": "auto", "npmRegistry": "npmmirror" },
            "search": { "rootHistory": [] },
            "hotkeys": { "screenshot": "Ctrl+Shift+A" },
            "notes": { "vaultDir": "" }
        }"#;
        let settings: crate::settings::Settings = serde_json::from_str(text).unwrap();
        assert!(settings.plugins.pinned.is_empty());
    }

    #[test]
    #[allow(non_snake_case)]
    fn deployed_version三态_缺失为None_一致与他版都能读出() {
        let profile = scratch("deployed");
        fs::create_dir_all(profile.join("node_modules/pkg")).unwrap();
        // 缺失：None。
        assert_eq!(deployed_version(&profile, "pkg"), None);
        // 落盘：读出版本。
        fs::write(
            profile.join("node_modules/pkg/package.json"),
            r#"{ "name": "pkg", "version": "1.2.3" }"#,
        )
        .unwrap();
        assert_eq!(deployed_version(&profile, "pkg").as_deref(), Some("1.2.3"));
        // manifest 不合法 → None（按未落盘处理）。
        fs::write(profile.join("node_modules/pkg/package.json"), "not json").unwrap();
        assert_eq!(deployed_version(&profile, "pkg"), None);
        fs::remove_dir_all(&profile).ok();
    }

    #[test]
    fn sync规划_跳过与安装与拒绝() {
        // 已落盘且版本一致 → Skip（幂等：重复点同步不重装）。
        assert_eq!(
            plan_sync_entry(&PinnedPlugin::new("a", "1.0.0"), Some("1.0.0")),
            SyncAction::Skip
        );
        // 未落盘或版本漂移 → Install（pnpm add 精确版本天然收敛）。
        assert_eq!(
            plan_sync_entry(&PinnedPlugin::new("a", "1.0.0"), None),
            SyncAction::Install
        );
        assert_eq!(
            plan_sync_entry(&PinnedPlugin::new("a", "2.0.0"), Some("1.0.0")),
            SyncAction::Install
        );
        // 内核包混入 → 拒绝。
        assert_eq!(
            plan_sync_entry(&PinnedPlugin::new("@deepseek-ai/dsh-base", "1.0.0"), None),
            SyncAction::Reject("内核组件不走清单")
        );
        // 条目非法 → 拒绝。
        assert_eq!(
            plan_sync_entry(&PinnedPlugin::new("", "1.0.0"), None),
            SyncAction::Reject("清单条目无效")
        );
        assert_eq!(
            plan_sync_entry(&PinnedPlugin::new("a", "  "), None),
            SyncAction::Reject("清单条目无效")
        );
    }
}

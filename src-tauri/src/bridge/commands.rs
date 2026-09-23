//! 桥部署 IPC：deploy（幂等落盘 + patch 写入）与 status（三处事实核对）。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::{self, DEFAULT_PROFILE};
use crate::settings::Settings;

const PLUGIN_ID: &str = "qx-bridge";
const PLUGIN_INDEX: &str = include_str!("assets/index.js");
const PLUGIN_WEBSEARCH: &str = include_str!("assets/websearch.js");
const PLUGIN_BROWSERSESSION: &str = include_str!("assets/browsersession.js");
const PLUGIN_PACKAGE_JSON: &str = include_str!("assets/package.json");
const PATCH_MARK: &str = "id: qx-bridge";

/// 桥状态：三处部署事实（源文件 / patch 条目 / vault 一致性）+ DSH 运行态。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    /// 插件文件已在 profile node_modules 就位。
    pub deployed: bool,
    /// cordis.patch.yml 已含 qx-bridge 条目。
    pub patch_entry: bool,
    /// patch 配置里的 vault 与当前设置一致。
    pub vault_match: bool,
    pub vault_dir: String,
    pub plugin_dir: String,
    pub profile_dir: String,
    /// DSH 进程当前是否在跑（跑着则需重启才加载桥）。
    pub dsh_running: bool,
}

/// 部署（幂等）：写插件文件 + 更新 patch 条目。返回最新状态。
#[tauri::command]
pub fn bridge_deploy(app: AppHandle) -> Result<BridgeStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let vault = vault_of(&settings)?;
    let plugin_dir = plugin_dir(&app, &settings)?;

    // 1. 插件文件（幂等覆盖：升级千寻 = 升级桥）。
    std::fs::create_dir_all(&plugin_dir)
        .map_err(|cause| Error::Bridge(format!("建插件目录失败：{cause}")))?;
    std::fs::write(plugin_dir.join("index.js"), PLUGIN_INDEX)
        .and_then(|_| std::fs::write(plugin_dir.join("websearch.js"), PLUGIN_WEBSEARCH))
        .and_then(|_| std::fs::write(plugin_dir.join("browsersession.js"), PLUGIN_BROWSERSESSION))
        .and_then(|_| std::fs::write(plugin_dir.join("package.json"), PLUGIN_PACKAGE_JSON))
        .map_err(|cause| Error::Bridge(format!("写插件文件失败：{cause}")))?;

    // 2. patch 条目（YAML 文本级维护，见 update_patch 注释）。
    let patch_path = patch_path(&app, &settings)?;
    update_patch(
        &patch_path,
        &vault,
        &moli_path_of(&settings),
        &playwright_core_path_of(&settings),
        &proxy_of(&settings),
    )?;

    Ok(status(&app, &settings))
}

/// 状态核对：不落盘、不重启。
#[tauri::command]
pub fn bridge_status(app: AppHandle) -> Result<BridgeStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    Ok(status(&app, &settings))
}

/// 已注册插件条目（插件页清单）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    /// 插件文件已在 profile node_modules 就位。
    pub deployed: bool,
}

/// 从 cordis.patch.yml 列出全部注册插件（文本级解析，与 update_patch
/// 同一套约定）；profile 尚不存在时返回空清单，不报错。
#[tauri::command]
pub fn plugins_list(app: AppHandle) -> Result<Vec<PluginEntry>> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let Ok(patch) = patch_path(&app, &settings) else {
        return Ok(Vec::new());
    };
    let Ok(text) = std::fs::read_to_string(patch) else {
        return Ok(Vec::new());
    };
    let node_modules = profile_dir(&app, &settings)
        .ok()
        .map(|dir| dir.join("node_modules"));

    let mut entries = Vec::new();
    let mut pending: Option<String> = None;
    let mut flush =
        |id: Option<String>, name: String, node_modules: Option<&std::path::PathBuf>| {
            let Some(id) = id else { return };
            let deployed = node_modules
                .map(|dir| dir.join(&id).is_dir())
                .unwrap_or(false);
            entries.push(PluginEntry { id, name, deployed });
        };
    for line in text.lines() {
        let trimmed = line.trim_start();
        let rest = trimmed
            .strip_prefix("- id:")
            .or_else(|| trimmed.strip_prefix("id:"));
        if let Some(id) = rest {
            flush(pending.take(), "未命名".to_owned(), node_modules.as_ref());
            pending = Some(id.trim().to_owned());
            continue;
        }
        if let Some(name) = trimmed.strip_prefix("name:") {
            flush(
                pending.take(),
                name.trim().to_owned(),
                node_modules.as_ref(),
            );
        }
    }
    flush(pending.take(), "未命名".to_owned(), node_modules.as_ref());
    Ok(entries)
}

/// 外壳启动自愈入口：已部署过（patch 有条目）但插件文件丢失（DSH 重装
/// 清理了 node_modules）时静默补齐。失败只记日志，不阻断启动。
pub fn heal(app: &AppHandle) {
    let settings = {
        let state = app.state::<crate::AppState>();
        let Ok(guard) = state.settings.try_lock() else {
            return;
        };
        guard.clone()
    };
    if settings.notes.vault_dir.trim().is_empty() {
        return;
    }
    let Ok(patch) = patch_path(app, &settings) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&patch) else {
        return;
    };
    if !text.contains(PATCH_MARK) {
        return; // 从未部署过：不打扰。
    }
    let Ok(plugin_dir) = plugin_dir(app, &settings) else {
        return;
    };
    if plugin_dir.join("index.js").is_file() {
        // 内容指纹自愈：文件存在 ≠ 内容最新。升级千寻（include_str! 常量
        // 随重编译变化）或 debug 期间改 assets 后，profile 副本会静默滞后，
        // DSH 加载的永远是旧文件。逐文件比对，一致才免于重写。
        if !plugin_stale(&plugin_dir) {
            return;
        }
        crate::logging::log("info", "桥插件副本过期，重新部署");
    }
    if let Err(cause) = bridge_deploy(app.clone()) {
        crate::logging::log("warn", &format!("桥自愈失败：{cause}"));
    }
}

// ---- 内部 ----

/// profile 副本是否与编译期内置不一致（任一文件缺失或内容不同 = 过期）。
fn plugin_stale(plugin_dir: &std::path::Path) -> bool {
    [
        ("index.js", PLUGIN_INDEX),
        ("websearch.js", PLUGIN_WEBSEARCH),
        ("browsersession.js", PLUGIN_BROWSERSESSION),
        ("package.json", PLUGIN_PACKAGE_JSON),
    ]
    .iter()
    .any(|(name, want)| {
        std::fs::read_to_string(plugin_dir.join(name))
            .map(|have| have != *want)
            .unwrap_or(true)
    })
}

/// moli 自备路径（可为空 = 桥走降级）；空值也写入，保证 patch 与设置一致。
fn moli_path_of(settings: &Settings) -> String {
    settings.tools.moli.binary_path.trim().to_owned()
}

/// playwright-core 加载路径（可为空 = 桥内自动探测）。
fn playwright_core_path_of(settings: &Settings) -> String {
    settings.tools.playwright_core_path.trim().to_owned()
}

/// moli 抓取代理（R001-TUN）：空 = 直连；"off" = 显式直连；其它传
/// moli --http-proxy（代理侧解析，绕开 TUN fake-ip 的内网误判）。
fn proxy_of(settings: &Settings) -> String {
    settings.web.proxy.trim().to_owned()
}

fn vault_of(settings: &Settings) -> Result<String> {
    let vault = settings.notes.vault_dir.trim().to_owned();
    if vault.is_empty() {
        return Err(Error::Bridge(
            "尚未初始化笔记库：先到笔记页初始化，再部署桥".to_owned(),
        ));
    }
    if !std::path::Path::new(&vault).is_dir() {
        return Err(Error::Bridge(format!("笔记库目录不存在：{vault}")));
    }
    Ok(vault)
}

fn profile_dir(app: &AppHandle, settings: &Settings) -> Result<std::path::PathBuf> {
    let dir = harness::dsh_home(app, settings)
        .join("profiles")
        .join(DEFAULT_PROFILE);
    if !dir.is_dir() {
        return Err(Error::Bridge(format!(
            "DSH profile 目录不存在：{}。请先完成 DSH 安装并至少启动一次。",
            dir.display()
        )));
    }
    Ok(dir)
}

fn plugin_dir(app: &AppHandle, settings: &Settings) -> Result<std::path::PathBuf> {
    Ok(profile_dir(app, settings)?
        .join("node_modules")
        .join(PLUGIN_ID))
}

fn patch_path(app: &AppHandle, settings: &Settings) -> Result<std::path::PathBuf> {
    Ok(profile_dir(app, settings)?.join("cordis.patch.yml"))
}

/// patch 文本级维护（三态）：
/// `[]` → 写完整模板；已有条目 → 只替换 vault 行；其余 → 文末追加 insert 块。
/// 个人工具的确定性维护，不引 YAML 依赖；任何形态下 `id: qx-bridge` 恒定可寻。
fn update_patch(
    patch: &std::path::Path,
    vault: &str,
    moli_path: &str,
    playwright_core_path: &str,
    proxy: &str,
) -> Result<()> {
    // YAML 双引号串里反斜杠是转义符：统一正斜杠（Node/Windows 均接受）。
    let vault_yaml = format!("\"{}\"", vault.replace('\\', "/").replace('"', ""));
    let moli_yaml = format!("\"{}\"", moli_path.replace('\\', "/").replace('"', ""));
    let pw_yaml = format!(
        "\"{}\"",
        playwright_core_path.replace('\\', "/").replace('"', "")
    );
    let proxy_yaml = format!("\"{}\"", proxy.replace('\\', "/").replace('"', ""));
    let entry_yaml = format!(
        "- insert:\n    - id: {PLUGIN_ID}\n      name: {PLUGIN_ID}\n      config:\n        vault: {vault_yaml}\n        moliPath: {moli_yaml}\n        playwrightCorePath: {pw_yaml}\n        proxy: {proxy_yaml}\n"
    );
    let text = std::fs::read_to_string(patch).unwrap_or_default();
    let new_text = if text.trim() == "[]" {
        format!("# qx-bridge（千寻笔记桥）：由千寻写入，请勿手工编辑\n{entry_yaml}")
    } else if text.contains(PATCH_MARK) {
        // 条目已存在：整块替换 config 各行（vault / moliPath / playwrightCorePath / proxy）。
        let mut replaced_vault = false;
        let mut replaced_moli = false;
        let mut replaced_pw = false;
        let mut replaced_proxy = false;
        let mut lines: Vec<String> = text
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("vault:") && !replaced_vault {
                    replaced_vault = true;
                    format!("        vault: {vault_yaml}")
                } else if line.trim_start().starts_with("moliPath:") && !replaced_moli {
                    replaced_moli = true;
                    format!("        moliPath: {moli_yaml}")
                } else if line.trim_start().starts_with("playwrightCorePath:") {
                    replaced_pw = true;
                    format!("        playwrightCorePath: {pw_yaml}")
                } else if line.trim_start().starts_with("proxy:") {
                    replaced_proxy = true;
                    format!("        proxy: {proxy_yaml}")
                } else {
                    line.to_owned()
                }
            })
            .collect();
        // 历史版本可能把 [] 残留在条目前（e2e 踩坑）：过滤空壳数组行。
        lines.retain(|line| line.trim() != "[]");
        if !replaced_vault {
            return Err(Error::Bridge(
                "patch 条目残缺（无 vault 行）：请手工清理后重试".to_owned(),
            ));
        }
        if !replaced_moli {
            // 旧版 patch 无 moliPath：在 vault 行后补插。
            if let Some(index) = lines
                .iter()
                .position(|line| line.trim_start().starts_with("vault:"))
            {
                lines.insert(index + 1, format!("        moliPath: {moli_yaml}"));
            }
        }
        if !replaced_proxy {
            // 旧版 patch 无 proxy（R001-TUN 前）：在 playwrightCorePath 行后补插。
            let anchor = lines
                .iter()
                .position(|line| line.trim_start().starts_with("playwrightCorePath:"))
                .or_else(|| {
                    lines
                        .iter()
                        .position(|line| line.trim_start().starts_with("moliPath:"))
                });
            if let Some(index) = anchor {
                lines.insert(index + 1, format!("        proxy: {proxy_yaml}"));
            }
        }
        let mut out = lines.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        out
    } else {
        // 全新 patch（DSH 默认模板 = 注释 + []）：剥掉注释与空壳 [] 后
        // 已无实质内容时直接重写为完整条目——[] 与 - insert 混排是非法
        // YAML，DSH 解析失败会拒绝启动（e2e 实测踩坑，2026-02）。
        let body: String = text
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed != "[]"
            })
            .collect::<Vec<_>>()
            .join("\n");
        if body.is_empty() {
            format!("# qx-bridge（千寻笔记桥）：由千寻写入，请勿手工编辑\n{entry_yaml}")
        } else {
            let mut out = text.trim_end().to_owned();
            out.push_str(&format!("\n{entry_yaml}"));
            out
        }
    };
    std::fs::write(patch, new_text)
        .map_err(|cause| Error::Bridge(format!("写 cordis.patch.yml 失败：{cause}")))
}

fn status(app: &AppHandle, settings: &Settings) -> BridgeStatus {
    let vault = settings.notes.vault_dir.trim().to_owned();
    let deployed = plugin_dir(app, settings)
        .map(|dir| dir.join("index.js").is_file())
        .unwrap_or(false);
    let patch_text = patch_path(app, settings)
        .and_then(|path| std::fs::read_to_string(path).map_err(|_| Error::Bridge(String::new())))
        .unwrap_or_default();
    let patch_entry = patch_text.contains(PATCH_MARK);
    let vault_match = patch_entry
        && patch_text.lines().any(|line| {
            line.trim_start().starts_with("vault:") && line.contains(&vault.replace('\\', "/"))
        });
    let dsh_running = matches!(
        app.state::<crate::AppState>().harness.supervisor.status(),
        crate::harness::supervisor::Status::Starting
            | crate::harness::supervisor::Status::Ready { .. }
            | crate::harness::supervisor::Status::Restarting { .. }
    );
    BridgeStatus {
        deployed,
        patch_entry,
        vault_match,
        vault_dir: vault,
        plugin_dir: plugin_dir(app, settings)
            .map(|dir| dir.display().to_string())
            .unwrap_or_default(),
        profile_dir: profile_dir(app, settings)
            .map(|dir| dir.display().to_string())
            .unwrap_or_default(),
        dsh_running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch三态维护() {
        let dir = std::env::temp_dir().join(format!("qx-patch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let patch = dir.join("cordis.patch.yml");

        // 空模板 → 完整条目（含 moliPath 与 proxy，R001/R001-TUN）。
        std::fs::write(&patch, "[]\n").unwrap();
        update_patch(&patch, r"D:\docs\千寻笔记", "", "", "").unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("id: qx-bridge"));
        assert!(text.contains(r#"vault: "D:/docs/千寻笔记""#));
        assert!(text.contains(r#"moliPath: """#));
        assert!(text.contains(r#"playwrightCorePath: """#));
        assert!(text.contains(r#"proxy: """#));

        // 已有条目 → 只换 vault / moliPath / proxy，条目不重复。
        update_patch(
            &patch,
            r"D:\other\vault",
            "D:/tools/moli.exe",
            "",
            "socks5://127.0.0.1:1080",
        )
        .unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains(r#""D:/other/vault""#));
        assert!(text.contains(r#"moliPath: "D:/tools/moli.exe""#));
        assert!(text.contains(r#"proxy: "socks5://127.0.0.1:1080""#));
        assert_eq!(text.matches("id: qx-bridge").count(), 1);
        assert_eq!(text.matches("moliPath:").count(), 1);
        assert_eq!(text.matches("playwrightCorePath:").count(), 1);
        assert_eq!(text.matches("proxy:").count(), 1);

        // 旧版条目无 proxy 行 → 补插一行，不重复、不破坏既有行。
        let legacy = "# qx-bridge（千寻笔记桥）：由千寻写入，请勿手工编辑\n- insert:\n    - id: qx-bridge\n      name: qx-bridge\n      config:\n        vault: \"D:/v\"\n        moliPath: \"\"\n";
        std::fs::write(&patch, legacy).unwrap();
        update_patch(&patch, r"D:\v", "", "", "off").unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains(r#"proxy: "off""#));
        assert_eq!(text.matches("proxy:").count(), 1);
        assert!(text.contains(r#"vault: "D:/v""#));
        assert!(text.contains(r#"moliPath: """#));

        // 用户已有其他条目 → 文末追加。
        std::fs::write(&patch, "- insert:\n    - id: my-thing\n      name: foo\n").unwrap();
        update_patch(&patch, r"D:\docs\v", "", "", "").unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("id: my-thing"));
        assert!(text.contains("id: qx-bridge"));

        // DSH 默认模板（注释 + []）→ 重写为合法单条目（e2e 踩坑回归）。
        std::fs::write(
            &patch,
            "# Your patch layer for this dsh profile, applied after every bundle layer:\n# a top-level YAML array of loader patch entries.\n[]\n",
        )
        .unwrap();
        update_patch(&patch, r"D:\docs\v2", "", "", "").unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(!text.contains("[]"), "[] 残留会产出非法 YAML：{text}");
        assert!(text.contains("id: qx-bridge"));
        assert!(text.contains(r#"vault: "D:/docs/v2""#));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 内容指纹自愈缺文件或旧内容判定过期() {
        let dir = std::env::temp_dir().join(format!("qx-bridge-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 全新目录：缺文件 = 过期。
        assert!(plugin_stale(&dir));

        // 全部写入当前内置内容 → 新鲜。
        std::fs::write(dir.join("index.js"), PLUGIN_INDEX).unwrap();
        std::fs::write(dir.join("websearch.js"), PLUGIN_WEBSEARCH).unwrap();
        std::fs::write(dir.join("browsersession.js"), PLUGIN_BROWSERSESSION).unwrap();
        std::fs::write(dir.join("package.json"), PLUGIN_PACKAGE_JSON).unwrap();
        assert!(!plugin_stale(&dir));

        // 只改一个文件（模拟 debug 改 assets / 升级千寻）→ 过期。
        std::fs::write(dir.join("websearch.js"), "// stale\n").unwrap();
        assert!(plugin_stale(&dir));

        std::fs::remove_dir_all(&dir).ok();
    }
}

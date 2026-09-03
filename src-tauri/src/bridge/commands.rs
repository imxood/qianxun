//! 桥部署 IPC：deploy（幂等落盘 + patch 写入）与 status（三处事实核对）。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::{self, DEFAULT_PROFILE};
use crate::settings::Settings;

const PLUGIN_ID: &str = "qx-bridge";
const PLUGIN_INDEX: &str = include_str!("assets/index.js");
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
        .and_then(|_| std::fs::write(plugin_dir.join("package.json"), PLUGIN_PACKAGE_JSON))
        .map_err(|cause| Error::Bridge(format!("写插件文件失败：{cause}")))?;

    // 2. patch 条目（YAML 文本级维护，见 update_patch 注释）。
    let patch_path = patch_path(&app, &settings)?;
    update_patch(&patch_path, &vault)?;

    Ok(status(&app, &settings))
}

/// 状态核对：不落盘、不重启。
#[tauri::command]
pub fn bridge_status(app: AppHandle) -> Result<BridgeStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    Ok(status(&app, &settings))
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
        return;
    }
    if let Err(cause) = bridge_deploy(app.clone()) {
        crate::logging::log("warn", &format!("桥自愈失败：{cause}"));
    }
}

// ---- 内部 ----

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
fn update_patch(patch: &std::path::Path, vault: &str) -> Result<()> {
    // YAML 双引号串里反斜杠是转义符：统一正斜杠（Node/Windows 均接受）。
    let vault_yaml = format!("\"{}\"", vault.replace('\\', "/").replace('"', ""));
    let text = std::fs::read_to_string(patch).unwrap_or_default();
    let new_text = if text.trim() == "[]" {
        format!(
            "# qx-bridge（千寻笔记桥）：由千寻写入，请勿手工编辑\n- insert:\n    - id: {PLUGIN_ID}\n      name: {PLUGIN_ID}\n      config:\n        vault: {vault_yaml}\n"
        )
    } else if text.contains(PATCH_MARK) {
        let mut replaced = false;
        let lines: Vec<String> = text
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("vault:") && !replaced {
                    // 只改 qx-bridge 块内的第一处 vault（条目内即本桥）。
                    replaced = true;
                    format!("        vault: {vault_yaml}")
                } else {
                    line.to_owned()
                }
            })
            .collect();
        if !replaced {
            return Err(Error::Bridge(
                "patch 条目残缺（无 vault 行）：请手工清理后重试".to_owned(),
            ));
        }
        let mut out = lines.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        out
    } else {
        let mut out = text.trim_end().to_owned();
        out.push_str(&format!(
            "\n- insert:\n    - id: {PLUGIN_ID}\n      name: {PLUGIN_ID}\n      config:\n        vault: {vault_yaml}\n"
        ));
        out
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

        // 空模板 → 完整条目。
        std::fs::write(&patch, "[]\n").unwrap();
        update_patch(&patch, r"D:\docs\千寻笔记").unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("id: qx-bridge"));
        assert!(text.contains(r#"vault: "D:/docs/千寻笔记""#));

        // 已有条目 → 只换 vault。
        update_patch(&patch, r"D:\other\vault").unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains(r#""D:/other/vault""#));
        assert_eq!(text.matches("id: qx-bridge").count(), 1);

        // 用户已有其他条目 → 文末追加。
        std::fs::write(&patch, "- insert:\n    - id: my-thing\n      name: foo\n").unwrap();
        update_patch(&patch, r"D:\docs\v").unwrap();
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("id: my-thing"));
        assert!(text.contains("id: qx-bridge"));

        std::fs::remove_dir_all(&dir).ok();
    }
}

//! 内置 playwright 工具部署：vendor 三件套落盘 + dsh-mcp-client patch 条目
//! 维护（moli/Edge 浏览器选择随环境页设置动态对齐）+ 状态核对 + 启动自愈。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::{self, DEFAULT_PROFILE};
use crate::settings::Settings;

const PLUGIN_ID: &str = "qx-playwright";
/// patch 里本包的寻址标记。
const PATCH_MARK: &str = "id: qx-playwright";
/// 受管块的头注释：块替换的起点（到下一个 "# " 注释行或文末）。
const BLOCK_HEAD: &str = "# qx-playwright（千寻内置 playwright 工具）：由千寻写入，请勿手工编辑";
/// vendor 三件套（sync 脚本产出，打包进资源；dev 直读 src-tauri/vendor）。
const VENDOR_PACKAGES: [&str; 3] = ["@playwright/mcp", "playwright", "playwright-core"];
/// launcher 外壳：moli serve --layout sidecar + MCP stdio 透传（内容指纹自愈）。
const PLUGIN_LAUNCHER: &str = include_str!("assets/launcher.js");
const PLUGIN_PACKAGE_JSON: &str = include_str!("assets/package.json");

/// 状态：三处部署事实 + 当前浏览器选择。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaywrightStatus {
    /// vendor 三件套已在 profile node_modules 就位。
    pub deployed: bool,
    /// cordis.patch.yml 已含 qx-playwright 条目。
    pub patch_entry: bool,
    /// 当前选择的浏览器：moli / msedge / none（都不是 → 工具调用时报错）。
    pub browser: String,
    pub plugin_dir: String,
    /// DSH 进程当前是否在跑（跑着则需重启才加载新条目）。
    pub dsh_running: bool,
}

/// 部署（幂等）：拷贝 vendor 三件套 + 维护 patch 条目。返回最新状态。
#[tauri::command]
pub fn playwright_deploy(app: AppHandle) -> Result<PlaywrightStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    deploy(&app, &settings)?;
    Ok(status(&app, &settings))
}

/// 状态核对：不落盘、不重启。
#[tauri::command]
pub fn playwright_status(app: AppHandle) -> Result<PlaywrightStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    Ok(status(&app, &settings))
}

/// 启动自愈/保活：三件套版本与 launcher 内容指纹均一致时只对齐 patch，
/// 任一过期则整包重铺。失败只记日志。
pub fn ensure(app: &AppHandle) {
    let settings = {
        let state = app.state::<crate::AppState>();
        let Ok(guard) = state.settings.try_lock() else {
            return;
        };
        guard.clone()
    };
    let Ok(profile_nm) = profile_dir(app, &settings).map(|dir| dir.join("node_modules")) else {
        return; // profile 尚不存在：首次安装流程稍后会再触发。
    };
    let Some(source) = vendor_source(app) else {
        return; // 无内置副本（CI 等场景）：静默跳过。
    };
    let plugin_dir = profile_nm.join(PLUGIN_ID);
    let deployed_ver = package_version(&profile_nm.join("@playwright").join("mcp"));
    let source_ver = package_version(&source.join("@playwright").join("mcp"));
    let version_fresh = deployed_ver.is_some() && deployed_ver == source_ver;
    let launcher_fresh = std::fs::read_to_string(plugin_dir.join("launcher.js"))
        .map(|have| have == PLUGIN_LAUNCHER)
        .unwrap_or(false);
    if version_fresh && launcher_fresh {
        // 已就位：只需对齐 patch 的动态值（moli 路径可能变了）。
        let Ok(patch) = patch_path(app, &settings) else {
            return;
        };
        match build_and_write_patch(app, &settings, &patch, &profile_nm) {
            Ok(true) => crate::logging::log("info", "内置 playwright patch 已对齐"),
            Ok(false) => {}
            Err(cause) => {
                crate::logging::log("warn", &format!("内置 playwright patch 写入失败：{cause}"))
            }
        }
        return;
    }
    if let Err(cause) = deploy(app, &settings) {
        crate::logging::log("warn", &format!("内置 playwright 部署失败：{cause}"));
    }
}

// ---- 内部 ----

fn deploy(app: &AppHandle, settings: &Settings) -> Result<()> {
    let profile_nm = profile_dir(app, settings)?.join("node_modules");
    let Some(source_nm) = vendor_source(app) else {
        return Err(Error::Bridge(
            "内置 playwright vendor 缺失：请先运行 pnpm sync:pwc 再构建".to_owned(),
        ));
    };

    // 1. 三件套拷贝（幂等：@playwright/mcp 版本一致跳过）。
    let deployed_ver = package_version(&profile_nm.join("@playwright").join("mcp"));
    let source_ver = package_version(&source_nm.join("@playwright").join("mcp"));
    if deployed_ver.is_none() || deployed_ver != source_ver {
        for name in VENDOR_PACKAGES {
            let dest = profile_nm.join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
            if dest.exists() {
                std::fs::remove_dir_all(&dest)
                    .map_err(|cause| Error::Bridge(format!("清理旧 {name} 失败：{cause}")))?;
            }
            copy_dir_recursive(&source_nm.join(name), &dest)?;
        }
        crate::logging::log(
            "info",
            &format!(
                "内置 playwright 三件套就绪（@playwright/mcp {}）",
                source_ver.as_deref().unwrap_or("?")
            ),
        );
    }

    // 2. launcher 外壳（内容指纹：升级千寻 = 升级外壳）。
    let plugin_dir = profile_nm.join(PLUGIN_ID);
    std::fs::create_dir_all(&plugin_dir)
        .map_err(|cause| Error::Bridge(format!("建插件目录失败：{cause}")))?;
    let stale = std::fs::read_to_string(plugin_dir.join("launcher.js"))
        .map(|have| have != PLUGIN_LAUNCHER)
        .unwrap_or(true);
    if stale {
        std::fs::write(plugin_dir.join("launcher.js"), PLUGIN_LAUNCHER)
            .and_then(|_| std::fs::write(plugin_dir.join("package.json"), PLUGIN_PACKAGE_JSON))
            .map_err(|cause| Error::Bridge(format!("写 launcher 失败：{cause}")))?;
    }

    // 3. patch 条目（幂等 + 动态值对齐）。
    let patch = patch_path(app, settings)?;
    build_and_write_patch(app, settings, &patch, &profile_nm)?;

    Ok(())
}

/// 递归拷贝目录（跟随 symlink，pnpm 的 node_modules 条目是真包的链接）。
fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|cause| Error::Bridge(format!("建目录失败：{cause}")))?;
    let entries =
        std::fs::read_dir(src).map_err(|cause| Error::Bridge(format!("读源目录失败：{cause}")))?;
    for entry in entries {
        let entry = entry.map_err(|cause| Error::Bridge(format!("遍历源目录失败：{cause}")))?;
        let target = dest.join(entry.file_name());
        let is_dir = entry
            .file_type()
            .map_err(|cause| Error::Bridge(format!("读文件类型失败：{cause}")))?
            .is_dir();
        if is_dir {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|cause| Error::Bridge(format!("拷贝文件失败：{cause}")))?;
        }
    }
    Ok(())
}

/// patch 块生成 + 块级替换落盘。返回是否发生写入。
fn build_and_write_patch(
    app: &AppHandle,
    settings: &Settings,
    patch: &std::path::Path,
    profile_nm: &std::path::Path,
) -> Result<bool> {
    let node = node_command(app, settings);
    // launcher 外壳：浏览器选择在运行时判定（moli serve --layout +
    // --cdp-endpoint，或 msedge 兜底），patch 只需静态指向外壳。
    let launcher = yaml_str(
        &profile_nm
            .join(PLUGIN_ID)
            .join("launcher.js")
            .to_string_lossy()
            .replace('\\', "/"),
    );
    let moli_yaml = crate::tools::moli::resolve_binary(app, settings)
        .map(|(path, _)| yaml_str(&path.to_string_lossy().replace('\\', "/")))
        .unwrap_or_else(|| "\"\"".to_owned());
    let profile_dir_yaml = yaml_str(
        &harness::dsh_home(app, settings)
            .join("playwright-mcp-profile")
            .to_string_lossy()
            .replace('\\', "/"),
    );
    let block = format!(
        "{BLOCK_HEAD}\n- insert:\n    - id: {PLUGIN_ID}\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: playwright\n        transport: stdio\n        failOnStartupError: false\n        command: {node}\n        args:\n          - {launcher}\n        env:\n          QX_MOLI_PATH: {moli_yaml}\n          QX_MCP_PROFILE_DIR: {profile_dir_yaml}\n"
    );
    update_patch(patch, &block)
}

/// patch 文本级维护（块级替换）：已有受管块（BLOCK_HEAD 起，到下一个
/// "# " 注释行或文末）整体替换；没有则文末追加。内容无变化不落盘。
fn update_patch(patch: &std::path::Path, block: &str) -> Result<bool> {
    let text = std::fs::read_to_string(patch).unwrap_or_else(|_| "[]\n".to_owned());
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let updated = if let Some(start) = lines.iter().position(|line| line.trim() == BLOCK_HEAD) {
        let end = lines[start + 1..]
            .iter()
            .position(|line| line.starts_with("# "))
            .map(|offset| start + 1 + offset)
            .unwrap_or(lines.len());
        let mut next: Vec<String> = lines[..start].to_vec();
        next.extend(block.lines().map(str::to_owned));
        next.extend(lines[end..].iter().cloned());
        lines = next;
        render(&lines)
    } else {
        let mut joined = text.clone();
        if !joined.ends_with('\n') {
            joined.push('\n');
        }
        joined.push('\n');
        joined.push_str(block);
        joined
    };
    if updated == text {
        return Ok(false);
    }
    std::fs::write(patch, updated)
        .map_err(|cause| Error::Bridge(format!("写 patch 失败：{cause}")))?;
    Ok(true)
}

fn render(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// YAML 双引号串：反斜杠是转义符，统一正斜杠（Node/Windows 均接受）。
fn yaml_str(raw: &str) -> String {
    format!("\"{}\"", raw.replace('\\', "/").replace('"', ""))
}

/// node 可执行：优先千寻托管的 Node 安装，缺失退回 PATH 上的 node。
fn node_command(app: &AppHandle, settings: &Settings) -> String {
    harness::environment(app, settings)
        .node
        .map(|node| yaml_str(&node.path.to_string_lossy().replace('\\', "/")))
        .unwrap_or_else(|| "node".to_owned())
}

/// 当前浏览器选择（与 patch args 一致的判定）："moli" / "msedge" / "none"。
fn browser_choice(app: &AppHandle, settings: &Settings) -> String {
    if crate::tools::moli::resolve_binary(app, settings).is_some() {
        return "moli".to_owned();
    }
    if cfg!(windows) {
        return "msedge".to_owned();
    }
    "none".to_owned()
}

/// vendor 三件套来源：打包资源 → dev 编译期仓库路径。
fn vendor_source(app: &AppHandle) -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("playwright-mcp").join("node_modules"));
    }
    candidates.push(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("playwright-mcp")
            .join("node_modules"),
    );
    candidates.into_iter().find(|dir| {
        dir.join("@playwright")
            .join("mcp")
            .join("package.json")
            .is_file()
    })
}

/// 读包目录 package.json 的 version 字段。
fn package_version(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("version")?
        .as_str()
        .map(str::to_owned)
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

fn patch_path(app: &AppHandle, settings: &Settings) -> Result<std::path::PathBuf> {
    Ok(profile_dir(app, settings)?.join("cordis.patch.yml"))
}

fn status(app: &AppHandle, settings: &Settings) -> PlaywrightStatus {
    let profile_ok = profile_dir(app, settings)
        .map(|dir| {
            VENDOR_PACKAGES.iter().all(|name| {
                dir.join("node_modules")
                    .join(name.replace('/', std::path::MAIN_SEPARATOR_STR))
                    .is_dir()
            })
        })
        .unwrap_or(false);
    let patch_entry = patch_path(app, settings)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.contains(PATCH_MARK))
        .unwrap_or(false);
    PlaywrightStatus {
        deployed: profile_ok,
        patch_entry,
        browser: browser_choice(app, settings),
        plugin_dir: profile_dir(app, settings)
            .map(|dir| dir.join("node_modules").to_string_lossy().into_owned())
            .unwrap_or_default(),
        dsh_running: matches!(
            app.state::<crate::AppState>().harness.supervisor.status(),
            crate::harness::supervisor::Status::Starting
                | crate::harness::supervisor::Status::Ready { .. }
                | crate::harness::supervisor::Status::Restarting { .. }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(file: &std::path::Path, text: &str) {
        std::fs::write(file, text).unwrap();
    }

    const HEAD: &str = BLOCK_HEAD;

    #[test]
    fn patch空文件追加完整块() {
        let dir = std::env::temp_dir().join(format!("qx-pw-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let patch = dir.join("cordis.patch.yml");
        write(&patch, "[]\n");
        let block = format!("{HEAD}\n- insert:\n    - id: qx-playwright\n      config: {{}}\n");
        assert!(update_patch(&patch, &block).unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains(PATCH_MARK));
        assert!(text.starts_with("[]\n"));
    }

    #[test]
    fn patch已有块整体替换且幂等() {
        let dir = std::env::temp_dir().join(format!("qx-pw-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let patch = dir.join("cordis.patch.yml");
        let old_block = format!(
            "{HEAD}\n- insert:\n    - id: qx-playwright\n      config:\n        command: old\n"
        );
        write(
            &patch,
            &format!("# 桥（保留）：由千寻写入\n- insert:\n    - id: qx-bridge\n      config: {{}}\n\n{old_block}\n"),
        );
        let new_block = format!(
            "{HEAD}\n- insert:\n    - id: qx-playwright\n      config:\n        command: new\n"
        );
        assert!(update_patch(&patch, &new_block).unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("command: new"));
        assert!(!text.contains("command: old"));
        assert!(text.contains("id: qx-bridge"), "邻居条目不得被动");
        // 幂等：同块再写不落盘。
        assert!(!update_patch(&patch, &new_block).unwrap());
    }

    #[test]
    fn patch替换不吞后续受管块() {
        let dir = std::env::temp_dir().join(format!("qx-pw-test3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let patch = dir.join("cordis.patch.yml");
        write(
            &patch,
            &format!(
                "{HEAD}\n- insert:\n    - id: qx-playwright\n      config: {{}}\n# qx-websearch（保留）：由千寻写入\n- insert:\n    - id: qx-websearch\n      config: {{}}\n"
            ),
        );
        let block =
            format!("{HEAD}\n- insert:\n    - id: qx-playwright\n      config:\n        v: 2\n");
        assert!(update_patch(&patch, &block).unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("id: qx-websearch"), "后续块不得被吞");
        assert!(text.contains("v: 2"));
    }
}

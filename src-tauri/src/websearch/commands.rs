//! 内置联网搜索部署：插件文件落盘 + patch 行维护 + 状态核对。
//!
//! 与桥（bridge）同一套部署模型：宿主托管（千寻写 patch 行 + 幂等落盘
//! profile node_modules），包不经过包管理器、不进 dsh.profile.bundles。
//! patch 两行：`web` 行覆盖（searchProvider 指向本包）+ insert 挂载本包
//! （name 解析 exports["."] 装载宿主半段，dsh.client 声明装载设置卡）。

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};
use crate::harness::{self, DEFAULT_PROFILE};
use crate::settings::Settings;

const PLUGIN_ID: &str = "qx-websearch";
const PLUGIN_PACKAGE_JSON: &str = include_str!("assets/package.json");
const PLUGIN_INDEX: &str = include_str!("assets/dsh/index.js");
const PLUGIN_CLIENT: &str = include_str!("assets/dsh/client.js");
const PLUGIN_SPAWN_HIDDEN: &str = include_str!("assets/dsh/spawnHidden.js");
const PLUGIN_SEARCH_SCHEMA: &str = include_str!("assets/dsh/search-schema.json");
const PLUGIN_FETCH_SCHEMA: &str = include_str!("assets/dsh/fetch-schema.json");
const PLUGIN_CLI: &str = include_str!("assets/dist/main.js");
/// patch 里本包的寻址标记：searchProvider 覆盖行是唯一写一次的值。
const PATCH_MARK: &str = "searchProvider: qx-websearch";
/// fetch provider 迁移标记：出现 = patch 已是含 web_fetch 的新形态。
const FETCH_MARK: &str = "fetchProvider: qx-websearch";

/// 联网搜索状态：部署事实 + DSH 运行态。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsearchStatus {
    /// 插件文件已在 profile node_modules 就位。
    pub deployed: bool,
    /// cordis.patch.yml 已含本包的 web 覆盖行。
    pub patch_entry: bool,
    pub plugin_dir: String,
    /// DSH 进程当前是否在跑（跑着则需重启才加载）。
    pub dsh_running: bool,
}

/// 部署（幂等）：写插件文件 + 确保 patch 行。返回最新状态。
#[tauri::command]
pub fn websearch_deploy(app: AppHandle) -> Result<WebsearchStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    deploy(&app, &settings)?;
    Ok(status(&app, &settings))
}

/// 状态核对：不落盘、不重启。
#[tauri::command]
pub fn websearch_status(app: AppHandle) -> Result<WebsearchStatus> {
    let state = app.state::<crate::AppState>();
    let settings = state.settings.lock().unwrap().clone();
    Ok(status(&app, &settings))
}

/// 启动自愈/保活：联网搜索是安装包内置能力，每次启动幂等确保。
/// profile 尚未就绪（首次安装未跑 DSH）时静默跳过；失败只记日志。
pub fn ensure(app: &AppHandle) {
    let settings = {
        let state = app.state::<crate::AppState>();
        let Ok(guard) = state.settings.try_lock() else {
            return;
        };
        guard.clone()
    };
    let Ok(plugin_dir) = plugin_dir(app, &settings) else {
        return; // profile 尚不存在：首次启动流程稍后会再触发。
    };
    if plugin_dir.join("dsh").join("index.js").is_file() {
        // 内容指纹自愈：文件存在 ≠ 内容最新。升级千寻（include_str! 常量随
        // 重编译变化）或 debug 期间改 assets 后，profile 副本会静默滞后，
        // DSH 加载的永远是旧文件。逐文件比对，不一致即强制重写部署。
        if plugin_stale(&plugin_dir) {
            crate::logging::log("info", "内置联网搜索副本过期，重新部署");
            if let Err(cause) = deploy(app, &settings) {
                crate::logging::log("warn", &format!("内置联网搜索重部署失败：{cause}"));
                return;
            }
        }
        // 文件已就位：补/迁移 patch 行 + 对齐动态值（moliPath/fetchProxy），
        // 不重复写大文件。
        let Ok(patch) = patch_path(app, &settings) else {
            return;
        };
        let (moli_path, proxy) = fetch_patch_values(app, &settings);
        match update_patch(&patch, &moli_path, &proxy) {
            Ok(true) => crate::logging::log("info", "内置联网搜索 patch 已写入"),
            Ok(false) => {}
            Err(cause) => {
                crate::logging::log("warn", &format!("内置联网搜索 patch 写入失败：{cause}"))
            }
        }
        return;
    }
    if let Err(cause) = deploy(app, &settings) {
        crate::logging::log("warn", &format!("内置联网搜索部署失败：{cause}"));
    }
}

// ---- 内部 ----

/// profile 副本是否与编译期内置不一致（任一文件缺失或内容不同 = 过期）。
/// 内容比对是微秒级开销，换取"升级/改动必然生效"的硬保证。
fn plugin_stale(plugin_dir: &std::path::Path) -> bool {
    let files: [(&str, &str); 7] = [
        ("package.json", PLUGIN_PACKAGE_JSON),
        ("dsh/index.js", PLUGIN_INDEX),
        ("dsh/client.js", PLUGIN_CLIENT),
        ("dsh/spawnHidden.js", PLUGIN_SPAWN_HIDDEN),
        ("dsh/search-schema.json", PLUGIN_SEARCH_SCHEMA),
        ("dsh/fetch-schema.json", PLUGIN_FETCH_SCHEMA),
        ("dist/main.js", PLUGIN_CLI),
    ];
    files.iter().any(|(rel, want)| {
        let path = rel.split('/').fold(plugin_dir.to_path_buf(), |p, seg| p.join(seg));
        std::fs::read_to_string(&path)
            .map(|have| have != *want)
            .unwrap_or(true)
    })
}

fn deploy(app: &AppHandle, settings: &Settings) -> Result<()> {
    let plugin_dir = plugin_dir(app, settings)?;

    // 1. 插件文件（幂等覆盖：升级千寻 = 升级本包）。
    std::fs::create_dir_all(plugin_dir.join("dsh"))
        .and_then(|_| std::fs::create_dir_all(plugin_dir.join("dist")))
        .map_err(|cause| Error::Bridge(format!("建联网搜索目录失败：{cause}")))?;
    std::fs::write(plugin_dir.join("package.json"), PLUGIN_PACKAGE_JSON)
        .and_then(|_| std::fs::write(plugin_dir.join("dsh").join("index.js"), PLUGIN_INDEX))
        .and_then(|_| std::fs::write(plugin_dir.join("dsh").join("client.js"), PLUGIN_CLIENT))
        .and_then(|_| {
            std::fs::write(
                plugin_dir.join("dsh").join("spawnHidden.js"),
                PLUGIN_SPAWN_HIDDEN,
            )
        })
        .and_then(|_| {
            std::fs::write(
                plugin_dir.join("dsh").join("search-schema.json"),
                PLUGIN_SEARCH_SCHEMA,
            )
        })
        .and_then(|_| {
            std::fs::write(
                plugin_dir.join("dsh").join("fetch-schema.json"),
                PLUGIN_FETCH_SCHEMA,
            )
        })
        .and_then(|_| std::fs::write(plugin_dir.join("dist").join("main.js"), PLUGIN_CLI))
        .map_err(|cause| Error::Bridge(format!("写联网搜索插件文件失败：{cause}")))?;

    // 2. patch 行（幂等 + 动态值对齐）。
    let patch = patch_path(app, settings)?;
    let (moli_path, proxy) = fetch_patch_values(app, settings);
    update_patch(&patch, &moli_path, &proxy)?;

    Ok(())
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

/// fetch provider 的两个动态 patch 值：moli 三源解析（custom → managed →
/// PATH，未装为空 = 插件 fetch 走 CLI 摘要降级）+ settings.web.proxy
/// （R001-TUN：空 = 直连；"off" = 显式直连；其它传 moli --http-proxy）。
fn fetch_patch_values(app: &AppHandle, settings: &Settings) -> (String, String) {
    let moli_path = crate::tools::moli::resolve_binary(app, settings)
        .map(|(path, _)| path.display().to_string())
        .unwrap_or_default();
    let proxy = settings.web.proxy.trim().to_owned();
    (moli_path, proxy)
}

/// patch 文本级维护（与桥同一套约定，不引 YAML 依赖）。块内除静态两行外，
/// insert 条目的 config（moliPath/fetchProxy）随启动动态对齐。三种形态：
/// - 无 PATCH_MARK：`[]`/纯注释 → 重写为完整块；已有其他条目 → 文末追加；
/// - 有 PATCH_MARK 无 FETCH_MARK：旧块迁移——searchProvider 行后补插
///   fetchProvider 行 + 补插 insert config 段；
/// - 双标记齐全：只对齐 moliPath/fetchProxy 的当前值。
///
/// 内容无变化时不落盘。返回是否发生了写入。
fn update_patch(patch: &std::path::Path, moli_path: &str, proxy: &str) -> Result<bool> {
    // YAML 双引号串里反斜杠是转义符：统一正斜杠（Node/Windows 均接受）。
    let yaml = |value: &str| format!("\"{}\"", value.replace('\\', "/").replace('"', ""));
    let moli_yaml = yaml(moli_path);
    let proxy_yaml = yaml(proxy);
    let block = format!(
        "# 内置联网搜索（qx-websearch）：由千寻写入，请勿手工编辑\n\
         - id: web\n  config:\n    searchProvider: {PLUGIN_ID}\n    fetchProvider: {PLUGIN_ID}\n\n\
         - insert:\n    - id: {PLUGIN_ID}\n      name: {PLUGIN_ID}\n      config:\n        moliPath: {moli_yaml}\n        fetchProxy: {proxy_yaml}\n"
    );
    let text = std::fs::read_to_string(patch).unwrap_or_default();
    let new_text = if !text.contains(PATCH_MARK) {
        if text.trim() == "[]" {
            block
        } else {
            // 与桥一致的兜底：剥掉注释与空壳 [] 后已无实质内容时整体重写，
            // 避免 [] 与条目混排出非法 YAML（DSH 会拒绝启动）。
            let body: String = text
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed != "[]"
                })
                .collect::<Vec<_>>()
                .join("\n");
            if body.is_empty() {
                block
            } else {
                let mut out = text.trim_end().to_owned();
                out.push('\n');
                out.push_str(&block);
                out
            }
        }
    } else {
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        // 1. 旧块迁移：fetchProvider 行紧跟 searchProvider 行补插。
        if !text.contains(FETCH_MARK) {
            if let Some(index) = lines
                .iter()
                .position(|line| line.trim() == format!("searchProvider: {PLUGIN_ID}"))
            {
                lines.insert(index + 1, format!("    fetchProvider: {PLUGIN_ID}"));
            }
        }
        // 2. insert 条目的 config 动态值。区间限定在本条目内
        //    （至下一条顶格 `- ` 行或文件尾），不误伤同文件其他条目的
        //    同名键（如桥条目自己的 moliPath）。
        let entry_start = lines.iter().position(|line| {
            line.starts_with("    - ") && line.trim_start() == format!("- id: {PLUGIN_ID}")
        });
        if let Some(start) = entry_start {
            let end = lines[start + 1..]
                .iter()
                .position(|line| line.starts_with("- "))
                .map(|offset| start + 1 + offset)
                .unwrap_or(lines.len());
            if let Some(name_offset) = lines[start..end]
                .iter()
                .position(|line| line.trim_start() == format!("name: {PLUGIN_ID}"))
            {
                let name_index = start + name_offset;
                if let Some(config_offset) = lines[name_index + 1..end]
                    .iter()
                    .position(|line| line.trim_start() == "config:")
                {
                    let config_index = name_index + 1 + config_offset;
                    upsert_line(
                        &mut lines,
                        config_index + 1,
                        end,
                        "moliPath:",
                        &format!("        moliPath: {moli_yaml}"),
                        "fetchProxy:",
                    );
                    upsert_line(
                        &mut lines,
                        config_index + 1,
                        end,
                        "fetchProxy:",
                        &format!("        fetchProxy: {proxy_yaml}"),
                        "moliPath:",
                    );
                } else {
                    lines.insert(name_index + 1, format!("        fetchProxy: {proxy_yaml}"));
                    lines.insert(name_index + 1, format!("        moliPath: {moli_yaml}"));
                    lines.insert(name_index + 1, "      config:".to_owned());
                }
            }
        }
        let mut out = lines.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        out
    };
    if new_text == text {
        return Ok(false);
    }
    std::fs::write(patch, new_text)
        .map(|_| true)
        .map_err(|cause| Error::Bridge(format!("写 cordis.patch.yml 失败：{cause}")))
}

/// 区间 [start, end) 内：替换第一条 `key` 开头的行；缺失则锚在 `anchor`
/// 行后（或区间末尾）插入。产出确定，值不变即幂等。
fn upsert_line(
    lines: &mut Vec<String>,
    start: usize,
    end: usize,
    key: &str,
    new_line: &str,
    anchor: &str,
) {
    if let Some(offset) = lines[start..end]
        .iter()
        .position(|line| line.trim_start().starts_with(key))
    {
        lines[start + offset] = new_line.to_owned();
        return;
    }
    let insert_at = lines[start..end]
        .iter()
        .position(|line| line.trim_start().starts_with(anchor))
        .map(|offset| start + offset + 1)
        .unwrap_or(end);
    lines.insert(insert_at, new_line.to_owned());
}

fn status(app: &AppHandle, settings: &Settings) -> WebsearchStatus {
    let deployed = plugin_dir(app, settings)
        .map(|dir| dir.join("dsh").join("index.js").is_file())
        .unwrap_or(false);
    let patch_text = patch_path(app, settings)
        .and_then(|path| std::fs::read_to_string(path).map_err(|_| Error::Bridge(String::new())))
        .unwrap_or_default();
    let patch_entry = patch_text.contains(PATCH_MARK);
    WebsearchStatus {
        deployed,
        patch_entry,
        plugin_dir: plugin_dir(app, settings)
            .map(|dir| dir.display().to_string())
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

    #[test]
    fn patch三态维护幂等且不破坏既有条目() {
        let dir = std::env::temp_dir().join(format!("qx-ws-patch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let patch = dir.join("cordis.patch.yml");

        // 空模板 → 完整块（含 fetchProvider 与 insert config）。
        std::fs::write(&patch, "[]\n").unwrap();
        assert!(update_patch(&patch, "D:/tools/moli.exe", "").unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("searchProvider: qx-websearch"));
        assert!(text.contains("fetchProvider: qx-websearch"));
        assert!(text.contains(r#"moliPath: "D:/tools/moli.exe""#));
        assert!(text.contains(r#"fetchProxy: """#));
        assert!(text.contains("id: qx-websearch"));
        assert!(!text.contains("[]"), "[] 残留会产出非法 YAML：{text}");

        // 再跑一次 → 幂等（内容不变不落盘），行不重复。
        assert!(!update_patch(&patch, "D:/tools/moli.exe", "").unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert_eq!(text.matches("searchProvider:").count(), 1);
        assert_eq!(text.matches("fetchProvider:").count(), 1);
        assert_eq!(text.matches("id: qx-websearch").count(), 1);
        assert_eq!(text.matches("moliPath:").count(), 1);

        // 桥条目已在（真实 dev 形态）→ 文末追加，桥条目完好。
        let bridge = "# qx-bridge（千寻笔记桥）：由千寻写入，请勿手工编辑\n- insert:\n    - id: qx-bridge\n      name: qx-bridge\n      config:\n        vault: \"D:/v\"\n";
        std::fs::write(&patch, bridge).unwrap();
        assert!(update_patch(&patch, "", "").unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("id: qx-bridge"));
        assert!(text.contains("searchProvider: qx-websearch"));
        assert_eq!(text.matches("id: qx-websearch").count(), 1);

        // DSH 默认模板（注释 + []）→ 重写，[] 不残留。
        std::fs::write(&patch, "# Your patch layer for this dsh profile:\n[]\n").unwrap();
        assert!(update_patch(&patch, "", "").unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(!text.contains("[]"));
        assert!(text.contains("searchProvider: qx-websearch"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 旧块迁移补fetch_provider行与config段() {
        let dir = std::env::temp_dir().join(format!("qx-ws-migr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let patch = dir.join("cordis.patch.yml");
        // 真实旧形态（首批安装用户手中的 patch）：无 fetchProvider、无 config。
        let legacy = "# 内置联网搜索（qx-websearch）：由千寻写入，请勿手工编辑\n- id: web\n  config:\n    searchProvider: qx-websearch\n\n- insert:\n    - id: qx-websearch\n      name: qx-websearch\n";
        std::fs::write(&patch, legacy).unwrap();
        assert!(update_patch(&patch, "D:/tools/moli.exe", "socks5://127.0.0.1:1080").unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(text.contains("    fetchProvider: qx-websearch"));
        assert!(text.contains("      config:"));
        assert!(text.contains(r#"moliPath: "D:/tools/moli.exe""#));
        assert!(text.contains(r#"fetchProxy: "socks5://127.0.0.1:1080""#));
        assert_eq!(text.matches("fetchProvider:").count(), 1);
        assert_eq!(text.matches("moliPath:").count(), 1);

        // 迁移后再跑 → 幂等。
        assert!(!update_patch(&patch, "D:/tools/moli.exe", "socks5://127.0.0.1:1080").unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 动态值对齐不误伤其他条目的同名键() {
        let dir = std::env::temp_dir().join(format!("qx-ws-dyn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let patch = dir.join("cordis.patch.yml");
        // 桥条目有自己的 moliPath，且 websearch 已是新版形态：只应动本条目的值。
        let mixed = "# qx-bridge：由千寻写入\n- insert:\n    - id: qx-bridge\n      name: qx-bridge\n      config:\n        vault: \"D:/v\"\n        moliPath: \"D:/bridge-moli.exe\"\n# 内置联网搜索（qx-websearch）：由千寻写入，请勿手工编辑\n- id: web\n  config:\n    searchProvider: qx-websearch\n    fetchProvider: qx-websearch\n\n- insert:\n    - id: qx-websearch\n      name: qx-websearch\n      config:\n        moliPath: \"D:/old-moli.exe\"\n        fetchProxy: \"\"\n";
        std::fs::write(&patch, mixed).unwrap();
        assert!(update_patch(&patch, "D:/new-moli.exe", "").unwrap());
        let text = std::fs::read_to_string(&patch).unwrap();
        assert!(
            text.contains(r#"moliPath: "D:/bridge-moli.exe""#),
            "桥的 moliPath 不得被动：{text}"
        );
        assert!(text.contains(r#"moliPath: "D:/new-moli.exe""#));
        assert_eq!(text.matches("moliPath:").count(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 内容指纹自愈缺文件或旧内容判定过期() {
        let dir = std::env::temp_dir().join(format!("qx-ws-stale-{}", std::process::id()));
        let dsh = dir.join("dsh");
        let dist = dir.join("dist");
        std::fs::create_dir_all(&dsh).unwrap();
        std::fs::create_dir_all(&dist).unwrap();

        // 全新目录：缺文件 = 过期。
        assert!(plugin_stale(&dir));

        // 全部写入当前内置内容 → 新鲜。
        std::fs::write(dir.join("package.json"), PLUGIN_PACKAGE_JSON).unwrap();
        std::fs::write(dsh.join("index.js"), PLUGIN_INDEX).unwrap();
        std::fs::write(dsh.join("client.js"), PLUGIN_CLIENT).unwrap();
        std::fs::write(dsh.join("spawnHidden.js"), PLUGIN_SPAWN_HIDDEN).unwrap();
        std::fs::write(dsh.join("search-schema.json"), PLUGIN_SEARCH_SCHEMA).unwrap();
        std::fs::write(dsh.join("fetch-schema.json"), PLUGIN_FETCH_SCHEMA).unwrap();
        std::fs::write(dist.join("main.js"), PLUGIN_CLI).unwrap();
        assert!(!plugin_stale(&dir));

        // 只改一个文件（模拟 debug 改 assets / 升级千寻）→ 过期。
        std::fs::write(dsh.join("index.js"), "// stale\n").unwrap();
        assert!(plugin_stale(&dir));

        std::fs::remove_dir_all(&dir).ok();
    }
}

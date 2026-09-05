//! 设置系统（架构文档 §4.1）：settings.json 是唯一持久化事实，
//! 本模块独占读写。命令层只做「补丁合并 → 校验 → 原子落盘」。
//!
//! 两个不变量：
//! - schemaVersion 与 window.geometry 由本模块独占管理，前端补丁
//!   里的越权字段会被剥除（见 `strip_managed_fields`）；
//! - 写入永远走 `atomic::write`，磁盘上任一时刻都是完整文件。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::AppHandle;

pub mod commands;

use crate::atomic;
use crate::error::{Error, Result};
use crate::logging;
use crate::paths;

pub const SCHEMA_VERSION: u32 = 1;

/// 端口合法区间：避开系统保留段；0 是动态端口，与 ADR-002（固定端口）相悖。
pub const PORT_MIN: u16 = 1024;
pub const PORT_MAX: u16 = 65535;

/// ADR-002：DSH 固定端口默认值。
fn default_port() -> u16 {
    17300
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowSettings {
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub geometry: Option<Geometry>,
}

impl WindowSettings {
    /// 默认值与 serde(default) 是两回事：serde 只负责「字段缺失」，
    /// 这里给出语义上的默认（关闭到托盘开、最小化启动关）。
    fn semantic_default() -> Self {
        Self {
            close_to_tray: true,
            start_minimized: false,
            geometry: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DshVersionStrategy {
    #[default]
    Pinned,
    Existing,
}

/// DSH HOME 策略（ADR-009）：isolated = 应用数据目录下的独立 DSH_HOME，
/// 与系统 ~/.dsh（可能承载外部实例）完全隔离；system = 直接用系统 ~/.dsh。
pub const DSH_HOME_ISOLATED: &str = "isolated";
pub const DSH_HOME_SYSTEM: &str = "system";

fn default_dsh_home() -> String {
    DSH_HOME_ISOLATED.to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DshSettings {
    pub port: u16,
    pub allow_random_fallback: bool,
    pub version_strategy: DshVersionStrategy,
    pub autostart: bool,
    /// ADR-009：见 `DSH_HOME_ISOLATED` 常量说明。
    pub home: String,
}

impl Default for DshSettings {
    fn default() -> Self {
        Self {
            port: default_port(),
            allow_random_fallback: false,
            version_strategy: DshVersionStrategy::default(),
            autostart: true,
            home: default_dsh_home(),
        }
    }
}

/// 镜像源策略（架构 §4.4）。值为白名单枚举或自定义 https URL（registry）。
pub const NODE_BINARY_AUTO: &str = "auto";
pub const NODE_BINARY_OFFICIAL: &str = "official";
pub const NODE_BINARY_NPMMIRROR: &str = "npmmirror";
pub const NPM_REGISTRY_OFFICIAL: &str = "official";
pub const NPM_REGISTRY_NPMMIRROR: &str = "npmmirror";

fn default_node_binary() -> String {
    NODE_BINARY_AUTO.to_owned()
}

fn default_npm_registry() -> String {
    NPM_REGISTRY_NPMMIRROR.to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MirrorsSettings {
    /// auto = 官方优先失败落 npmmirror；official / npmmirror = 强制。
    pub node_binary: String,
    /// official / npmmirror / 自定义 https:// URL。
    pub npm_registry: String,
}

impl Default for MirrorsSettings {
    fn default() -> Self {
        Self {
            node_binary: default_node_binary(),
            npm_registry: default_npm_registry(),
        }
    }
}

impl MirrorsSettings {
    /// 解析为真实 registry URL。自定义值必须是 http(s) URL。
    pub fn registry_url(&self) -> String {
        match self.npm_registry.as_str() {
            NPM_REGISTRY_OFFICIAL | "" => "https://registry.npmjs.org/".to_owned(),
            NPM_REGISTRY_NPMMIRROR => "https://registry.npmmirror.com/".to_owned(),
            custom if custom.starts_with("http://") || custom.starts_with("https://") => {
                custom.to_owned()
            }
            // validate 已把白名单外的值挡在门外；这里是防御式兜底。
            _ => "https://registry.npmmirror.com/".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchSettings {
    /// 根目录历史（最近优先，去重，上限 8）。搜索页 datalist 供选择。
    pub root_history: Vec<String>,
}

/// 截屏热键（M3）：Tauri 快捷键语法，如 "Ctrl+Shift+A"。空串 = 不注册。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HotkeysSettings {
    pub screenshot: String,
}

impl Default for HotkeysSettings {
    fn default() -> Self {
        // Alt+A 与微信截屏冲突，默认改用 Ctrl+Shift+A。
        Self {
            screenshot: "Ctrl+Shift+A".to_owned(),
        }
    }
}

/// 终端偏好（M4）：新建标签生效。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TerminalSettings {
    /// auto = pwsh 优先、powershell 兜底；否则为可执行文件路径。
    pub shell: String,
    pub font_size: u32,
    pub scrollback: u32,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: "auto".to_owned(),
            font_size: 13,
            scrollback: 5000,
        }
    }
}

/// 笔记库（M5）：一个目录 = 一个库（ADR-006 纯文件）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotesSettings {
    /// 库目录绝对路径；空串 = 尚未初始化（首用引导创建）。
    pub vault_dir: String,
}

/// 根目录历史上限：再多就没有记忆价值了。
const ROOT_HISTORY_LIMIT: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub schema_version: u32,
    pub theme: ThemePreference,
    pub window: WindowSettings,
    pub dsh: DshSettings,
    pub mirrors: MirrorsSettings,
    pub search: SearchSettings,
    pub hotkeys: HotkeysSettings,
    pub terminal: TerminalSettings,
    pub notes: NotesSettings,
    pub remote: crate::remote::RemoteSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            theme: ThemePreference::default(),
            window: WindowSettings::semantic_default(),
            dsh: DshSettings::default(),
            mirrors: MirrorsSettings::default(),
            search: SearchSettings::default(),
            hotkeys: HotkeysSettings::default(),
            terminal: TerminalSettings::default(),
            notes: NotesSettings::default(),
            remote: crate::remote::RemoteSettings::default(),
        }
    }
}

fn validate(settings: &Settings) -> Result<()> {
    if settings.schema_version != SCHEMA_VERSION {
        return Err(Error::SettingsInvalid(format!(
            "不支持的 schemaVersion：{}（当前支持 {SCHEMA_VERSION}）",
            settings.schema_version
        )));
    }
    if !(PORT_MIN..=PORT_MAX).contains(&settings.dsh.port) {
        return Err(Error::SettingsInvalid(format!(
            "端口必须在 {PORT_MIN}–{PORT_MAX} 之间，当前为 {}",
            settings.dsh.port
        )));
    }
    if settings.dsh.home != DSH_HOME_ISOLATED && settings.dsh.home != DSH_HOME_SYSTEM {
        return Err(Error::SettingsInvalid(format!(
            "dsh.home 只能是 isolated 或 system，当前为 {}",
            settings.dsh.home
        )));
    }
    let node_sources = [
        NODE_BINARY_AUTO,
        NODE_BINARY_OFFICIAL,
        NODE_BINARY_NPMMIRROR,
    ];
    if !node_sources.contains(&settings.mirrors.node_binary.as_str()) {
        return Err(Error::SettingsInvalid(format!(
            "mirrors.nodeBinary 只能是 auto/official/npmmirror，当前为 {}",
            settings.mirrors.node_binary
        )));
    }
    let registry = settings.mirrors.npm_registry.as_str();
    let known_registry = matches!(
        registry,
        NPM_REGISTRY_OFFICIAL | NPM_REGISTRY_NPMMIRROR | ""
    );
    if !known_registry && !(registry.starts_with("http://") || registry.starts_with("https://")) {
        return Err(Error::SettingsInvalid(
            "mirrors.npmRegistry 只能是 official/npmmirror 或以 http(s):// 开头的自定义地址"
                .to_owned(),
        ));
    }
    if settings.search.root_history.len() > ROOT_HISTORY_LIMIT {
        return Err(Error::SettingsInvalid(format!(
            "search.rootHistory 最多 {ROOT_HISTORY_LIMIT} 条，当前 {}",
            settings.search.root_history.len()
        )));
    }
    if settings.search.root_history.iter().any(String::is_empty) {
        return Err(Error::SettingsInvalid(
            "search.rootHistory 不能有空条目".to_owned(),
        ));
    }
    if !(8..=32).contains(&settings.terminal.font_size) {
        return Err(Error::SettingsInvalid(format!(
            "terminal.fontSize 必须在 8–32 之间，当前为 {}",
            settings.terminal.font_size
        )));
    }
    if !(100..=100_000).contains(&settings.terminal.scrollback) {
        return Err(Error::SettingsInvalid(format!(
            "terminal.scrollback 必须在 100–100000 之间，当前为 {}",
            settings.terminal.scrollback
        )));
    }
    if settings.terminal.shell.trim().is_empty() {
        return Err(Error::SettingsInvalid(
            "terminal.shell 不能为空（auto = 自动探测）".to_owned(),
        ));
    }
    if settings.remote.enabled {
        if settings.remote.bind_ip.trim().is_empty() {
            return Err(Error::SettingsInvalid(
                "remote.enabled 开启时必须选择绑定网卡地址".to_owned(),
            ));
        }
        if settings.remote.bind_ip.starts_with("127.") {
            return Err(Error::SettingsInvalid(
                "remote.bindIp 不能是回环地址（网关必须落在可达网卡上）".to_owned(),
            ));
        }
        if !(PORT_MIN..=PORT_MAX).contains(&settings.remote.port) {
            return Err(Error::SettingsInvalid(format!(
                "remote.port 必须在 {PORT_MIN}–{PORT_MAX} 之间，当前为 {}",
                settings.remote.port
            )));
        }
    }
    for (index, device) in settings.remote.devices.iter().enumerate() {
        if device.id.trim().is_empty() || device.token.len() != 64 {
            return Err(Error::SettingsInvalid(format!(
                "remote.devices[{index}] 形态不合法（id/token）"
            )));
        }
    }
    Ok(())
}

/// 读取设置。文件缺失 → 默认值并落盘；解析或校验失败 → 损坏文件改名
/// 保留证据，回退默认值。设置损坏不该阻止外壳启动。
pub fn load(app: &AppHandle) -> Result<Settings> {
    let path = paths::settings_path(app)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => match parse(&text) {
            Ok(settings) => Ok(settings),
            Err(error) => {
                logging::log("warn", &format!("设置文件损坏，回退默认值：{error}"));
                let backup = path.with_extension("json.corrupt");
                let _ = std::fs::rename(&path, &backup);
                let fresh = Settings::default();
                save(&path, &fresh)?;
                Ok(fresh)
            }
        },
        Err(_) if !path.exists() => {
            // 首次运行：写出默认设置，让用户能直接找到并理解这个文件。
            let fresh = Settings::default();
            save(&path, &fresh)?;
            Ok(fresh)
        }
        Err(error) => Err(Error::SettingsRead(error.to_string())),
    }
}

/// 纯解析路径，供测试直接使用。
fn parse(text: &str) -> Result<Settings> {
    let settings: Settings =
        serde_json::from_str(text).map_err(|error| Error::SettingsInvalid(error.to_string()))?;
    validate(&settings)?;
    Ok(settings)
}

pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    let text = serde_json::to_string_pretty(settings)
        .map_err(|error| Error::SettingsWrite(error.to_string()))?;
    atomic::write(path, text.as_bytes()).map_err(|error| Error::SettingsWrite(error.to_string()))
}

/// 剥除补丁里不允许前端修改的字段：schemaVersion 与窗口几何。
/// 前端 contract.ts 类型层面已禁止，这里是边界上的第二道防线
/// （编码规范 §7：不信任前端输入）。
fn strip_managed_fields(patch: &mut Map<String, Value>) {
    patch.remove("schemaVersion");
    if let Some(Value::Object(window)) = patch.get_mut("window") {
        window.remove("geometry");
    }
}

/// 深合并（对象递归、标量与数组整体替换），合并结果再走一次完整反序列化
/// 与校验——类型错误与越权值都会在这里被拒绝，而不是静默写入。
pub fn apply_patch(current: &Settings, patch: &Value) -> Result<Settings> {
    let mut document =
        serde_json::to_value(current).map_err(|error| Error::SettingsInvalid(error.to_string()))?;
    let Value::Object(mut fields) = patch.clone() else {
        return Err(Error::SettingsInvalid("设置补丁必须是对象".to_owned()));
    };
    strip_managed_fields(&mut fields);
    if let Value::Object(target) = &mut document {
        merge(target, fields);
    }
    let next: Settings = serde_json::from_value(document)
        .map_err(|error| Error::SettingsInvalid(error.to_string()))?;
    validate(&next)?;
    Ok(next)
}

fn merge(target: &mut Map<String, Value>, patch: Map<String, Value>) {
    for (key, value) in patch {
        match (target.get_mut(&key), value) {
            (Some(Value::Object(child)), Value::Object(patch_child)) => {
                merge(child, patch_child);
            }
            (_, value) => {
                target.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 空对象解析为默认设置() {
        let settings = parse("{}").unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.dsh.port, 17300);
        assert!(settings.window.close_to_tray);
        assert_eq!(settings.dsh.home, "isolated");
        assert_eq!(settings.mirrors.npm_registry, "npmmirror");
    }

    #[test]
    fn 完整文件往返一致() {
        // pinnedVersion 是历史字段：DSH 版本现在由千寻硬编码，样例里
        // 保留旧值验证向后兼容（未知字段被忽略，round-trip 后消失）。
        let text = r#"{
            "schemaVersion": 1,
            "theme": "dark",
            "window": { "closeToTray": false, "startMinimized": true, "geometry": null },
            "dsh": { "port": 2000, "allowRandomFallback": true, "versionStrategy": "existing", "autostart": false, "home": "system", "pinnedVersion": "0.1.1-rc.2" },
            "mirrors": { "nodeBinary": "official", "npmRegistry": "https://r.example.com/" }
        }"#;
        let settings = parse(text).unwrap();
        assert_eq!(settings.theme, ThemePreference::Dark);
        assert_eq!(settings.dsh.port, 2000);
        assert_eq!(settings.dsh.home, "system");
        assert_eq!(settings.mirrors.registry_url(), "https://r.example.com/");
        // 序列化再解析不丢信息。
        let again = parse(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(settings, again);
    }

    #[test]
    fn 未知schema版本被拒绝() {
        assert!(parse(r#"{"schemaVersion": 2}"#).is_err());
    }

    #[test]
    fn 越界端口被拒绝() {
        assert!(parse(r#"{"dsh": {"port": 80}}"#).is_err());
        assert!(parse(r#"{"dsh": {"port": 70000}}"#).is_err());
    }

    #[test]
    fn 非法home与镜像值被拒绝() {
        assert!(parse(r#"{"dsh": {"home": "shared"}}"#).is_err());
        assert!(parse(r#"{"mirrors": {"nodeBinary": "cnpm"}}"#).is_err());
        assert!(parse(r#"{"mirrors": {"npmRegistry": "npmmirror.com"}}"#).is_err());
    }

    #[test]
    fn registry解析覆盖三种形态() {
        let official = MirrorsSettings {
            npm_registry: "official".into(),
            ..MirrorsSettings::default()
        };
        assert_eq!(official.registry_url(), "https://registry.npmjs.org/");
        let mirror = MirrorsSettings::default();
        assert_eq!(mirror.registry_url(), "https://registry.npmmirror.com/");
    }

    #[test]
    fn 补丁只改目标字段其余保留() {
        let current = Settings::default();
        let patch = serde_json::json!({ "dsh": { "port": 2000 } });
        let next = apply_patch(&current, &patch).unwrap();
        assert_eq!(next.dsh.port, 2000);
        // 同域其他字段与跨域字段原样保留。
        assert!(next.dsh.autostart);
        assert!(next.window.close_to_tray);
    }

    #[test]
    fn 补丁里的越权字段被剥除() {
        let current = Settings::default();
        let patch = serde_json::json!({
            "schemaVersion": 99,
            "window": { "geometry": { "x": 1, "y": 2, "width": 3, "height": 4, "maximized": false } }
        });
        let next = apply_patch(&current, &patch).unwrap();
        assert_eq!(next.schema_version, SCHEMA_VERSION);
        assert_eq!(next.window.geometry, None);
    }

    #[test]
    fn 补丁类型错误被拒绝且不落盘() {
        let current = Settings::default();
        let patch = serde_json::json!({ "theme": "solarized" });
        assert!(apply_patch(&current, &patch).is_err());
        let patch = serde_json::json!({ "dsh": { "port": "17300" } });
        assert!(apply_patch(&current, &patch).is_err());
    }

    #[test]
    fn 默认截屏热键不与微信冲突() {
        assert_eq!(HotkeysSettings::default().screenshot, "Ctrl+Shift+A");
    }
}

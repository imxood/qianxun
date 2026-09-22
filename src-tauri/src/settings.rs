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
    /// auto = 国内 npmmirror 优先失败落官方；official / npmmirror = 强制。
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

/// 联网搜索的一个引擎（R001 D2）。`kind` 决定 URL 模板；`searxng` /
/// `custom` 必须带 https endpoint。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebEngineSettings {
    /// 用户可见标识（唯一）。
    pub id: String,
    /// 引擎类型：bing / baidu / duckduckgo / searxng。
    pub kind: String,
    /// searxng 实例地址（其余类型留空）。
    pub endpoint: String,
}

impl Default for WebEngineSettings {
    fn default() -> Self {
        Self {
            id: "bing".to_owned(),
            kind: "bing".to_owned(),
            endpoint: String::new(),
        }
    }
}

/// 联网搜索设置（R001）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebSettings {
    /// 默认引擎 id（必须能在 engines 里找到）。
    pub default_engine: String,
    /// 引擎清单（上限 8，id 唯一）。
    pub engines: Vec<WebEngineSettings>,
    /// 单次搜索返回条数（1–50）。
    pub result_limit: u32,
    /// moli 抓取代理（R001-TUN）：空 = 直连；"off" = 显式直连；
    /// 其它 = 传给 moli `--http-proxy`（如 socks5://127.0.0.1:1080）。
    /// TUN/fake-ip 网络下域名会被解析成 198.18.0.0/15 假 IP，被 moli
    /// private-network 守卫拦截；走代理时由代理侧解析，守卫不适用。
    pub proxy: String,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self {
            default_engine: "duckduckgo".to_owned(),
            engines: vec![
                WebEngineSettings {
                    id: "duckduckgo".to_owned(),
                    kind: "duckduckgo".to_owned(),
                    endpoint: String::new(),
                },
                WebEngineSettings {
                    id: "bing".to_owned(),
                    kind: "bing".to_owned(),
                    endpoint: String::new(),
                },
            ],
            result_limit: 10,
            proxy: String::new(),
        }
    }
}

/// Moli 无头浏览器的本机管理设置（R001 D8/D10）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MoliSettings {
    /// false = 桥内 web 能力走 Node fetch 降级路径。
    pub enabled: bool,
    /// 锁定版本（检测时与实际 `--version` 比对展示）。
    pub pinned_version: String,
    /// 自备二进制绝对路径；空 = 使用数据目录 tools/moli/ 下的受管安装。
    pub binary_path: String,
}

impl Default for MoliSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            pinned_version: "1.1.9".to_owned(),
            binary_path: String::new(),
        }
    }
}

/// 插件清单里的一项（08 设计 §2.1）：装成功的 name@version。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PinnedPlugin {
    /// npm 包名；内核包（@deepseek-ai/ 前缀）不入清单。
    pub name: String,
    /// 安装时的精确版本（registry detail 复核值）。
    pub version: String,
}

impl PinnedPlugin {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// 插件清单域（08 设计 §2）：`plugins.pinned` 随 settings.json 进备份包。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginsSettings {
    pub pinned: Vec<PinnedPlugin>,
}

impl PluginsSettings {
    /// 记入/更新一项并保持去重 + 按包名小写排序（文件 diff 干净）。
    pub fn pin(&mut self, plugin: PinnedPlugin) {
        self.pinned.retain(|entry| entry.name != plugin.name);
        self.pinned.push(plugin);
        self.normalize();
    }

    /// 移除一项；不存在是 no-op。
    pub fn unpin(&mut self, name: &str) {
        self.pinned.retain(|entry| entry.name != name);
        self.normalize();
    }

    /// 内核包混入检测（08 设计 §7）：@deepseek-ai/ 前缀不走清单。
    pub fn is_builtin(name: &str) -> bool {
        name.starts_with(crate::market::BUILTIN_PREFIX)
    }

    fn normalize(&mut self) {
        self.pinned
            .sort_by_key(|entry| entry.name.to_ascii_lowercase());
        self.pinned.dedup_by(|a, b| a.name == b.name);
    }
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
    /// 联网搜索域（R001）。
    pub web: WebSettings,
    pub hotkeys: HotkeysSettings,
    pub notes: NotesSettings,
    pub remote: crate::remote::RemoteSettings,
    /// 插件清单（08 设计 §2）：装成功的 name@version，随备份包走。
    pub plugins: PluginsSettings,
    /// 工具管理（R001）：Moli 无头浏览器。
    pub tools: ToolsSettings,
}

/// 工具管理设置（R001 D8）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolsSettings {
    /// Moli 无头浏览器。
    pub moli: MoliSettings,
    /// playwright-core 的加载路径（R001 browser_* 工具）；空 = 桥内
    /// 按 tools/playwright-core/ 约定位置与 require 链自动探测。
    pub playwright_core_path: String,
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
            web: WebSettings::default(),
            hotkeys: HotkeysSettings::default(),
            notes: NotesSettings::default(),
            remote: crate::remote::RemoteSettings::default(),
            plugins: PluginsSettings::default(),
            tools: ToolsSettings::default(),
        }
    }
}

/// 联网搜索段校验（R001）：kind 白名单、searxng 必带 https endpoint、
/// id 非空且唯一、default_engine 必须存在、resultLimit 1–50。
/// baidu 已移除（对无 cookie 抓取硬反爬，实测不可用）。
fn validate_web(web: &WebSettings) -> Result<()> {
    const KINDS: [&str; 3] = ["bing", "duckduckgo", "searxng"];
    if web.engines.is_empty() {
        return Err(Error::SettingsInvalid("web.engines 不能为空".to_owned()));
    }
    if web.engines.len() > 8 {
        return Err(Error::SettingsInvalid(
            "web.engines 最多 8 个引擎".to_owned(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for (index, engine) in web.engines.iter().enumerate() {
        if engine.id.trim().is_empty() {
            return Err(Error::SettingsInvalid(format!(
                "web.engines[{index}].id 不能为空"
            )));
        }
        if !ids.insert(engine.id.as_str()) {
            return Err(Error::SettingsInvalid(format!(
                "web.engines[{index}].id 重复：{}",
                engine.id
            )));
        }
        if !KINDS.contains(&engine.kind.as_str()) {
            return Err(Error::SettingsInvalid(format!(
                "web.engines[{index}].kind 只能是 bing/duckduckgo/searxng，当前为 {}",
                engine.kind
            )));
        }
        if engine.kind == "searxng" && !engine.endpoint.starts_with("https://") {
            return Err(Error::SettingsInvalid(format!(
                "web.engines[{index}]（searxng）必须提供 https:// 实例地址"
            )));
        }
        if engine.kind != "searxng" && !engine.endpoint.is_empty() {
            return Err(Error::SettingsInvalid(format!(
                "web.engines[{index}]（{}）不需要 endpoint",
                engine.kind
            )));
        }
    }
    if !web
        .engines
        .iter()
        .any(|engine| engine.id == web.default_engine)
    {
        return Err(Error::SettingsInvalid(format!(
            "web.defaultEngine 不在 engines 清单里：{}",
            web.default_engine
        )));
    }
    if !(1..=50).contains(&web.result_limit) {
        return Err(Error::SettingsInvalid(format!(
            "web.resultLimit 必须在 1–50 之间，当前为 {}",
            web.result_limit
        )));
    }
    // proxy：空或 "off"（显式直连）之外的值必须是 moli 认的代理 URL。
    let proxy = web.proxy.trim();
    if !proxy.is_empty() && !proxy.eq_ignore_ascii_case("off") {
        const SCHEMES: [&str; 6] = [
            "http://",
            "https://",
            "socks5://",
            "socks5h://",
            "socks4://",
            "socks4a://",
        ];
        let lower = proxy.to_ascii_lowercase();
        let has_scheme_and_host = SCHEMES
            .iter()
            .any(|scheme| lower.len() > scheme.len() && lower.starts_with(scheme));
        if !has_scheme_and_host {
            return Err(Error::SettingsInvalid(format!(
                "web.proxy 必须是 http(s):// 或 socks4(a)/socks5(h):// 代理地址、off（显式直连）或留空，当前为：{proxy}"
            )));
        }
    }
    Ok(())
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
    validate_web(&settings.web)?;
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
    let mut settings: Settings =
        serde_json::from_str(text).map_err(|error| Error::SettingsInvalid(error.to_string()))?;
    // 先迁移后校验：迁移会把旧文件里的已移除引擎（如 baidu）清掉，
    // 若先校验，老文件会被整体拒掉并触发「损坏回退默认」，误伤其余设置。
    settings = migrate(settings);
    validate(&settings)?;
    Ok(settings)
}

/// 字段级就地迁移（读入后、使用前）：旧默认值跟走到新默认。
/// 用户显式配置过的其它值原样保留。
fn migrate(mut settings: Settings) -> Settings {
    // R001：百度对无 cookie 的无头抓取硬反爬（超时/空内容），已从引擎
    // 清单移除——旧设置文件里的 baidu 引擎就地剔除，默认引擎跟走：
    // 优先 duckduckgo（实测相关性最好），否则清单首位。
    settings.web.engines.retain(|engine| engine.kind != "baidu");
    if !settings
        .web
        .engines
        .iter()
        .any(|engine| engine.id == settings.web.default_engine)
    {
        settings.web.default_engine = settings
            .web
            .engines
            .iter()
            .find(|engine| engine.kind == "duckduckgo")
            .or_else(|| settings.web.engines.first())
            .map(|engine| engine.id.clone())
            .unwrap_or_else(|| "duckduckgo".to_owned());
    }
    // 网关端口：17400 是历史默认；持久化过旧默认的设置文件迁移到
    // 按构建模式的新默认（release 23090 / debug 23091）。
    // 备份还原场景下也会带进另一构建模式（debug ↔ release）的默认值——
    // 那个端口此刻正被原实例占用，会撞 EADDRINUSE；同样迁到当前模式默认。
    let current_default = crate::remote::default_gateway_port();
    let other_mode_default = if cfg!(debug_assertions) { 23090 } else { 23091 };
    if settings.remote.port == crate::remote::LEGACY_GATEWAY_PORT
        || settings.remote.port == other_mode_default
    {
        settings.remote.port = current_default;
    }
    settings
}

pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    let text = serde_json::to_string_pretty(settings)
        .map_err(|error| Error::SettingsWrite(error.to_string()))?;
    atomic::write(path, text.as_bytes()).map_err(|error| Error::SettingsWrite(error.to_string()))
}

/// 就地更新设置并落盘（08 设计：插件清单由后端命令在安装/卸载成功后维护）。
/// 先落盘再更新内存，失败时内存仍是旧值（同 settings_update 的顺序）。
pub fn update<F>(app: &AppHandle, change: F) -> Result<()>
where
    F: FnOnce(&mut Settings),
{
    use tauri::Manager;
    let path = paths::settings_path(app)?;
    let state = app.state::<crate::AppState>();
    let mut guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut next = guard.clone();
    change(&mut next);
    save(&path, &next)?;
    *guard = next;
    Ok(())
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

    // ---- R001：web / tools.moli 设置段 ----

    #[test]
    fn web段缺省解析为默认引擎清单() {
        let settings = parse("{}").unwrap();
        assert_eq!(settings.web.default_engine, "duckduckgo");
        assert_eq!(settings.web.engines.len(), 2);
        assert_eq!(settings.web.engines[0].kind, "duckduckgo");
        assert_eq!(settings.web.engines[1].kind, "bing");
        assert_eq!(settings.web.result_limit, 10);
        assert!(settings.web.proxy.is_empty());
        assert!(settings.tools.moli.enabled);
        assert_eq!(settings.tools.moli.pinned_version, "1.1.9");
        assert!(settings.tools.moli.binary_path.is_empty());
    }

    #[test]
    fn web段迁移_剔除baidu并跟走默认引擎() {
        // 旧默认清单（bing/baidu/duckduckgo）+ 默认引擎指向 baidu：
        // 迁移应剔除 baidu、默认引擎落到清单首位。
        let settings = parse(
            r#"{"web": {"defaultEngine": "baidu", "engines": [
                {"id": "bing", "kind": "bing"},
                {"id": "baidu", "kind": "baidu"},
                {"id": "duckduckgo", "kind": "duckduckgo"}
            ]}}"#,
        )
        .unwrap();
        assert_eq!(settings.web.engines.len(), 2);
        assert!(settings
            .web
            .engines
            .iter()
            .all(|engine| engine.kind != "baidu"));
        assert_eq!(settings.web.default_engine, "duckduckgo");
    }

    #[test]
    fn web段旧文件零迁移可读() {
        // 旧 settings.json 无 web/tools 字段：serde default 兜底。
        let settings = parse(r#"{"schemaVersion": 1, "theme": "dark"}"#).unwrap();
        assert_eq!(settings.web, WebSettings::default());
        assert_eq!(settings.tools, ToolsSettings::default());
        // 往返后再写盘不带空段（serde 只序列化有值字段？不——
        // rename_all + default 序列化仍会写出全部字段，但读取端兼容）。
        let text = to_text(&settings);
        assert!(text.contains("defaultEngine"));
    }

    #[test]
    fn web段校验_searxng缺endpoint拒绝() {
        let settings = parse(
            r#"{"web": {"defaultEngine": "s", "engines": [{"id": "s", "kind": "searxng"}]}}"#,
        );
        assert!(settings.is_err());
    }

    #[test]
    fn web段校验_kind白名单外拒绝() {
        let settings =
            parse(r#"{"web": {"defaultEngine": "x", "engines": [{"id": "x", "kind": "google"}]}}"#);
        assert!(settings.is_err());
    }

    #[test]
    fn web段迁移_defaultengine不在清单时跟走() {
        // 读入路径：default_engine 指向不存在的 id 由迁移兜底（跟走清单首位），
        // 不再是校验错误——校验错误留给 apply_patch 等不走迁移的入口。
        let settings = parse(
            r#"{"web": {"defaultEngine": "ghost", "engines": [{"id": "bing", "kind": "bing"}]}}"#,
        )
        .unwrap();
        assert_eq!(settings.web.default_engine, "bing");
    }

    #[test]
    fn web段校验_id重复拒绝() {
        let settings = parse(
            r#"{"web": {"engines": [
                {"id": "b", "kind": "bing"},
                {"id": "b", "kind": "duckduckgo"}
            ]}}"#,
        );
        assert!(settings.is_err());
    }

    #[test]
    fn web段校验_合法searxng通过() {
        let settings = parse(
            r#"{"web": {"defaultEngine": "s", "resultLimit": 20, "engines": [
                {"id": "s", "kind": "searxng", "endpoint": "https://searx.example.com"}
            ]}}"#,
        )
        .unwrap();
        assert_eq!(settings.web.result_limit, 20);
        assert_eq!(
            settings.web.engines[0].endpoint,
            "https://searx.example.com"
        );
    }

    #[test]
    fn web段校验_resultlimit越界拒绝() {
        let settings = parse(r#"{"web": {"resultLimit": 0}}"#);
        assert!(settings.is_err());
        let settings = parse(r#"{"web": {"resultLimit": 51}}"#);
        assert!(settings.is_err());
    }

    #[test]
    fn web段proxy_空off与合法代理通过() {
        // 空 = 直连（缺省已测）；"off" = 显式直连；各 scheme 形态都放行。
        for proxy in [
            "",
            "off",
            "OFF",
            "socks5://127.0.0.1:1080",
            "http://127.0.0.1:7890",
            "socks5h://proxy.lan:1080",
        ] {
            let settings = parse(&format!(r#"{{"web": {{"proxy": "{proxy}"}}}}"#)).unwrap();
            assert_eq!(settings.web.proxy, proxy);
        }
    }

    #[test]
    fn web段proxy_非法值拒绝() {
        for proxy in ["127.0.0.1:1080", "ftp://x", "socks5://", "just text"] {
            let settings = parse(&format!(r#"{{"web": {{"proxy": "{proxy}"}}}}"#));
            assert!(settings.is_err(), "应拒绝：{proxy}");
        }
    }

    /// parse 私有 helper 沿用上方测试的入口；这里补 to_text 引用避免死码。
    fn to_text(settings: &Settings) -> String {
        serde_json::to_string(settings).unwrap_or_default()
    }

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
    fn 旧默认网关端口迁移到按模式新默认() {
        // 持久化过旧默认 17400 的设置文件 → 迁移到当前构建模式的新默认。
        let settings = parse(r#"{"remote": {"port": 17400}}"#).unwrap();
        assert_eq!(settings.remote.port, crate::remote::default_gateway_port());
        // 用户显式配置过的其它端口原样保留，不被迁移波及。
        let custom = parse(r#"{"remote": {"port": 30000}}"#).unwrap();
        assert_eq!(custom.remote.port, 30000);
    }

    #[test]
    fn 另一构建模式的网关端口也迁移() {
        // release 备份还原到 debug：备份里 port=23090（release 默认），
        // 此刻该端口被 release 实例占用会 EADDRINUSE，必须迁到 23091。
        // debug 备份还原到 release：对称地 23091 → 23090。
        let other_mode_port = if cfg!(debug_assertions) {
            23090u16
        } else {
            23091
        };
        let text = format!(r#"{{"remote": {{"port": {other_mode_port}}}}}"#);
        let settings = parse(&text).unwrap();
        assert_eq!(settings.remote.port, crate::remote::default_gateway_port());
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

//! Linux 启动期的输入法（IM）与 locale 环境补齐。
//!
//! 背景（远端实测 192.168.20.3，omarchy/Arch + Hyprland + fcitx5）：
//! 千寻的进程环境是从 tty/SSH 侧继承的，**没有**图形会话的 IM 环境——
//! `XMODIFIERS`、`GTK_IM_MODULE` 全缺，`LANG`/`LC_*` 也全缺（locale=POSIX）。
//! 后果：GTK 的 IM 模块没被装载（CEF 日志
//! `object class 'GtkSettings' has no property named 'gtk-modules'`——GTK4
//! 已移除该属性）、XIM 也没接上（fcitx5 的 InputContext 里查不到千寻进程）
//! ⇒ **输入法完全不可用**。
//!
//! 这里在 `run()` 的最开头（`tauri::Builder` 之前，即 GTK/CEF 初始化之前）
//! 补齐这些变量：
//! - `XMODIFIERS` / `GTK_IM_MODULE`：分两层。
//!   - `QIANXUN_IM_MODULE` **显式指定** ⇒ **强制**写入（覆盖已有非空值，
//!     那是用户的明确意图）。取值：`auto`（默认）/ `fcitx` / `ibus` / `xim` /
//!     `none`。`xim` 走 X11 的 XIM（绕开 GTK 的 fcitx 模块，用来绕开「候选窗
//!     被 resize 成整窗」的怪现象）；`none` 表示明确**不要**输入法 ⇒ 移除
//!     这两个变量。非法值 ⇒ warn 并回退 `auto`。
//!   - 未指定（`auto`）⇒ **只补缺失或为空**的变量，已有非空值一律不动；
//!     探测到 fcitx 系（fcitx5/fcitx）→ `@im=fcitx` / `fcitx`；探测到
//!     ibus 系（ibus-daemon/ibus）→ `@im=ibus` / `ibus`。
//! - `LANG`：`LANG` 与 `LC_ALL` **都**缺失或为 `POSIX`/`C` 时才补，
//!   优先 `en_US.UTF-8`（系统确有该 locale 时），否则 `C.UTF-8`。
//!
//! **分层约定（重要）**：决策逻辑一律写成不依赖平台与全局状态的纯函数
//! （`detect_im_from` / `pick_im` / `decide_lang` / `locale_is_usable` /
//! `resolve_im_override` / `plan_im_env`），因此在 Windows 上也能跑单元测试；
//! 真正读写环境变量的代码全部关在 `#[cfg(target_os = "linux")]` 里。
//! Windows 侧 cargo 根本不编译平台那段，写错了只有远端 `cargo build` 能
//! 暴露——所以别把纯逻辑塞进 cfg 里。

use std::path::{Path, PathBuf};

/// 输入法种类。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImKind {
    /// fcitx / fcitx5（同一族，IM 变量取值相同）。
    Fcitx,
    /// ibus。
    Ibus,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl ImKind {
    /// `XMODIFIERS` 的取值：fcitx 家族（含 fcitx5）统一用 `fcitx`。
    pub fn xmodifiers(self) -> &'static str {
        match self {
            Self::Fcitx => "@im=fcitx",
            Self::Ibus => "@im=ibus",
        }
    }

    /// `GTK_IM_MODULE` 的取值。
    pub fn gtk_module(self) -> &'static str {
        match self {
            Self::Fcitx => "fcitx",
            Self::Ibus => "ibus",
        }
    }
}

/// 用户显式指定的输入法模块（`QIANXUN_IM_MODULE` 的取值语义）。
///
/// 与 [`ImKind`] 的区别：`ImKind` 是「**探测**到了什么」，`ImChoice` 是
/// 「**用户要求**用什么」——显式指定时**强制**写入（覆盖已有非空值），
/// 因为那是用户的明确意图；未指定时才走探测 + 只补缺失的保守路径。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImChoice {
    /// 自动探测（默认）：只在缺失或为空时补，已有非空值不覆盖。
    Auto,
    /// fcitx / fcitx5：`XMODIFIERS=@im=fcitx` + `GTK_IM_MODULE=fcitx`。
    Fcitx,
    /// ibus：`XMODIFIERS=@im=ibus` + `GTK_IM_MODULE=ibus`。
    Ibus,
    /// XIM（走 X11 的 XIM，绕开 GTK 的 fcitx 模块）：
    /// `XMODIFIERS=@im=xim` + `GTK_IM_MODULE=xim`。
    Xim,
    /// 明确**不要**输入法：移除 `XMODIFIERS` 与 `GTK_IM_MODULE`。
    None,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl ImChoice {
    /// 该选择对应的 IM 模块名（`Auto` 与禁用态没有模块名）。
    pub fn module(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Fcitx => Some("fcitx"),
            Self::Ibus => Some("ibus"),
            Self::Xim => Some("xim"),
            Self::None => None,
        }
    }

    /// 是否是「用户显式指定」（显式 ⇒ 强制覆盖已有非空值）。
    pub fn is_forced(self) -> bool {
        !matches!(self, Self::Auto)
    }
}

/// 用户指定 IM 模块的环境变量名。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const IM_MODULE_ENV: &str = "QIANXUN_IM_MODULE";

/// 解析 `QIANXUN_IM_MODULE` 的原始值 → 生效的选择（纯函数）。
///
/// - 未设置 / 空串 / 只有空白 → [`ImChoice::Auto`]（视为未指定）
/// - `auto` → [`ImChoice::Auto`]
/// - `fcitx` / `ibus` / `xim` / `none` → 对应选择
/// - 其它值 → 回退 [`ImChoice::Auto`]（非法值由调用方打 warn，
///   判定见 [`im_override_is_valid`]）
///
/// 大小写不敏感、前后空白忽略。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn resolve_im_override(raw: Option<&str>) -> ImChoice {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return ImChoice::Auto;
    };
    match value.to_ascii_lowercase().as_str() {
        "fcitx" => ImChoice::Fcitx,
        "ibus" => ImChoice::Ibus,
        "xim" => ImChoice::Xim,
        "none" => ImChoice::None,
        // "auto" 与非法值都回退 Auto（非法值的 warn 由调用方负责）。
        _ => ImChoice::Auto,
    }
}

/// `QIANXUN_IM_MODULE` 的原始值是否合法（纯函数）。
///
/// 未设置 / 空串视为「未指定」，合法；`auto`/`fcitx`/`ibus`/`xim`/`none`
/// 合法；其它非空值非法 ⇒ 调用方打 warn 并回退 auto。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn im_override_is_valid(raw: Option<&str>) -> bool {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    matches!(
        value.to_ascii_lowercase().as_str(),
        "auto" | "fcitx" | "ibus" | "xim" | "none"
    )
}

/// 对单个环境变量的处置动作。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImEnvAction {
    /// 不动（保持原值）。
    Keep,
    /// 写入该值。
    Set(String),
    /// 删除该变量。
    Remove,
}

/// IM 环境的处置计划（纯函数产物；执行层只照做，不做判断）。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImEnvPlan {
    /// `XMODIFIERS` 的处置。
    pub xmodifiers: ImEnvAction,
    /// `GTK_IM_MODULE` 的处置。
    pub gtk_im_module: ImEnvAction,
    /// 决策来源说明（供日志与测试断言）。
    pub source: String,
}

/// 算出 `XMODIFIERS` / `GTK_IM_MODULE` 该怎么处置（纯函数）。
///
/// 语义分层：
/// - **显式指定**（`choice` 非 `Auto`）⇒ **强制**：直接写目标值（`none`
///   则两个都删），覆盖已有的非空值——那是用户的明确意图。
/// - **`Auto`** ⇒ **保守**：只在缺失或为空时补；已有非空值一律不动。
/// - `detected` 是探测到的 IM 模块名（`fcitx` / `ibus`）；`None` 或空串
///   表示没探测到 ⇒ 什么都不动。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn plan_im_env(
    choice: ImChoice,
    detected: Option<&str>,
    current_xmodifiers: Option<&str>,
    current_gtk_im_module: Option<&str>,
) -> ImEnvPlan {
    match choice {
        // 自动：只补缺失/空值，已有非空值不动。
        ImChoice::Auto => {
            let detected = detected.map(str::trim).filter(|value| !value.is_empty());
            let Some(module) = detected else {
                return ImEnvPlan {
                    xmodifiers: ImEnvAction::Keep,
                    gtk_im_module: ImEnvAction::Keep,
                    source: "auto（未探测到输入法）".to_string(),
                };
            };
            let has_x = current_xmodifiers
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            let has_g = current_gtk_im_module
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            ImEnvPlan {
                xmodifiers: if has_x {
                    ImEnvAction::Keep
                } else {
                    ImEnvAction::Set(format!("@im={module}"))
                },
                gtk_im_module: if has_g {
                    ImEnvAction::Keep
                } else {
                    ImEnvAction::Set(module.to_string())
                },
                source: format!("auto（探测到 {module}）"),
            }
        }
        // 明确不要输入法：两个变量都删掉。
        ImChoice::None => ImEnvPlan {
            xmodifiers: ImEnvAction::Remove,
            gtk_im_module: ImEnvAction::Remove,
            source: format!("{IM_MODULE_ENV}=none"),
        },
        // 显式指定：强制覆盖。
        forced => {
            let module = forced.module().unwrap_or_default();
            ImEnvPlan {
                xmodifiers: ImEnvAction::Set(format!("@im={module}")),
                gtk_im_module: ImEnvAction::Set(module.to_string()),
                source: format!("{IM_MODULE_ENV}={module}"),
            }
        }
    }
}

/// PATH 里的 IM 可执行文件候选，**按优先级排列**：
/// fcitx5 → fcitx → ibus-daemon → ibus。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const IM_CANDIDATES: &[(&str, ImKind)] = &[
    ("fcitx5", ImKind::Fcitx),
    ("fcitx", ImKind::Fcitx),
    ("ibus-daemon", ImKind::Ibus),
    ("ibus", ImKind::Ibus),
];

/// 从一个可执行文件路径判定它属于哪种输入法（纯函数）。
///
/// 只看文件名（`fcitx5` / `fcitx` / `ibus-daemon` / `ibus`），不碰文件系统。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn detect_im_from(path: &Path) -> Option<ImKind> {
    let name = path.file_name()?.to_str()?;
    IM_CANDIDATES
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, kind)| *kind)
}

/// 从「已确认存在」的可执行文件路径里按优先级挑出输入法（纯函数）。
///
/// 优先级：fcitx5 → fcitx → ibus-daemon → ibus；同名取列表里第一个。
/// 返回命中的 IM 种类与它的路径（路径用于日志，说明「探测自哪里」）。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn pick_im(present: &[PathBuf]) -> Option<(ImKind, PathBuf)> {
    for (candidate, kind) in IM_CANDIDATES {
        let found = present
            .iter()
            .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(*candidate));
        if let Some(path) = found {
            return Some((*kind, path.clone()));
        }
    }
    None
}

/// locale 值是否「像样」：存在、非空、且不是 `POSIX`/`C`（纯函数）。
///
/// `C.UTF-8` 算像样（至少是 UTF-8），`POSIX` 与 `C` 不算（等于没设语言）。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn locale_is_usable(value: Option<&str>) -> bool {
    match value {
        None => false,
        Some(raw) => {
            let value = raw.trim();
            !value.is_empty() && !is_c_locale(value)
        }
    }
}

/// `POSIX` / `C`（大小写不敏感）——这类值等价于「没设语言」。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_c_locale(value: &str) -> bool {
    matches!(value.to_ascii_uppercase().as_str(), "POSIX" | "C")
}

/// 决定要写入的 `LANG` 值（纯函数）。
///
/// `LANG` 与 `LC_ALL` **都**缺失或为 `POSIX`/`C` 时才需要补：
/// - 系统确有 `en_US.UTF-8` → `en_US.UTF-8`
/// - 否则 → `C.UTF-8`（至少是 UTF-8，不至于让 GTK 退回 ASCII）
///
/// 已有可用值时返回 `None`——**不覆盖**用户的显式设置。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn decide_lang(
    current_lang: Option<&str>,
    current_lc_all: Option<&str>,
    has_en_us_utf8: bool,
) -> Option<&'static str> {
    if locale_is_usable(current_lang) || locale_is_usable(current_lc_all) {
        return None;
    }
    Some(if has_en_us_utf8 {
        "en_US.UTF-8"
    } else {
        "C.UTF-8"
    })
}

/// `LC_ALL` 当前是否会遮蔽 `LANG`（纯函数）。
///
/// glibc 里 `LC_ALL` 优先级最高：它还是 `POSIX`/`C` 时，光设 `LANG` 不生效，
/// 必须把 `LC_ALL` 清掉（清掉后各 locale 类别回落到 `LANG`）。
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn lc_all_shadows_lang(current_lc_all: Option<&str>) -> bool {
    match current_lc_all {
        Some(raw) => {
            let value = raw.trim();
            !value.is_empty() && is_c_locale(value)
        }
        None => false,
    }
}

/// 启动前补齐输入法与 locale 环境。
///
/// 非 Linux 平台直接 no-op（Windows 的输入法由系统 WebView2/TSF 接管，
/// 没有等价变量可设）。Linux 上必须**在 GTK/CEF 初始化之前**调用——
/// GTK 模块与 XIM 连接都是在 GTK 初始化时读这些变量的，晚了就没用。
pub fn ensure_input_method_env() {
    #[cfg(target_os = "linux")]
    platform::ensure();
}

/// Linux 平台实现：读环境、探测、写环境。**只有这一段是平台专属的。**
#[cfg(target_os = "linux")]
mod platform {
    use super::{
        decide_lang, lc_all_shadows_lang, pick_im, ImChoice, ImEnvAction, ImKind, IM_CANDIDATES,
    };
    use std::path::{Path, PathBuf};

    /// 补齐全部（IM 变量 + locale）。
    pub(super) fn ensure() {
        ensure_im_env();
        ensure_locale();
    }

    /// PATH 里的文件能不能直接跑（存在 + 是普通文件 + 有 x 位）。
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }

    /// `PATH` 拆成目录列表（Linux 用 `:` 分隔）。
    fn path_dirs() -> Vec<PathBuf> {
        match std::env::var("PATH") {
            Ok(raw) => raw
                .split(':')
                .filter(|part| !part.is_empty())
                .map(PathBuf::from)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 在 PATH 里找出优先级最高的 IM 可执行文件。
    ///
    /// 判据是「有可执行文件」而非「守护进程在跑」——守护进程状态要读
    /// /proc 或连 DBus，成本与出错面都大得多；fcitx5 与 ibus 通常同机只装
    /// 一个，实际冲突概率低。
    fn find_im() -> Option<(ImKind, PathBuf)> {
        let dirs = path_dirs();
        if dirs.is_empty() {
            return None;
        }
        let mut present: Vec<PathBuf> = Vec::new();
        for dir in &dirs {
            for (name, _) in IM_CANDIDATES {
                let candidate = dir.join(*name);
                if is_executable(&candidate) && super::detect_im_from(&candidate).is_some() {
                    present.push(candidate);
                }
            }
        }
        pick_im(&present)
    }

    /// `XMODIFIERS` / `GTK_IM_MODULE` 的处置。
    ///
    /// 语义分层（决策全在纯函数里，这里只照做）：
    /// - `QIANXUN_IM_MODULE` 显式指定 ⇒ **强制**写入（覆盖已有非空值）。
    /// - `auto`（未指定/非法）⇒ **只在缺失或为空时补**，已有非空值不动。
    fn ensure_im_env() {
        let raw = std::env::var(super::IM_MODULE_ENV).ok();
        let choice = super::resolve_im_override(raw.as_deref());
        if !super::im_override_is_valid(raw.as_deref()) {
            crate::logging::log(
                "warn",
                &format!(
                    "{} 取值非法（{}），回退 auto（自动探测 + 只补缺失值）",
                    super::IM_MODULE_ENV,
                    raw.as_deref().unwrap_or("<未设置>")
                ),
            );
        }
        // 探测只在 auto 时需要（显式指定时目标值已定，探测纯属浪费）。
        let detected = match choice {
            ImChoice::Auto => find_im().map(|(kind, path)| (kind.gtk_module().to_string(), path)),
            _ => None,
        };
        let current_xmodifiers = std::env::var("XMODIFIERS").ok();
        let current_gtk_im_module = std::env::var("GTK_IM_MODULE").ok();
        let plan = super::plan_im_env(
            choice,
            detected.as_ref().map(|(module, _)| module.as_str()),
            current_xmodifiers.as_deref(),
            current_gtk_im_module.as_deref(),
        );

        // 日志口径：显式指定叫「强制」，自动探测叫「补齐」，禁用叫「已移除」。
        let forced = choice.is_forced();
        let probe_note = detected
            .as_ref()
            .map(|(_, path)| format!("探测自 {}", path.display()))
            .unwrap_or_else(|| "未探测到输入法".to_string());
        let note = if forced {
            format!("来源 {}", plan.source)
        } else {
            probe_note.clone()
        };

        match &plan.xmodifiers {
            ImEnvAction::Set(value) => {
                std::env::set_var("XMODIFIERS", value);
                crate::logging::log(
                    "info",
                    &format!(
                        "输入法环境：{} XMODIFIERS={}（{}）",
                        if forced { "强制" } else { "补齐" },
                        value,
                        note
                    ),
                );
            }
            ImEnvAction::Remove => {
                std::env::remove_var("XMODIFIERS");
            }
            ImEnvAction::Keep => {
                let current = current_xmodifiers.as_deref().unwrap_or("");
                if current.trim().is_empty() {
                    crate::logging::log(
                        "warn",
                        "输入法环境：未探测到可用输入法（PATH 里没有 \
                         fcitx5/fcitx/ibus-daemon/ibus），XMODIFIERS 保持原样，\
                         输入法可能不可用",
                    );
                } else {
                    crate::logging::log(
                        "info",
                        &format!("输入法环境：XMODIFIERS 已有值 {current}，保持不动"),
                    );
                }
            }
        }

        match &plan.gtk_im_module {
            ImEnvAction::Set(value) => {
                std::env::set_var("GTK_IM_MODULE", value);
                crate::logging::log(
                    "info",
                    &format!(
                        "输入法模块：{} GTK_IM_MODULE={}（{}）",
                        if forced { "强制" } else { "补齐" },
                        value,
                        note
                    ),
                );
            }
            ImEnvAction::Remove => {
                std::env::remove_var("GTK_IM_MODULE");
            }
            ImEnvAction::Keep => {
                let current = current_gtk_im_module.as_deref().unwrap_or("");
                if current.trim().is_empty() {
                    crate::logging::log(
                        "warn",
                        "输入法环境：未探测到可用输入法（PATH 里没有 \
                         fcitx5/fcitx/ibus-daemon/ibus），GTK_IM_MODULE 保持原样，\
                         输入法可能不可用",
                    );
                } else {
                    crate::logging::log(
                        "info",
                        &format!("输入法环境：GTK_IM_MODULE 已有值 {current}，保持不动"),
                    );
                }
            }
        }

        if matches!(plan.xmodifiers, ImEnvAction::Remove)
            && matches!(plan.gtk_im_module, ImEnvAction::Remove)
        {
            crate::logging::log(
                "info",
                &format!(
                    "输入法环境：已移除 XMODIFIERS/GTK_IM_MODULE（{}）",
                    plan.source
                ),
            );
        }
    }

    /// 系统里是否确有 `en_US.UTF-8` 这个 locale。
    ///
    /// 先问 `locale -a`（权威），它不可用时退回目录判断
    /// （`/usr/lib/locale/en_US.utf8` 是 glibc 的 locale 归档目录）。
    fn has_en_us_utf8() -> bool {
        if let Ok(output) = std::process::Command::new("locale").arg("-a").output() {
            if output.status.success() {
                let listing = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
                if listing
                    .split_whitespace()
                    .any(|entry| entry == "en_us.utf8" || entry == "en_us.utf-8")
                {
                    return true;
                }
            }
        }
        [
            "/usr/lib/locale/en_US.utf8",
            "/usr/lib/locale/en_US.UTF-8",
            "/usr/share/locale/en_US.utf8",
        ]
        .iter()
        .any(|dir| Path::new(dir).is_dir())
    }

    /// `LANG` / `LC_ALL`：两者都缺失或为 `POSIX`/`C` 时补 `LANG`。
    fn ensure_locale() {
        let lang = std::env::var("LANG").ok();
        let lc_all = std::env::var("LC_ALL").ok();
        let Some(target) = decide_lang(lang.as_deref(), lc_all.as_deref(), has_en_us_utf8()) else {
            return;
        };
        std::env::set_var("LANG", target);
        crate::logging::log(
            "info",
            &format!(
                "补齐 locale：LANG={target}（原值 {}）",
                lang.as_deref().unwrap_or("<未设置>")
            ),
        );
        // LC_ALL 优先级最高：它当前是 POSIX/C 时会盖住刚补的 LANG，必须清掉。
        if lc_all_shadows_lang(lc_all.as_deref()) {
            std::env::remove_var("LC_ALL");
            crate::logging::log("info", "清除会遮蔽 LANG 的 LC_ALL（值为 POSIX/C）");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decide_lang, detect_im_from, im_override_is_valid, lc_all_shadows_lang, locale_is_usable,
        pick_im, plan_im_env, resolve_im_override, ImChoice, ImEnvAction, ImKind, IM_CANDIDATES,
    };
    use std::path::PathBuf;

    #[test]
    fn 识别各输入法可执行文件() {
        assert_eq!(
            detect_im_from(std::path::Path::new("/usr/bin/fcitx5")),
            Some(ImKind::Fcitx)
        );
        assert_eq!(
            detect_im_from(std::path::Path::new("/usr/bin/fcitx")),
            Some(ImKind::Fcitx)
        );
        assert_eq!(
            detect_im_from(std::path::Path::new("/usr/bin/ibus-daemon")),
            Some(ImKind::Ibus)
        );
        assert_eq!(
            detect_im_from(std::path::Path::new("/usr/bin/ibus")),
            Some(ImKind::Ibus)
        );
        // 不认识的程序名不算输入法。
        assert_eq!(detect_im_from(std::path::Path::new("/usr/bin/ls")), None);
        assert_eq!(detect_im_from(std::path::Path::new("/usr/bin")), None);
    }

    #[test]
    fn 候选优先级是fcitx5优先于fcitx优先于ibus() {
        let names: Vec<&str> = IM_CANDIDATES.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec!["fcitx5", "fcitx", "ibus-daemon", "ibus"],
            "候选顺序即优先级"
        );
    }

    #[test]
    fn 多个输入法共存时按优先级取() {
        // fcitx5 与 fcitx 都在 → 取 fcitx5。
        let present = vec![
            PathBuf::from("/usr/bin/fcitx"),
            PathBuf::from("/usr/bin/fcitx5"),
        ];
        let (kind, path) = pick_im(&present).expect("应命中 fcitx5");
        assert_eq!(kind, ImKind::Fcitx);
        assert_eq!(path, PathBuf::from("/usr/bin/fcitx5"));

        // fcitx 与 ibus 都在 → fcitx 优先。
        let present = vec![
            PathBuf::from("/usr/bin/ibus-daemon"),
            PathBuf::from("/usr/bin/fcitx"),
        ];
        assert_eq!(pick_im(&present).map(|(kind, _)| kind), Some(ImKind::Fcitx));

        // 只有 ibus 系 → ibus。
        let present = vec![PathBuf::from("/usr/bin/ibus")];
        assert_eq!(pick_im(&present).map(|(kind, _)| kind), Some(ImKind::Ibus));

        // 都没有 → None（调用方据此告警，不乱设变量）。
        assert!(pick_im(&[PathBuf::from("/usr/bin/ls")]).is_none());
        assert!(pick_im(&[]).is_none());
    }

    #[test]
    fn locale可用性判定() {
        assert!(locale_is_usable(Some("zh_CN.UTF-8")));
        assert!(locale_is_usable(Some("en_US.UTF-8")));
        assert!(locale_is_usable(Some("C.UTF-8")));
        assert!(!locale_is_usable(Some("POSIX")));
        assert!(!locale_is_usable(Some("posix")));
        assert!(!locale_is_usable(Some("C")));
        assert!(!locale_is_usable(Some("")));
        assert!(!locale_is_usable(Some("   ")));
        assert!(!locale_is_usable(None));
    }

    #[test]
    fn locale缺失或posix时才补() {
        // 全缺 + 系统有 en_US.UTF-8 → en_US.UTF-8
        assert_eq!(decide_lang(None, None, true), Some("en_US.UTF-8"));
        // 全缺 + 系统没有 en_US.UTF-8 → C.UTF-8
        assert_eq!(decide_lang(None, None, false), Some("C.UTF-8"));
        // 两边都是 POSIX/C → 照样补
        assert_eq!(
            decide_lang(Some("POSIX"), Some("C"), true),
            Some("en_US.UTF-8")
        );
        assert_eq!(
            decide_lang(Some("C"), Some("POSIX"), false),
            Some("C.UTF-8")
        );
        // 已有可用值 → 不覆盖
        assert_eq!(decide_lang(Some("zh_CN.UTF-8"), None, true), None);
        assert_eq!(decide_lang(None, Some("en_US.UTF-8"), true), None);
        assert_eq!(decide_lang(Some("C.UTF-8"), None, true), None);
    }

    #[test]
    fn lc_all为posix时遮蔽lang() {
        assert!(lc_all_shadows_lang(Some("POSIX")));
        assert!(lc_all_shadows_lang(Some("C")));
        assert!(!lc_all_shadows_lang(Some("en_US.UTF-8")));
        assert!(!lc_all_shadows_lang(Some("")));
        assert!(!lc_all_shadows_lang(None));
    }

    #[test]
    fn 解析输入法模块覆盖值() {
        // 未设置 / 空串 / 纯空白 ⇒ 视为未指定 → auto。
        assert_eq!(resolve_im_override(None), ImChoice::Auto);
        assert_eq!(resolve_im_override(Some("")), ImChoice::Auto);
        assert_eq!(resolve_im_override(Some("   ")), ImChoice::Auto);
        // auto 显式写出来也是 auto。
        assert_eq!(resolve_im_override(Some("auto")), ImChoice::Auto);
        assert_eq!(resolve_im_override(Some("AUTO")), ImChoice::Auto);
        // 四个合法取值。
        assert_eq!(resolve_im_override(Some("fcitx")), ImChoice::Fcitx);
        assert_eq!(resolve_im_override(Some("ibus")), ImChoice::Ibus);
        assert_eq!(resolve_im_override(Some("xim")), ImChoice::Xim);
        assert_eq!(resolve_im_override(Some("none")), ImChoice::None);
        // 大小写不敏感 + 去空白。
        assert_eq!(resolve_im_override(Some("FCITX")), ImChoice::Fcitx);
        assert_eq!(resolve_im_override(Some("  Xim  ")), ImChoice::Xim);
        assert_eq!(resolve_im_override(Some("None")), ImChoice::None);
        // 非法值回退 auto（同时由 im_override_is_valid 报非法）。
        assert_eq!(resolve_im_override(Some("foo")), ImChoice::Auto);
        assert_eq!(resolve_im_override(Some("@im=fcitx")), ImChoice::Auto);
    }

    #[test]
    fn 输入法模块覆盖值合法性() {
        assert!(im_override_is_valid(None));
        assert!(im_override_is_valid(Some("")));
        assert!(im_override_is_valid(Some("auto")));
        assert!(im_override_is_valid(Some("FCITX")));
        assert!(im_override_is_valid(Some("ibus")));
        assert!(im_override_is_valid(Some("xim")));
        assert!(im_override_is_valid(Some("none")));
        assert!(!im_override_is_valid(Some("foo")));
        assert!(!im_override_is_valid(Some("@im=fcitx")));
    }

    #[test]
    fn auto模式下只补缺失不覆盖已有() {
        // 已有非空值 → 两个变量都不动。
        let plan = plan_im_env(
            ImChoice::Auto,
            Some("fcitx"),
            Some("@im=ibus"),
            Some("ibus"),
        );
        assert_eq!(plan.xmodifiers, ImEnvAction::Keep);
        assert_eq!(plan.gtk_im_module, ImEnvAction::Keep);

        // 缺失 → 按探测结果补齐。
        let plan = plan_im_env(ImChoice::Auto, Some("fcitx"), None, None);
        assert_eq!(plan.xmodifiers, ImEnvAction::Set("@im=fcitx".to_string()));
        assert_eq!(plan.gtk_im_module, ImEnvAction::Set("fcitx".to_string()));

        // 空串等同缺失 → 补齐。
        let plan = plan_im_env(ImChoice::Auto, Some("ibus"), Some(""), Some(""));
        assert_eq!(plan.xmodifiers, ImEnvAction::Set("@im=ibus".to_string()));
        assert_eq!(plan.gtk_im_module, ImEnvAction::Set("ibus".to_string()));

        // 只缺一个 → 只补那个。
        let plan = plan_im_env(ImChoice::Auto, Some("fcitx"), Some("@im=fcitx"), None);
        assert_eq!(plan.xmodifiers, ImEnvAction::Keep);
        assert_eq!(plan.gtk_im_module, ImEnvAction::Set("fcitx".to_string()));

        // 没探测到 → 什么都不动（不乱设变量）。
        let plan = plan_im_env(ImChoice::Auto, None, None, None);
        assert_eq!(plan.xmodifiers, ImEnvAction::Keep);
        assert_eq!(plan.gtk_im_module, ImEnvAction::Keep);
        let plan = plan_im_env(ImChoice::Auto, Some(""), None, None);
        assert_eq!(plan.xmodifiers, ImEnvAction::Keep);
        assert_eq!(plan.gtk_im_module, ImEnvAction::Keep);
    }

    #[test]
    fn 显式指定时强制覆盖() {
        // 已有 ibus，强制改成 fcitx。
        let plan = plan_im_env(ImChoice::Fcitx, None, Some("@im=ibus"), Some("ibus"));
        assert_eq!(plan.xmodifiers, ImEnvAction::Set("@im=fcitx".to_string()));
        assert_eq!(plan.gtk_im_module, ImEnvAction::Set("fcitx".to_string()));

        // 强制 ibus。
        let plan = plan_im_env(ImChoice::Ibus, None, Some("@im=fcitx"), Some("fcitx"));
        assert_eq!(plan.xmodifiers, ImEnvAction::Set("@im=ibus".to_string()));
        assert_eq!(plan.gtk_im_module, ImEnvAction::Set("ibus".to_string()));

        // 强制 xim（绕开 GTK 的 fcitx 模块）。
        let plan = plan_im_env(ImChoice::Xim, None, None, None);
        assert_eq!(plan.xmodifiers, ImEnvAction::Set("@im=xim".to_string()));
        assert_eq!(plan.gtk_im_module, ImEnvAction::Set("xim".to_string()));
    }

    #[test]
    fn 明确禁用输入法时移除两个变量() {
        let plan = plan_im_env(ImChoice::None, None, Some("@im=fcitx"), Some("fcitx"));
        assert_eq!(plan.xmodifiers, ImEnvAction::Remove);
        assert_eq!(plan.gtk_im_module, ImEnvAction::Remove);
        assert!(plan.source.contains("none"));
    }
}

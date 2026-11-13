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
//! - `XMODIFIERS` / `GTK_IM_MODULE`：缺（或空）才补。探测到 fcitx 系
//!   （fcitx5/fcitx）→ `@im=fcitx` / `fcitx`；探测到 ibus 系
//!   （ibus-daemon/ibus）→ `@im=ibus` / `ibus`。
//! - `LANG`：`LANG` 与 `LC_ALL` **都**缺失或为 `POSIX`/`C` 时才补，
//!   优先 `en_US.UTF-8`（系统确有该 locale 时），否则 `C.UTF-8`。
//!
//! **分层约定（重要）**：决策逻辑一律写成不依赖平台与全局状态的纯函数
//! （`detect_im_from` / `pick_im` / `decide_lang` / `locale_is_usable`），
//! 因此在 Windows 上也能跑单元测试；真正读写环境变量的代码全部关在
//! `#[cfg(target_os = "linux")]` 里。Windows 侧 cargo 根本不编译平台那段，
//! 写错了只有远端 `cargo build` 能暴露——所以别把纯逻辑塞进 cfg 里。

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
    use super::{decide_lang, lc_all_shadows_lang, pick_im, ImKind, IM_CANDIDATES};
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

    /// `XMODIFIERS` / `GTK_IM_MODULE`：缺（或空）才补，已有值一律不动。
    fn ensure_im_env() {
        let has_xmodifiers = std::env::var("XMODIFIERS")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let has_gtk_module = std::env::var("GTK_IM_MODULE")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if has_xmodifiers && has_gtk_module {
            return;
        }
        let Some((kind, path)) = find_im() else {
            crate::logging::log(
                "warn",
                "未探测到可用输入法（PATH 里没有 fcitx5/fcitx/ibus-daemon/ibus）；\
                 XMODIFIERS/GTK_IM_MODULE 保持原样，输入法可能不可用",
            );
            return;
        };
        if !has_xmodifiers {
            std::env::set_var("XMODIFIERS", kind.xmodifiers());
            crate::logging::log(
                "info",
                &format!(
                    "补齐输入法环境：XMODIFIERS={}（探测自 {}）",
                    kind.xmodifiers(),
                    path.display()
                ),
            );
        }
        if !has_gtk_module {
            std::env::set_var("GTK_IM_MODULE", kind.gtk_module());
            crate::logging::log(
                "info",
                &format!(
                    "补齐输入法环境：GTK_IM_MODULE={}（探测自 {}）",
                    kind.gtk_module(),
                    path.display()
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
        decide_lang, detect_im_from, lc_all_shadows_lang, locale_is_usable, pick_im, ImKind,
        IM_CANDIDATES,
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
}

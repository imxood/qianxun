//! Linux 路线的 runtime 装配：CEF（内嵌 Chromium）。
//!
//! Windows 走 `tauri-runtime-wry`（系统 WebView2），Linux 走
//! `tauri-runtime-cef`（CEF 154.x 自带 Chromium 内核）。两者的依赖在
//! Cargo.toml 里按 target 拆开，互不同时链接——tauri 只绑定一套 GTK，
//! 而 wry 路线要拖 WebKitGTK，CEF 路线走 GTK4。
//!
//! 远程调试（CDP）：CEF 不读 wry 那套 `additionalBrowserArgs`，端口要在
//! runtime 初始化参数里给。默认只在 debug 构建开，且端口可用
//! `QIANXUN_CDP_PORT` 覆盖；release 构建默认不开（远程调试端口对全机
//! 可达且无认证，属于安全暴露面）。

use tauri_runtime_cef::{Cef, RemoteDebugging};

/// 远程调试端口环境变量。
pub const CDP_PORT_ENV: &str = "QIANXUN_CDP_PORT";

/// 允许接入调试端口的额外来源环境变量（逗号分隔）。
pub const CDP_ORIGINS_ENV: &str = "QIANXUN_CDP_ORIGINS";

/// debug 构建的默认 CDP 端口（与 Windows 侧 tauri.debug.conf.json 的
/// `--remote-debugging-port=10222` 保持同一套口径，便于两平台用同一
/// 套探针脚本）。
pub const DEFAULT_CDP_PORT: u16 = 10222;

/// 额外放行的 WebSocket 来源。Chromium 默认接受无 Origin 头的客户端
/// （Playwright 的 connectOverCDP 就是这种），这里只在显式配置时追加，
/// 避免用 `--remote-allow-origins` 覆盖掉默认放行规则。
fn allowed_origins() -> Vec<String> {
    std::env::var(CDP_ORIGINS_ENV)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(str::to_string)
        .collect()
}

/// 追加 Chromium 命令行开关的环境变量（逗号或空格分隔）。
///
/// 用途：CEF/Chromium 在 Linux 上的行为（输入法、弹窗、缩放…）有大量
/// 开关可调，但 tauri-runtime-cef 尚未提供逐项配置。留这个口子后，遇到
/// 「改开关才能修」的问题时，用户可以只改环境变量重启验证，不必重新
/// 编译整个应用。
///
/// 例：`QIANXUN_CEF_ARGS=--disable-gtk-ime,--force-device-scale-factor=1`
///
/// 只接受以 `--` 开头的项，避免误把普通参数塞进 Chromium 命令行。
pub const EXTRA_ARGS_ENV: &str = "QIANXUN_CEF_ARGS";

/// 解析追加开关：逗号或空白分隔，忽略非 `--` 开头的项。
fn extra_args() -> Vec<String> {
    parse_extra_args(&std::env::var(EXTRA_ARGS_ENV).unwrap_or_default())
}

/// 纯函数：把环境变量里的原始文本解析成 Chromium 开关列表。
fn parse_extra_args(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|arg| arg.starts_with("--"))
        .map(str::to_string)
        .collect()
}

/// 解析远程调试端口：环境变量优先，其次 debug 构建用默认端口。
/// 端口必须在 1024-65535 之间（CEF 对范围外的值会忽略并告警）。
fn cdp_port() -> Option<u16> {
    let raw = match std::env::var(CDP_PORT_ENV) {
        Ok(value) => Some(value),
        Err(_) => {
            if cfg!(debug_assertions) {
                Some(DEFAULT_CDP_PORT.to_string())
            } else {
                None
            }
        }
    };
    let raw = raw?;
    let port = raw.trim().parse::<u16>().ok()?;
    if (1024..=65535).contains(&port) {
        Some(port)
    } else {
        None
    }
}

/// 装配 CEF runtime。
pub fn linux_cef_runtime() -> Cef {
    let mut cef = Cef::default();
    if let Some(port) = cdp_port() {
        cef = cef.remote_debugging(RemoteDebugging::Port {
            port,
            allowed_origins: allowed_origins(),
        });
    }
    for arg in extra_args() {
        cef = cef.command_line_arg(arg, None::<String>);
    }
    cef
}

#[cfg(test)]
mod tests {
    use super::parse_extra_args;

    #[test]
    fn 追加开关只认横线开头的项() {
        assert_eq!(
            parse_extra_args("--disable-gtk-ime, --force-device-scale-factor=1"),
            vec![
                "--disable-gtk-ime".to_string(),
                "--force-device-scale-factor=1".to_string()
            ]
        );
    }

    #[test]
    fn 追加开关忽略非开关文本与空串() {
        assert!(parse_extra_args("").is_empty());
        assert!(parse_extra_args("   ").is_empty());
        assert!(parse_extra_args("not-a-switch").is_empty());
        assert!(parse_extra_args("--keep,drop-me").eq(&vec!["--keep".to_string()]));
    }

    #[test]
    fn 追加开关支持空格分隔() {
        assert_eq!(
            parse_extra_args("--a --b\t--c"),
            vec!["--a".to_string(), "--b".to_string(), "--c".to_string()]
        );
    }
}

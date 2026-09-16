//! 千寻的进程入口。发布构建隐藏控制台窗口；装配逻辑全部在 lib.rs，
//! 这里保持只有一行可执行代码，方便移动端入口宏复用同一入口。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // debug 构建的 WebView2 隔离配置，必须早于 WebView2 SDK 初始化（loader
    // 在本进程内读 GetEnvironmentVariable，set_var 时机足够早）。release 构
    // 建完全跳过此段。
    #[cfg(all(windows, debug_assertions))]
    {
        // 独立 user-data-dir：WebView2 同一 user-data-dir 全机只允许一个
        // browser 主进程；release 千寻常驻时 debug 实例会挂到它的进程上，
        // 窗口加载 devUrl 等行为全部异常（2026-09 排障实录）。指到 debug
        // 专属目录后两实例彻底隔离，pnpm dev 可与 release 并存。
        //
        // 注意：CDP 调试端口不在这里设——`WEBVIEW2_ADDITIONAL_BROWSER_
        // ARGUMENTS` 会被 wry 显式传入的 options.AdditionalBrowserArguments
        // 覆盖（见 wry webview2/mod.rs 的 set_additional_browser_arguments），
        // env 形式永远无效。debug 的 CDP 端口走 `tauri.debug.conf.json` 的
        // `additionalBrowserArgs`（package.json 的 dev script 已挂 --config）。
        if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none() {
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                let mut dir = std::path::PathBuf::from(local);
                dir.push("com.qianxun.desktop");
                dir.push("EBWebView-dev");
                std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", dir);
            }
        }
    }
    qianxun_lib::run()
}

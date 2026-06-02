// 千寻 Tauri 桌面端 — Stage 2 入口
// 把所有真正的逻辑放在 lib.rs (qianxun_desktop_lib::run),
// 这样 mobile (iOS/Android, P1) 可以复用 lib entry.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    qianxun_desktop_lib::run();
}

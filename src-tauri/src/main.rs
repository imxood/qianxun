//! 千寻的进程入口。发布构建隐藏控制台窗口；装配逻辑全部在 lib.rs，
//! 这里保持只有一行可执行代码，方便移动端入口宏复用同一入口。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    qianxun_lib::run()
}

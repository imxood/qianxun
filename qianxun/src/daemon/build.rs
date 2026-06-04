//! 千寻 daemon Web UI 自动 build 钩子 (MVP-2 落地后, 2026-06-04 阶段 1).
//!
//! 触发: `cargo build -p qianxun` (or `cargo run --bin qx -- daemon ...`).
//! 行为: 检测 `qianxun/src/daemon/ui/package.json` 存在时, 自动跑
//!   1. `pnpm --dir qianxun/src/daemon/ui install --frozen-lockfile` (拉依赖)
//!   2. `pnpm --dir qianxun/src/daemon/ui build` (vite build → `build/`)
//!
//! 跳过: `CARGO_CFG_TEST=1` (cargo test 时不跑, 避免拖慢测试)
//! 跳过: `ui/build/index.html` 已存在 (incremental, 不重复跑)
//!
//! 错误: pnpm 找不到 / build 失败 → panic with 友好提示, 提示需 pnpm + node ≥ 18.
//!
//! 路径来源: `CARGO_MANIFEST_DIR` = `qianxun/` (qianxun crate root).
//!          ui 在 `qianxun/src/daemon/ui/`.

use std::path::Path;
use std::process::Command;

fn main() {
    // 1. cargo test 不需要 build UI (否则拖慢)
    if std::env::var("CARGO_CFG_TEST").is_ok() {
        return;
    }

    // 2. 定位 ui/ (CARGO_MANIFEST_DIR = qianxun/, ui 在 src/daemon/ui/)
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("src").join("daemon").join("ui");
    if !ui_dir.join("package.json").exists() {
        println!(
            "cargo:warning=qianxun-ui/ package.json not found at {}, skip pnpm build",
            ui_dir.display()
        );
        return;
    }

    // 3. 增量构建监听: 改了 ui 任一文件就重 build
    println!("cargo:rerun-if-changed={}/package.json", ui_dir.display());
    println!("cargo:rerun-if-changed={}/pnpm-lock.yaml", ui_dir.display());
    println!("cargo:rerun-if-changed={}/svelte.config.js", ui_dir.display());
    println!("cargo:rerun-if-changed={}/vite.config.ts", ui_dir.display());
    println!("cargo:rerun-if-changed={}/src", ui_dir.display());
    println!("cargo:rerun-if-changed={}/static", ui_dir.display());
    // 监听 build/ 目录: 删了 build/ 后 cargo 才会重跑本 build.rs,
    // 否则 cargo fingerprint 缓存认为 build.rs 已执行, 跳过 (印的是旧 warning).
    println!("cargo:rerun-if-changed={}/build", ui_dir.display());

    // 4. 增量构建: build/index.html 已有就跳过
    let build_index = ui_dir.join("build").join("index.html");
    if build_index.exists() {
        println!(
            "cargo:warning=ui/build/index.html exists, skip pnpm build (delete build/ to force rebuild)"
        );
        return;
    }

    // 5. 跑 pnpm install + build
    //
    // Windows 注意: 全局 pnpm shim 经常是 `pnpm` (无后缀, POSIX shell 脚本
    // `#!/bin/sh`) + `pnpm.cmd` (Windows batch) + `pnpm.ps1` (PowerShell).
    // Rust `Command::new("pnpm")` 调 CreateProcessW, 找到 `pnpm` 无后缀文件
    // 就直接 exec, 但 Windows 不认 POSIX shebang → spawn 失败.
    // 修法: Windows 强制用 `pnpm.cmd`, Linux/macOS 用 `pnpm`.
    #[cfg(windows)]
    const PNPM_CMD: &str = "pnpm.cmd";
    #[cfg(not(windows))]
    const PNPM_CMD: &str = "pnpm";

    // 用绝对路径, 避免 current_dir 是 qianxun/ 时再传相对路径 `--dir qianxun/src/...`
    // 翻倍成 `qianxun/qianxun/src/...`.
    let ui_dir_str = ui_dir.to_str().expect("ui_dir is not valid UTF-8");

    let run_pnpm = |args: &[&str]| -> bool {
        println!("[qianxun build.rs] running: {} {}", PNPM_CMD, args.join(" "));
        let status = Command::new(PNPM_CMD).args(args).status();
        match status {
            Ok(s) => s.success(),
            Err(e) => {
                println!("cargo:warning={} spawn failed: {e}", PNPM_CMD);
                println!(
                    "cargo:warning=确认 PATH 含 pnpm 安装目录 (Windows: C:\\Users\\<你>\\AppData\\Roaming\\npm), \
                     且该目录下同时有 pnpm (无后缀) 和 pnpm.cmd (Windows batch) 两个 shim. \
                     没有 pnpm 就用: npm install -g pnpm"
                );
                false
            }
        }
    };

    if !run_pnpm(&["--dir", ui_dir_str, "install", "--frozen-lockfile"]) {
        panic!(
            "pnpm install failed. 需要: pnpm + node ≥ 18. 手动跑: cd {} && pnpm install",
            ui_dir_str
        );
    }
    if !run_pnpm(&["--dir", ui_dir_str, "build"]) {
        panic!(
            "pnpm build failed. 手动跑: cd {} && pnpm build 查错",
            ui_dir_str
        );
    }

    // 6. 验证 build 产物
    if !build_index.exists() {
        panic!(
            "pnpm build 完成但 {}/index.html 不存在, 检查 svelte.config.js 输出目录",
            build_index.display()
        );
    }
    println!(
        "[qianxun build.rs] ✅ UI built to {}/ (index.html ready)",
        ui_dir.join("build").display()
    );
}

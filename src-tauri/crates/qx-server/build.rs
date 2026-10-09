//! 把控制台前端产物编译进二进制（docs/20 §7.4 决策 3：单二进制铁律）。
//!
//! 刻意不引 rust-embed：它只省掉本文件这几十行，却多一条必须联网拉的依赖，
//! 而「构建期不需要联网」对自托管分发是硬收益。代价是自己管三件小事——
//! 目录缺失、Content-Type、gzip 变体，都在本文件里显式处理。
//!
//! 产物形态：`$OUT_DIR/console_assets.rs`，由 assets.rs `include!`。
//! dist 不存在（没跑 `pnpm build:console`）时生成空表并留 cargo 警告，
//! 而不是编译失败：cargo test / clippy 不该被前端构建绑住。

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // crates/qx-server → crates → src-tauri → 仓库根（两处 ../ 是 src-tauri，还差一跳）。
    let console_pkg = manifest.join("../../../packages/console-web");
    let dist = console_pkg.join("dist");
    let gz_dir = console_pkg.join("dist-gz");

    // 监听整个控制台包目录（含 src/scripts/dist/dist-gz）：目录一定存在，
    // cargo 才会记住这条监听；只监听 dist 的话，首次构建时它还不存在，
    // 指纹就永远建立不起来，前端产物变了也不会重新嵌入。
    println!("cargo:rerun-if-changed={}", console_pkg.display());

    let mut files = Vec::new();
    collect(&dist, &dist, &mut files);
    files.sort();

    if files.is_empty() {
        println!(
            "cargo:warning=控制台资产未找到（{}）；请先运行 `pnpm build:console`，否则 /console 返回 503",
            dist.display()
        );
        std::fs::write(
            out.join("console_assets.rs"),
            "pub static ASSETS: &[Asset] = &[];\n",
        )
        .expect("写生成文件");
        return;
    }

    let mut code = String::from("pub static ASSETS: &[Asset] = &[\n");
    let mut total = 0u64;
    for rel in &files {
        let raw_path = dist.join(rel);
        let gz_path = gz_dir.join(format!("{rel}.gz"));
        let raw_len = std::fs::metadata(&raw_path).map(|m| m.len()).unwrap_or(0);
        total += raw_len;
        let gz_expr = if gz_path.exists() {
            format!("Some(include_bytes!({}))", lit(&gz_path))
        } else {
            "None".to_string()
        };
        code.push_str(&format!(
            "    Asset {{ path: {path}, mime: {mime}, raw: include_bytes!({raw}), gz: {gz} }},\n",
            path = lit_str(rel),
            mime = lit_str(mime_of(rel)),
            raw = lit(&raw_path),
            gz = gz_expr,
        ));
    }
    code.push_str("];\n");
    std::fs::write(out.join("console_assets.rs"), code).expect("写生成文件");
    println!(
        "cargo:warning=嵌入控制台资产 {} 个文件 / {} B 原始（压缩变体 {} 个）",
        files.len(),
        total,
        files
            .iter()
            .filter(|f| gz_dir.join(format!("{f}.gz")).exists())
            .count()
    );
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn lit(path: &Path) -> String {
    format!("r#\"{}\"#", path.display())
}

fn lit_str(text: &str) -> String {
    format!("r#\"{text}\"#")
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "map" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

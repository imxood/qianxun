//! 受管发行版目录的回归测试。
//!
//! 真实缺陷：千寻安装器解压官方 zip 后，顶层目录沿用官方命名
//! `node-v<ver>-win-x64`，而扫描器只认裸版本号目录名——装好的 Node
//! 因此永远「未检测到」。本测试用与用户机器一致的目录布局锁死修复。

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use node_runtime::{discover_in, Source};

#[test]
fn managed_store_recognizes_official_release_directory_names() {
    let Some(system_node) = system_node() else {
        // 本机没有可用的 Node 就无法构造「能通过探测」的安装；跳过。
        return;
    };

    let root = std::env::temp_dir().join(format!(
        "qianxun-detect-sim-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let release = root.join("node-v24.0.0-win-x64");
    fs::create_dir_all(&release).expect("release directory");
    // 硬链接零拷贝；跨卷时退回复制。
    if fs::hard_link(&system_node, release.join("node.exe")).is_err() {
        fs::copy(&system_node, release.join("node.exe")).expect("node.exe copy");
    }

    let found = discover_in(Some(&root));
    let _ = fs::remove_dir_all(&root);

    // 版本以二进制的真实输出为准（探测不信任目录名），所以预期值
    // 取自同一个 node.exe 的直接探测结果。
    let managed = found
        .iter()
        .find(|install| install.source == Source::Managed);
    assert_eq!(
        managed.map(|install| install.version.to_string()),
        node_runtime::probe(&system_node).map(|version| version.to_string()),
        "官方命名的受管发行版必须被识别并探测到版本"
    );
}

/// PATH 上的 node 可执行文件（模拟用「真 Node」的来源）。
fn system_node() -> Option<PathBuf> {
    let executable = if cfg!(windows) { "node.exe" } else { "node" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
}

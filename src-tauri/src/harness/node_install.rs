//! Node 一键安装（ADR-005 裸机验收链的第 1 环）。
//!
//! 下载官方 win-x64 zip、按 SHASUMS256.txt 校验、解压进应用数据目录的
//! `node/`——`node-runtime::discover_in` 已经扫描那里，装完即被环境探测
//! 无缝选中。下载用系统自带的 curl.exe：复用安装器的进程树闸门、
//! 输出转发与超时框架，不需要把 reqwest 的 TLS 栈拖进来。

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::install::{
    hide_console_window, run_with_limits, INSTALL_TOTAL_TIMEOUT, PIPE_DRAIN_TIMEOUT,
};
use super::supervisor::Stream;
use super::InstallProgress;
use crate::error::{Error, Result};
use crate::Settings;

/// 千寻自带的 Node 版本：当前 LTS 线上被验证存在且满足
/// `node_runtime::MINIMUM_SUPPORTED` 的具体版本。升级只改这里。
pub const NODE_VERSION: &str = "24.20.0";

/// Windows x64 zip 的文件名（官方与 npmmirror 同名同内容）。
fn archive_name() -> String {
    format!("node-v{NODE_VERSION}-win-x64.zip")
}

/// 单一镜像源描述：显示名 + dist 根目录。
struct DistSource {
    label: &'static str,
    base: &'static str,
}

const OFFICIAL: DistSource = DistSource {
    label: "官方",
    base: "https://nodejs.org/dist",
};
const NPMMIRROR: DistSource = DistSource {
    label: "npmmirror",
    base: "https://registry.npmmirror.com/-/binary/node",
};

fn sources_for(policy: &str) -> Vec<&'static DistSource> {
    match policy {
        "official" => vec![&OFFICIAL],
        "npmmirror" => vec![&NPMMIRROR],
        // auto：官方优先，失败落 npmmirror（默认）。
        _ => vec![&OFFICIAL, &NPMMIRROR],
    }
}

/// 一次 Node 安装的全部输入与产物路径。
#[derive(Clone, Debug)]
pub struct NodeInstallPlan {
    /// 系统自带的 curl.exe（Windows 10 1803+ 标配）。
    pub curl: PathBuf,
    /// dist 目录（含版本号）：zip 与 SHASUMS256.txt 都在这里。
    pub dist: String,
    pub source_label: &'static str,
    /// 下载落位的 zip。
    pub archive: PathBuf,
    /// 解压目标：`node/node-v<ver>-win-x64/`（zip 内同名顶层目录）。
    pub extracted: PathBuf,
}

impl NodeInstallPlan {
    fn url(&self, file: &str) -> String {
        format!("{}/{}", self.dist, file)
    }

    fn curl_command(&self, url: &str, output: &Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.curl);
        command
            .arg("--location")
            .arg("--fail")
            // 静默进度条：curl 的进度用 \r 刷新、不换行，行读取器看不到
            // 活动；改为完全静默 + 放宽空闲时限（下载几百 MB 慢网常见）。
            // 进度改由「落盘字节轮询」上报，见 download_verify_extract。
            .arg("--no-progress-meter")
            .arg("--show-error")
            .arg("--output")
            .arg(output)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        hide_console_window(&mut command);
        command
    }
}

/// 按镜像策略构造计划。curl 缺失（极老系统）在这里就报清楚。
/// （生产路径在 install() 里逐源构造同形计划；这里给测试与后续
/// 复用留一个构造点。）
#[cfg_attr(not(test), allow(dead_code))]
pub fn plan(settings: &Settings, managed_dir: &Path) -> Result<NodeInstallPlan> {
    let curl = PathBuf::from("curl.exe");
    let policy = settings.mirrors.node_binary.as_str();
    let sources = sources_for(policy);
    let source = sources
        .first()
        .copied()
        .ok_or_else(|| Error::Install("镜像源策略为空".to_owned()))?;
    let dist = format!("{}/v{NODE_VERSION}", source.base);
    Ok(NodeInstallPlan {
        curl,
        dist,
        source_label: source.label,
        archive: managed_dir.join(archive_name()),
        extracted: managed_dir.join(format!("node-v{NODE_VERSION}-win-x64")),
    })
}

/// 执行安装：逐源尝试「SHASUMS → zip → 校验 → 解压 → 复核」。
/// 每一步的输出都实时进日志面板，字节级进度经 progress 事件上报。
pub async fn install<R, P>(
    settings: &Settings,
    managed_dir: &Path,
    report: R,
    progress: P,
) -> Result<Option<String>>
where
    R: Fn(Stream, String) + Clone + Send + 'static,
    P: Fn(InstallProgress) + Clone + Send + 'static,
{
    // 幂等入口：已有满足最低版本的 Node 就不动。
    if let Some(found) = managed_node_version(managed_dir).await {
        return Ok(Some(found));
    }
    std::fs::create_dir_all(managed_dir)
        .map_err(|cause| Error::Install(format!("无法创建 {}：{cause}", managed_dir.display())))?;

    let mut last_failure = None;
    for source in sources_for(&settings.mirrors.node_binary) {
        let attempt = NodeInstallPlan {
            curl: PathBuf::from("curl.exe"),
            dist: format!("{}/v{NODE_VERSION}", source.base),
            source_label: source.label,
            archive: managed_dir.join(archive_name()),
            extracted: managed_dir.join(format!("node-v{NODE_VERSION}-win-x64")),
        };
        match download_verify_extract(&attempt, report.clone(), progress.clone()).await {
            Ok(()) => {
                // 装完必须真的能被探测到并满足最低版本——信复核不信退出码。
                return match managed_node_version(managed_dir).await {
                    Some(version) => Ok(Some(version)),
                    None => Err(Error::Install(
                        "解压完成，但 node.exe 无法运行或版本过低".to_owned(),
                    )),
                };
            }
            Err(failure) => {
                report(
                    Stream::Stderr,
                    format!("源「{}」失败：{failure}", source.label),
                );
                let _ = std::fs::remove_file(&attempt.archive);
                last_failure = Some(failure);
            }
        }
    }
    Err(last_failure.unwrap_or_else(|| Error::Install("没有可用的下载源".to_owned())))
}

async fn download_verify_extract<R, P>(plan: &NodeInstallPlan, report: R, progress: P) -> Result<()>
where
    R: Fn(Stream, String) + Clone + Send + 'static,
    P: Fn(InstallProgress) + Clone + Send + 'static,
{
    report(
        Stream::Stdout,
        format!("从源「{}」获取 Node v{NODE_VERSION}", plan.source_label),
    );
    let source = plan.source_label.to_owned();

    // 1. SHASUMS256.txt（几 KB；官方与镜像同源同内容）。
    progress(InstallProgress::NodeManifest {
        source: source.clone(),
    });
    let shasums_path = plan.archive.with_extension("SHASUMS256.txt");
    fetch(
        plan.curl_command(&plan.url("SHASUMS256.txt"), &shasums_path),
        report.clone(),
        "下载校验清单",
    )
    .await?;
    let expected = expected_sha(&shasums_path, &archive_name())?;
    let _ = std::fs::remove_file(&shasums_path);

    // 2. zip 本体。已存在且校验通过就复用（断点续传由重下替代）。
    if !archive_matches(&plan.archive, &expected) {
        let _ = std::fs::remove_file(&plan.archive);
        let url = plan.url(&archive_name());
        // 总大小来自 HEAD 探测；源不配合时进度卡退化为只显示已下载。
        let total_bytes = remote_content_length(&plan.curl, &url).await;
        let downloaded_now = || {
            std::fs::metadata(&plan.archive)
                .map(|meta| meta.len())
                .unwrap_or(0)
        };
        let emit_download = {
            let progress = progress.clone();
            let source = source.clone();
            let url = url.clone();
            move |downloaded_bytes: u64| {
                progress(InstallProgress::NodeDownload {
                    source: source.clone(),
                    url: url.clone(),
                    total_bytes,
                    downloaded_bytes,
                })
            }
        };
        emit_download(downloaded_now());
        // 轮询任务：curl 静默下载时落盘字节是唯一可信进度；500ms 足够
        // 平滑（前端进度条带过渡动画），又不至于刷爆事件通道。
        let watcher = {
            let progress = progress.clone();
            let source = source.clone();
            let url = url.clone();
            let archive = plan.archive.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_millis(500));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    ticker.tick().await;
                    let downloaded_bytes = std::fs::metadata(&archive)
                        .map(|meta| meta.len())
                        .unwrap_or(0);
                    progress(InstallProgress::NodeDownload {
                        source: source.clone(),
                        url: url.clone(),
                        total_bytes,
                        downloaded_bytes,
                    });
                }
            })
        };
        let outcome = fetch(
            plan.curl_command(&url, &plan.archive),
            report.clone(),
            "下载 Node 发行包",
        )
        .await;
        watcher.abort();
        outcome?;
        // 收尾补一发最终字节数：轮询已停，别让进度条停在 99%。
        emit_download(downloaded_now());
    }

    // 3. 校验。
    progress(InstallProgress::NodeFinalize {
        source: source.clone(),
        activity: "校验 SHA-256".to_owned(),
    });
    if !archive_matches(&plan.archive, &expected) {
        let _ = std::fs::remove_file(&plan.archive);
        return Err(Error::Install("SHA-256 校验失败".to_owned()));
    }

    // 4. 解压（覆盖式：清掉旧目录再解，不留半截混合状态）。
    progress(InstallProgress::NodeFinalize {
        source: source.clone(),
        activity: "解压".to_owned(),
    });
    let _ = std::fs::remove_dir_all(&plan.extracted);
    unzip(
        &plan.archive,
        plan.extracted.parent().unwrap_or(Path::new(".")),
    )?;
    if !plan.extracted.join("node.exe").is_file() {
        return Err(Error::Install("压缩包里没有 node.exe".to_owned()));
    }
    Ok(())
}

/// HEAD 探测发行包大小（重定向链取最后一个 content-length）。
/// 探测失败不阻断安装：没有总大小也能如实显示已下载字节。
async fn remote_content_length(curl: &Path, url: &str) -> Option<u64> {
    let mut command = tokio::process::Command::new(curl);
    command
        .arg("--head")
        .arg("--location")
        .arg("--silent")
        .arg("--max-time")
        .arg("15")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    hide_console_window(&mut command);
    let attempt = tokio::time::timeout(Duration::from_secs(20), command.output()).await;
    let output = attempt.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    let headers = String::from_utf8_lossy(&output.stdout);
    headers
        .lines()
        .rfind(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|length| *length > 0)
}

async fn fetch<R>(command: tokio::process::Command, report: R, label: &'static str) -> Result<()>
where
    R: Fn(Stream, String) + Clone + Send + 'static,
{
    // curl 无行输出：空闲上限放宽到 10 分钟（INSTALL_TOTAL_TIMEOUT 仍兜底）。
    run_with_limits(
        command,
        report,
        label,
        Duration::from_secs(10 * 60),
        INSTALL_TOTAL_TIMEOUT,
        PIPE_DRAIN_TIMEOUT,
    )
    .await
}

/// 从 SHASUMS256.txt 里解析目标文件的 sha256（小写十六进制）。
fn expected_sha(shasums: &Path, name: &str) -> Result<String> {
    let body = std::fs::read_to_string(shasums)
        .map_err(|cause| Error::Install(format!("无法读取校验清单：{cause}")))?;
    for line in body.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sha), Some(file)) = (parts.next(), parts.next()) else {
            continue;
        };
        if file == name && sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(sha.to_ascii_lowercase());
        }
    }
    Err(Error::Install(format!("校验清单里没有 {name}")))
}

fn archive_matches(archive: &Path, expected: &str) -> bool {
    let Ok(raw) = std::fs::read(archive) else {
        return false;
    };
    let digest = Sha256::digest(&raw);
    format!("{digest:x}") == expected
}

/// 解压 zip 到目录（zip 内含顶层 node-v<ver>-win-x64/，直接展开）。
fn unzip(archive: &Path, into: &Path) -> Result<()> {
    let reader = std::fs::File::open(archive)
        .map_err(|cause| Error::Install(format!("无法打开发行包：{cause}")))?;
    let mut zip = zip::ZipArchive::new(reader)
        .map_err(|cause| Error::Install(format!("发行包格式损坏：{cause}")))?;
    zip.extract(into)
        .map_err(|cause| Error::Install(format!("解压失败：{cause}")))
}

/// 探测 managed 目录里满足最低版本的 Node，返回其版本号。
/// discover_in 会 spawn `node --version`，调用侧在异步上下文里。
async fn managed_node_version(managed_dir: &Path) -> Option<String> {
    let managed = managed_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        node_runtime::discover_in(Some(&managed))
            .into_iter()
            .find(|install| install.version >= node_runtime::MINIMUM_SUPPORTED)
            .map(|install| install.version.to_string())
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实网络下载链路（SHASUMS → zip → SHA-256 → 解压）。
    /// 默认忽略：只在需要验证下载链路时 `cargo test -- --ignored` 手跑。
    /// 注意不走 install() 的幂等入口——开发机自带的 Node 会被全盘探测
    /// 短路成「已满足」，那样测不到下载本身。
    #[tokio::test]
    #[ignore = "真实网络下载，约 30MB"]
    async fn 真实网络下载与解压链路() {
        let root = scratch("live");
        let settings = Settings::default();
        let plan = plan(&settings, &root).expect("计划");
        download_verify_extract(&plan, |_, _| {}, |_| {})
            .await
            .expect("下载校验解压");
        let node = plan.extracted.join("node.exe");
        assert!(node.is_file(), "缺少 {}", node.display());
        // 解压出来的 node.exe 必须真的能跑。
        let output = std::process::Command::new(&node)
            .arg("--version")
            .output()
            .expect("运行 node");
        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        assert!(text.starts_with("v"), "版本输出异常：{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qianxun-node-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn 校验清单解析大小写与杂行() {
        let root = scratch("shasums");
        let file = root.join("SHASUMS256.txt");
        std::fs::write(
            &file,
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef  other-file.tar.gz\n\
             ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789  node-v22.19.0-win-x64.zip\n\
             not a valid line\n",
        )
        .expect("write");
        let sha = expected_sha(&file, "node-v22.19.0-win-x64.zip").expect("sha");
        assert_eq!(
            sha,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert!(expected_sha(&file, "missing.zip").is_err());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn 镜像策略展开次序() {
        assert_eq!(sources_for("auto").len(), 2);
        assert_eq!(sources_for("auto")[0].label, "官方");
        assert_eq!(sources_for("official").len(), 1);
        assert_eq!(sources_for("npmmirror")[0].label, "npmmirror");
    }

    #[test]
    fn 计划路径随版本与目录落位() {
        let settings = Settings::default();
        let root = scratch("plan");
        let plan = plan(&settings, &root).expect("plan");
        assert!(plan.dist.ends_with(&format!("/v{NODE_VERSION}")));
        assert!(plan
            .archive
            .ends_with(format!("node-v{NODE_VERSION}-win-x64.zip")));
        assert!(plan
            .extracted
            .ends_with(format!("node-v{NODE_VERSION}-win-x64")));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn 哈希匹配对残缺文件说不() {
        let root = scratch("hash");
        let file = root.join("a.zip");
        std::fs::write(&file, b"partial").expect("write");
        assert!(!archive_matches(
            &file,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
        assert!(!archive_matches(&root.join("missing.zip"), "00"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}

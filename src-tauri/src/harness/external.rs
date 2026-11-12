//! 探测用户全局安装的 `dsh` 可执行文件（外部模式：千寻不内置 DSH）。
//!
//! 外部模式下千寻不装、不管、不升级 DSH：它只负责找到用户自己装的
//! `dsh`，按与内置模式完全相同的参数把它拉起来。探测顺序从「最明确
//! 的意图」到「最可能的位置」：
//!
//! 1. 环境变量 `QIANXUN_DSH_BIN`（显式路径，覆盖一切；不可用即失败，
//!    不再往下猜——用户既然点名了某个路径，就不该被静默替换）；
//! 2. PATH 上的 `dsh`（unix）/ `dsh.exe`、`dsh.cmd`（windows）；
//! 3. 已知全局 bin 目录候选（mise shims、`/usr/local/bin`、`~/.local/bin`、
//!    `~/.npm-global/bin`、`~/.nvm/versions/node/*/bin`、`~/AppData/Roaming/npm`）。
//!
//! 版本靠 `<exe> --version` 探测：mise shim 会先打一行 WARN 噪声，
//! 解析时跳过 WARN 行，取第一个 semver 形状的 token。探测有 5 秒时限，
//! 失败只当「版本未知」，绝不把界面卡住。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 显式指定 dsh 可执行文件的环境变量（覆盖一切）。
pub const DSH_BIN_ENV: &str = "QIANXUN_DSH_BIN";

/// `--version` 探测的时限：超过即放弃（宁可说「版本未知」也不卡住界面）。
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// 版本探测的次数上限：PATH 上可能有几十个目录，逐个 spawn 进程太贵。
/// 命中第一个能报出版本的就停；全部都没报出才回落到第一个可用候选。
const VERSION_PROBE_BUDGET: usize = 8;

/// 命中全局 dsh 的结果。
#[derive(Clone, Debug)]
pub struct ResolvedDsh {
    /// 可执行文件路径。
    pub path: PathBuf,
    /// 探测到的版本号（`--version` 解析失败时为 None）。
    pub version: Option<String>,
}

/// 按优先级找一个能用的全局 dsh。找不到返回 None。
pub fn resolve() -> Option<ResolvedDsh> {
    let mut fallback: Option<ResolvedDsh> = None;
    let mut probes = 0usize;
    for candidate in candidates() {
        if !is_usable(&candidate) {
            continue;
        }
        let version = if probes < VERSION_PROBE_BUDGET {
            probes += 1;
            detect_version(&candidate)
        } else {
            None
        };
        let resolved = ResolvedDsh {
            path: candidate,
            version: version.clone(),
        };
        // 报得出版本的可执行文件几乎一定是真 dsh：优先用它。
        if version.is_some() {
            return Some(resolved);
        }
        if fallback.is_none() {
            fallback = Some(resolved);
        }
    }
    fallback
}

/// 候选路径（按优先级）。
fn candidates() -> Vec<PathBuf> {
    let mut list = Vec::new();
    if let Some(explicit) = std::env::var_os(DSH_BIN_ENV).filter(|text| !text.is_empty()) {
        // 显式路径覆盖一切：不可用就直接失败，不再往下猜。
        list.push(PathBuf::from(explicit));
        return list;
    }
    list.extend(path_candidates());
    list.extend(known_bin_candidates());
    list
}

/// PATH 上的 dsh。
fn path_candidates() -> Vec<PathBuf> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for directory in std::env::split_paths(&path) {
        for name in executable_names() {
            list.push(directory.join(name));
        }
    }
    list
}

/// 各平台上 dsh 在 PATH 里的文件名（windows 上没有扩展名的文件不可执行）。
#[cfg(windows)]
fn executable_names() -> &'static [&'static str] {
    &["dsh.exe", "dsh.cmd", "dsh"]
}

#[cfg(not(windows))]
fn executable_names() -> &'static [&'static str] {
    &["dsh"]
}

/// 已知全局 bin 目录。
fn known_bin_candidates() -> Vec<PathBuf> {
    let mut list = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return list;
    };
    // mise shims（远程 Arch 机上 dsh 的实际来源）。
    list.push(
        home.join(".local")
            .join("share")
            .join("mise")
            .join("shims")
            .join("dsh"),
    );
    list.push(PathBuf::from("/usr/local/bin/dsh"));
    list.push(home.join(".local").join("bin").join("dsh"));
    list.push(home.join(".npm-global").join("bin").join("dsh"));
    // windows：npm 全局前缀。
    list.push(
        home.join("AppData")
            .join("Roaming")
            .join("npm")
            .join("dsh.cmd"),
    );
    list.extend(nvm_candidates(&home));
    list
}

/// nvm 装过的 Node 里最新的那个（`~/.nvm/versions/node/*/bin/dsh`）。
fn nvm_candidates(home: &Path) -> Vec<PathBuf> {
    let versions = home.join(".nvm").join("versions").join("node");
    let Ok(entries) = std::fs::read_dir(&versions) else {
        return Vec::new();
    };
    let mut found: Vec<(Vec<u64>, PathBuf)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.path().is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = version_key(&name);
            Some((key, entry.path().join("bin").join("dsh")))
        })
        .collect();
    // 版本号大的更新（v24.19.0 > v9.1.0：字典序会错，必须按数字比）。
    found.sort_by(|left, right| left.0.cmp(&right.0));
    found.into_iter().map(|(_, path)| path).collect()
}

/// 从目录名里抽出数字段（`v24.19.0` → [24, 19, 0]；认不出 → 空）。
fn version_key(name: &str) -> Vec<u64> {
    name.trim_start_matches('v')
        .split(['.', '-'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// 候选是否真的能用：文件在，且（unix 上）有可执行位。
fn is_usable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    has_executable_bit(&metadata)
}

/// unix 上检查可执行位（mode & 0o111）。
#[cfg(unix)]
fn has_executable_bit(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

/// windows 上不看权限位（能列出来就交给 spawn 判定）。
#[cfg(not(unix))]
fn has_executable_bit(_metadata: &std::fs::Metadata) -> bool {
    true
}

/// 跑一次 `<exe> --version` 并解析出版本号。失败返回 None。
pub fn detect_version(exe: &Path) -> Option<String> {
    let (stdout, stderr) = run_capture(exe, &["--version"], VERSION_TIMEOUT)?;
    parse_version(&stdout).or_else(|| parse_version(&stderr))
}

/// 从输出里解析版本号：跳过 WARN 噪声行，取第一个 semver 形状的 token。
fn parse_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // mise shim 会先打一行 WARN 噪声（有时还带版本号样的数字）。
        if trimmed.to_ascii_lowercase().contains("warn") {
            continue;
        }
        if let Some(version) = find_semver_token(trimmed) {
            return Some(version);
        }
    }
    None
}

/// 行内第一个 semver 形状的 token（`0.2.2`、`0.2.2-rc.1`、`v1.2.3`）。
fn find_semver_token(line: &str) -> Option<String> {
    for raw in line.split_whitespace() {
        let token = raw.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric()
                && character != '.'
                && character != '-'
                && character != '+'
        });
        let core = token.strip_prefix('v').unwrap_or(token);
        let numeric = core.split(['-', '+']).next().unwrap_or("");
        let parts: Vec<&str> = numeric.split('.').collect();
        if parts.len() < 3 {
            continue;
        }
        let all_digits = parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
        if all_digits {
            return Some(core.to_owned());
        }
    }
    None
}

/// 带时限地跑一条命令并收下 stdout/stderr。超时或失败返回 None。
fn run_capture(program: &Path, args: &[&str], timeout: Duration) -> Option<(String, String)> {
    let (spawn_program, spawn_args) = spawn_target(program, args);
    let mut command = std::process::Command::new(&spawn_program);
    command
        .args(&spawn_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let stderr = child.stderr.take()?;
    let out_handle = std::thread::spawn(move || read_all(stdout));
    let err_handle = std::thread::spawn(move || read_all(stderr));

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    Some((stdout, stderr))
}

/// 实际要 spawn 的程序与参数。
///
/// windows 上 `.cmd`/`.bat` 不是可执行文件（CreateProcess 直接失败），
/// 必须经 `cmd.exe /C` 启动；参数次序保持不变。
fn spawn_target(program: &Path, args: &[&str]) -> (OsString, Vec<OsString>) {
    #[cfg(windows)]
    {
        let extension = program
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if extension == "cmd" || extension == "bat" {
            let mut full = vec![OsString::from("/C"), program.as_os_str().to_owned()];
            full.extend(args.iter().map(|value| OsString::from(*value)));
            return (OsString::from("cmd.exe"), full);
        }
    }
    (
        program.as_os_str().to_owned(),
        args.iter().map(|value| OsString::from(*value)).collect(),
    )
}

/// 读干一个管道。
fn read_all<R: std::io::Read>(mut reader: R) -> String {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 版本号从噪声行里被挑出来() {
        // mise shim 的典型输出：先一行 WARN，再报版本。
        let text = "WARN  mise: version not installed\ndsh 0.2.2-rc.1\n";
        assert_eq!(parse_version(text).as_deref(), Some("0.2.2-rc.1"));
    }

    #[test]
    fn 版本号接受多种写法() {
        assert_eq!(parse_version("0.2.2").as_deref(), Some("0.2.2"));
        assert_eq!(parse_version("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(
            parse_version("dsh version 0.2.2-rc.2").as_deref(),
            Some("0.2.2-rc.2")
        );
        assert_eq!(
            parse_version("[DSH] 0.2.2 (build 42)").as_deref(),
            Some("0.2.2")
        );
    }

    #[test]
    fn 没有版本号时返回空() {
        assert!(parse_version("").is_none());
        assert!(parse_version("command not found").is_none());
        assert!(parse_version("WARN  mise: 2025.1.1 not installed").is_none());
    }

    #[test]
    fn 目录名版本号按数字排序() {
        let mut keys = [
            version_key("v9.1.0"),
            version_key("v24.19.0"),
            version_key("v10.0.0"),
        ];
        keys.sort();
        assert_eq!(keys[0], vec![9, 1, 0]);
        assert_eq!(keys[2], vec![24, 19, 0]);
    }

    #[test]
    fn 不存在的文件不可用() {
        assert!(!is_usable(Path::new("/definitely/not/here/dsh")));
    }
}

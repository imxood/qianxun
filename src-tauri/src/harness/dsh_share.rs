//! 复用用户自己已经在跑的 `dsh web`（方案 A）。
//!
//! 背景：千寻托管的 DSH 与用户在终端里手起的 DSH 共用同一份 `~/.dsh`，
//! 同一把会话锁会互相踢——表现为 "This session is already in use"。
//! 根治办法不是再抢一把锁，而是**复用已经在跑的那个实例**：连上去、
//! 把它当成自己的 DSH 用，从此只有一个实例、一把锁。
//!
//! 凭据从哪来：DSH web 的启动 token 只活在进程内存里（stdout 打印的
//! `?token=` 换到 cookie 后即失效，日志里也没有），拿不到。但 DSH_HOME
//! 下的 `.credentials.yaml` 里存着**持久签名密钥**，用它自签一张与启动
//! token 等价的浏览器会话 cookie，就能直接连上任意在跑的 dsh web。
//!
//! 取舍：
//! - **全程只读**：不杀进程、不写文件、不启动进程；探测只有 GET。
//! - 端口枚举走 `/proc/net/tcp`（Linux）/ `netstat`（Windows），不引额外
//!   依赖；其它平台直接放弃（返回 None）而不是瞎猜。
//! - 探测**并发**：所有候选端口同时探，整体墙钟约等于单端口超时，与
//!   端口数量无关（串行探测会让环境页的探测按钮卡住几十秒）。
//! - 密钥读不到就放弃：自造密钥去撞 401 只是浪费一轮探测。

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use reqwest::header::COOKIE;
use reqwest::Client;
use serde_yaml::Value;
use sha2::{Digest, Sha256};

/// HMAC-SHA256：DSH 的 cookie 签名算法。
type HmacSha256 = Hmac<Sha256>;

/// DSH web 未鉴权时的响应体特征串（实测 401 响应体里就有这句）。
const AUTH_MARKER: &str = "dsh web authentication required";

/// 浏览器会话 cookie 的名前缀。
const COOKIE_PREFIX: &str = "dsh-auth-";

/// 单个端口的探测超时。
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// 候选端口上限：机器上监听端口多得离谱时只探这些。
const MAX_CANDIDATES: usize = 128;

/// 临时端口下限（Linux 32768+ / Windows 49152+ 从这里起）。
const EPHEMERAL_FLOOR: u16 = 32768;

/// 优先探测的常见 dsh web 端口。
///
/// DSH 自己的默认 web 端口是 3080，千寻托管的实例占 17300 段；
/// 用户手起的实例多半落在这几个上（或临时端口）。把它们单列出来，
/// 是为了不被「端口多的机器上先探临时端口」这条规则挤掉。
const PRIORITY_PORTS: [u16; 4] = [3080, 17300, 3000, 8080];

/// 自签 cookie 的有效期（毫秒）。
///
/// 服务端拿 `Date.now()`（**毫秒**）跟 `issuedAt`/`expiresAt` 比：
/// `issuedAt > now || expiresAt <= now` 一律判为无效——单位写错就等于
/// 「已过期」，表现为上游一律 401。
///
/// 有效期上限是服务端的 `cookieMaxAgeDays`（默认 30 天），超出同样被拒；
/// 这里留 1 分钟余量，保证 `expiresAt - issuedAt` 严格小于上限。
const COOKIE_TTL_MILLISECONDS: u64 = 30 * 24 * 60 * 60 * 1000 - 60_000;

/// 签名密钥文件相对 DSH_HOME 的名字。
const CREDENTIALS_FILE: &str = ".credentials.yaml";

/// 凭据记录在 YAML 里的键。
const SESSION_RECORD_KEY: &str = "client-connection/browser-session";

/// 已在运行的 DSH 实例（可以直接连上去的那种）。
#[derive(Clone, Debug)]
pub struct SharedInstance {
    /// origin，形如 `127.0.0.1:3080`（无 scheme）。
    pub origin: String,
    /// 自签的浏览器会话 cookie，形如 `dsh-auth-xxx=v1.yyy.zzz`（Name=Value）。
    pub cookie: String,
    /// 持有该实例的进程 pid（探测得到时填，None 不影响使用）。
    pub pid: Option<u32>,
}

/// 探测本机是否已有在跑的 dsh web；有就返回可直接使用的实例（含自签 cookie）。
///
/// 全程只读：不杀进程、不写文件、不启动进程。找不到返回 None。
///
/// `exclude_ports` 是**要避开的端口**：千寻自己的回环网关就监听在其中
/// 一个上，而网关会把请求转发给上游 DSH —— 上游没带 cookie 时回的正是
/// 「dsh web authentication required」。不排除它，千寻会把**自己**当成
/// 可复用的实例接上去（自环）。
pub async fn detect(exclude_ports: &[u16]) -> Option<SharedInstance> {
    // 密钥是「能不能连上」的前提：读不到就直接放弃，别白扫一遍端口。
    let keys = load_secret_keys();
    if keys.is_empty() {
        return None;
    }
    let candidates = select_candidates(candidate_listeners())
        .into_iter()
        .filter(|candidate| !exclude_ports.contains(&candidate.port))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }

    let client = match probe_client() {
        Some(client) => client,
        None => return None,
    };

    let mut hits = probe_all(&client, &candidates).await;
    // 多个端口都命中时取端口号最小的：选择稳定，不随探测顺序漂移。
    hits.sort_by_key(|candidate| candidate.port);

    for candidate in hits {
        let origin = format!("127.0.0.1:{}", candidate.port);
        // 多个密钥来源（DSH_HOME 环境变量 / ~/.dsh）逐个试：签错密钥的
        // cookie 会被 401 挡下，正好当作「这个密钥不属于这个实例」的判据。
        for key in &keys {
            let Some(cookie) = forge_cookie_with(&origin, key, now_millis()) else {
                continue;
            };
            if verify_with_cookie(&client, &origin, &cookie).await {
                let pid = candidate
                    .pid
                    .or_else(|| candidate.inode.and_then(pid_for_inode));
                return Some(SharedInstance {
                    origin,
                    cookie,
                    pid,
                });
            }
        }
    }
    None
}

/// 自签并**当场验签**的 cookie：逐个密钥试，谁能让上游回 200 就用谁。
///
/// 用于「已有的 cookie 失效要重签」这条路——重签时不能凭猜：密钥属于
/// 哪个 DSH_HOME 只有上游知道，验过才算数。
pub async fn issue_verified_cookie(origin: &str) -> Option<String> {
    let authority = normalize_authority(origin)?;
    let client = probe_client()?;
    for key in load_secret_keys() {
        let Some(cookie) = forge_cookie_with(&authority, &key, now_millis()) else {
            continue;
        };
        if verify_with_cookie(&client, &authority, &cookie).await {
            return Some(cookie);
        }
    }
    None
}

/// 探测用的 HTTP client：回环探针不走系统代理，也不跟随重定向。
fn probe_client() -> Option<Client> {
    reqwest::Client::builder()
        // 重定向一律不跟随：我们要的是「这个端口是不是 dsh web」的原始应答。
        .redirect(reqwest::redirect::Policy::none())
        // 回环探针不该走系统代理：HTTP_PROXY 指向远端时，
        // 请求会被送到代理而不是本机端口，探测结论直接失真。
        .no_proxy()
        .build()
        .ok()
}

// ---------------------------------------------------------------------------
// 探测
// ---------------------------------------------------------------------------

/// 一个候选监听端口（附能查到的进程线索）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    port: u16,
    /// 进程 pid：Windows 上 netstat 直接给；Linux 上靠 inode 反查。
    pid: Option<u32>,
    /// `/proc/net/tcp` 里的 socket inode（Linux 反查 pid 用）。
    inode: Option<u64>,
}

/// 并发探测所有候选端口；返回「确实是 dsh web」的那些。
///
/// 并发是这里唯一重要的性能决定：串行探 60 个端口最坏要 90 秒，
/// 并发则约等于一次超时。
async fn probe_all(client: &Client, candidates: &[Candidate]) -> Vec<Candidate> {
    let probes = candidates
        .iter()
        .map(|candidate| probe_one(client.clone(), candidate.clone()));
    futures_util::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// 探一个端口：状态 401 且响应体含鉴权提示，才认定它是 dsh web。
///
/// 认 401 而不是认状态码 200/其它：任何服务都可能回 200，但那串
/// 「dsh web authentication required」是 DSH 自己的话。
async fn probe_one(client: Client, candidate: Candidate) -> Option<Candidate> {
    let url = format!("http://127.0.0.1:{}/", candidate.port);
    let response = match tokio::time::timeout(PROBE_TIMEOUT, client.get(&url).send()).await {
        Ok(Ok(response)) => response,
        _ => return None,
    };
    if response.status().as_u16() != 401 {
        return None;
    }
    let body = match tokio::time::timeout(PROBE_TIMEOUT, response.text()).await {
        Ok(Ok(body)) => body,
        _ => return None,
    };
    body.contains(AUTH_MARKER).then_some(candidate)
}

/// 带自签 cookie 再探一次：状态 200 才算真的能用。
async fn verify_with_cookie(client: &Client, origin: &str, cookie: &str) -> bool {
    let url = format!("http://{origin}/");
    let response = match tokio::time::timeout(
        PROBE_TIMEOUT,
        client.get(&url).header(COOKIE, cookie).send(),
    )
    .await
    {
        Ok(Ok(response)) => response,
        _ => return false,
    };
    response.status().as_u16() == 200
}

/// 候选端口的排序与裁剪。
///
/// 端口多的机器上不能把监听端口全探一遍（一次探测就花掉一次超时的量），
/// 所以分三档取：
/// 1. **常用 dsh web 端口**（`PRIORITY_PORTS`）：永远保留，
///    3080 这类端口不会因为机器上别的服务多而被挤掉；
/// 2. **临时端口**（>= `EPHEMERAL_FLOOR`）：降序取，用户随手起的实例
///    最可能落在临时端口段，端口号大的通常是最近起的；
/// 3. **其余固定端口**：从低到高补齐名额。
fn select_candidates(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.sort_by_key(|candidate| candidate.port);
    candidates.dedup_by_key(|candidate| candidate.port);

    let mut priority: Vec<Candidate> = Vec::new();
    let mut ephemeral: Vec<Candidate> = Vec::new();
    let mut fixed: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if PRIORITY_PORTS.contains(&candidate.port) {
            priority.push(candidate);
        } else if candidate.port >= EPHEMERAL_FLOOR {
            ephemeral.push(candidate);
        } else {
            fixed.push(candidate);
        }
    }
    // 临时端口降序：先探最新的。
    ephemeral.reverse();

    let mut selected: Vec<Candidate> = Vec::with_capacity(MAX_CANDIDATES);
    selected.extend(priority);
    selected.extend(ephemeral.into_iter().take(MAX_CANDIDATES - selected.len()));
    selected.extend(fixed.into_iter().take(MAX_CANDIDATES - selected.len()));
    selected
}

// ---------------------------------------------------------------------------
// 端口枚举（平台相关）
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn candidate_listeners() -> Vec<Candidate> {
    let mut candidates = Vec::new();
    // IPv4 与 IPv6 各一份：DSH 可能只监听 ::1。
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        candidates.extend(parse_proc_net(&text));
    }
    candidates
}

#[cfg(target_os = "windows")]
fn candidate_listeners() -> Vec<Candidate> {
    let Some(text) = run_netstat() else {
        return Vec::new();
    };
    parse_netstat(&text)
}

/// 其它平台没有可移植的监听端口枚举：如实放弃，不猜。
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn candidate_listeners() -> Vec<Candidate> {
    Vec::new()
}

/// 解析 `/proc/net/tcp`（或 tcp6）文本，取 LISTEN 状态的端口与 inode。
///
/// 纯函数：格式来自内核，不依赖运行平台，测试可以直接喂样例行。
///
/// `allow(dead_code)`：本函数只被 Linux 分支调用，但在 Windows 上
/// 仍要存在——单测在两个平台都要能跑同一份样例。
#[allow(dead_code)]
fn parse_proc_net(text: &str) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // 0: sl | 1: local_address | 2: rem_address | 3: st | … | 9: inode
        if fields.len() < 10 {
            continue;
        }
        // `0A` = TCP_LISTEN。
        if fields[3] != "0A" {
            continue;
        }
        let Some(port) = fields[1]
            .rsplit(':')
            .next()
            .and_then(|hex| u16::from_str_radix(hex, 16).ok())
        else {
            continue;
        };
        candidates.push(Candidate {
            port,
            pid: None,
            inode: fields[9].parse::<u64>().ok(),
        });
    }
    candidates
}

/// 解析 `netstat -ano -p tcp` 输出，取 LISTENING 行的端口与 pid。
///
/// `allow(dead_code)`：同 `parse_proc_net`——两个平台的单测共用同一份
/// 解析器，避免两套格式理解各自漂移。
#[allow(dead_code)]
fn parse_netstat(text: &str) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // TCP | 本地地址 | 远端地址 | 状态 | PID
        if fields.len() < 5 {
            continue;
        }
        if !fields[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if !fields[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        let Some(port) = fields[1]
            .rsplit(':')
            .next()
            .and_then(|text| text.parse::<u16>().ok())
        else {
            continue;
        };
        candidates.push(Candidate {
            port,
            pid: fields[4].parse::<u32>().ok(),
            inode: None,
        });
    }
    candidates
}

#[cfg(target_os = "windows")]
fn run_netstat() -> Option<String> {
    let mut command = std::process::Command::new("netstat");
    command
        .args(["-ano", "-p", "tcp"])
        .stdin(std::process::Stdio::null());
    // 一次性子进程不该在用户桌面上闪一个控制台窗。
    proc_guard::hide_console(&mut command);
    let output = command.output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 由 socket inode 反查持有它的进程 pid（Linux）。
///
/// `/proc/net/tcp` 只给 inode，pid 要靠扫 `/proc/<pid>/fd` 的软链反查；
/// 查不到就留 None——pid 只是给人看的线索，不影响连接。
#[cfg(target_os = "linux")]
fn pid_for_inode(inode: u64) -> Option<u32> {
    let target = format!("socket:[{inode}]");
    let entries = std::fs::read_dir("/proc").ok()?;
    let mut processes: Vec<_> = entries.flatten().collect();
    // 按 pid 排序：同一台机器上多个进程持有同一 socket 时取小的，结果稳定。
    processes.sort_by_key(|entry| entry.file_name());
    for process in processes {
        let name = process.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Ok(descriptors) = std::fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        for descriptor in descriptors.flatten() {
            if std::fs::read_link(descriptor.path())
                .is_ok_and(|link| link.to_string_lossy() == target)
            {
                return Some(pid);
            }
        }
    }
    None
}

/// 非 Linux 平台没有 inode → pid 的可移植办法。
#[cfg(not(target_os = "linux"))]
fn pid_for_inode(_inode: u64) -> Option<u32> {
    None
}

// ---------------------------------------------------------------------------
// 自签 cookie
// ---------------------------------------------------------------------------

/// cookie 的名字：前缀 + authority 的 sha256（base64url 无 padding）。
///
/// 名字跟着 authority 走：同一端口换端口就是另一张 cookie，
/// 服务端按名字挑出对应 authority 的那一张。
fn cookie_name(authority: &str) -> String {
    let digest = Sha256::digest(authority.as_bytes());
    format!("{COOKIE_PREFIX}{}", URL_SAFE_NO_PAD.encode(digest))
}

/// cookie 的载荷体：手拼 JSON。
///
/// 字段固定、取值全是我们自己拼出来的（authority 只含 host:port 字符），
/// 为此引一个序列化框架不值得。
fn cookie_body(authority: &str, issued_at: u64, expires_at: u64) -> String {
    format!(
        "{{\"version\":1,\"authority\":\"{}\",\"issuedAt\":{issued_at},\"expiresAt\":{expires_at}}}",
        escape_json_string(authority)
    )
}

/// JSON 字符串里的最小转义：引号与反斜杠必须处理，
/// 否则 authority 里一个引号就能把整个载荷拼坏。
fn escape_json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// 用持久密钥给载荷体签名（HMAC-SHA256，base64url 无 padding）。
///
/// 签名对象是**载荷体的 base64url 文本**，不是原文——服务端按同样
/// 顺序校验，差一个字符就对不上。
fn sign_body(body_encoded: &str, key: &[u8]) -> Option<String> {
    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(body_encoded.as_bytes());
    Some(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// 自签一张 cookie（`Name=Value`），纯函数：无文件、无网络、无时钟。
fn forge_cookie_with(authority: &str, key: &[u8], issued_at: u64) -> Option<String> {
    let expires_at = issued_at + COOKIE_TTL_MILLISECONDS;
    let body = cookie_body(authority, issued_at, expires_at);
    let body_encoded = URL_SAFE_NO_PAD.encode(body.as_bytes());
    let signature = sign_body(&body_encoded, key)?;
    let value = format!("v1.{body_encoded}.{signature}");
    Some(format!("{}={}", cookie_name(authority), value))
}

/// origin → cookie 里的 authority：只保留 `host:port`。
///
/// 服务端比对的是 authority（无 scheme），调用方却可能塞进
/// `http://127.0.0.1:3080`——这里统一剥掉前缀。
fn normalize_authority(origin: &str) -> Option<String> {
    let trimmed = origin.trim().trim_end_matches('/');
    let authority = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    if authority.is_empty() || !authority.contains(':') {
        return None;
    }
    Some(authority.to_owned())
}

/// 读取持久签名密钥（base64url 文本解码成字节）。
///
/// 可能不止一份：千寻自己可能跑在别的 DSH_HOME 里（嵌套在 DSH 会话中
/// 启动时，DSH_HOME 环境变量指向的是会话的私有 home），而用户手起的
/// 实例用的是系统 `~/.dsh`。两份都读，谁能让上游认账就用谁。
fn load_secret_keys() -> Vec<Vec<u8>> {
    let mut keys: Vec<Vec<u8>> = Vec::new();
    for home in candidate_homes() {
        let Ok(raw) = std::fs::read_to_string(home.join(CREDENTIALS_FILE)) else {
            continue;
        };
        let Some(secret) = secret_from_yaml(&raw) else {
            continue;
        };
        let Some(key) = decode_secret(&secret) else {
            continue;
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// 候选 DSH_HOME 目录：环境变量优先，然后系统 `~/.dsh`（去重）。
fn candidate_homes() -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("DSH_HOME").filter(|value| !value.is_empty()) {
        homes.push(PathBuf::from(dir));
    }
    if let Some(home) = dirs::home_dir() {
        let system = home.join(".dsh");
        if !homes.contains(&system) {
            homes.push(system);
        }
    }
    homes
}

/// 从 `.credentials.yaml` 文本里取出签名密钥（纯函数，便于测试）。
fn secret_from_yaml(raw: &str) -> Option<String> {
    let document: Value = serde_yaml::from_str(raw).ok()?;
    let records = document.get("records")?;
    // 标准键找不到就按后缀兜底：DSH 不同版本的记录键可能带前缀。
    let record = records
        .get(SESSION_RECORD_KEY)
        .or_else(|| find_session_record(records))?;
    let secret = record.get("payload")?.get("secret")?.as_str()?.trim();
    (!secret.is_empty()).then_some(secret.to_owned())
}

/// 在 records 里找键名以 `browser-session` 结尾的那条记录。
fn find_session_record(records: &Value) -> Option<&Value> {
    let mapping = records.as_mapping()?;
    mapping
        .iter()
        .find(|(key, _)| {
            key.as_str()
                .is_some_and(|key| key.ends_with("browser-session"))
        })
        .map(|(_, value)| value)
}

/// base64url 解码：容忍缺 padding 与标准 base64 字母表。
fn decode_secret(secret: &str) -> Option<Vec<u8>> {
    let trimmed = secret.trim();
    let without_padding = trimmed.trim_end_matches('=');
    URL_SAFE_NO_PAD
        .decode(without_padding)
        .ok()
        .or_else(|| URL_SAFE.decode(trimmed).ok())
        .or_else(|| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(without_padding)
                .ok()
        })
        .or_else(|| {
            base64::engine::general_purpose::STANDARD
                .decode(trimmed)
                .ok()
        })
}

/// 当前 unix 毫秒（签发时间）。
///
/// **必须是毫秒**：服务端用 `Date.now()` 比对，秒级时间戳会被判成
/// 「早于签发时刻且早已过期」，表现为一律 401。
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一段 32 字节的假密钥（base64url 无 padding，与真实文件同形）。
    const FAKE_SECRET: &str = "8J3vQ2xW7pLzR1kY9mNbC4dE6fH0gA5iJ7lK2nO8pQ4";

    const SAMPLE_CREDENTIALS: &str = r#"version: 1
refs: {}
records:
  client-connection/browser-session:
    kind: grant
    payload:
      version: 1
      secret: "8J3vQ2xW7pLzR1kY9mNbC4dE6fH0gA5iJ7lK2nO8pQ4"
"#;

    #[test]
    fn cookie名只取决于authority() {
        let first = cookie_name("127.0.0.1:3080");
        assert_eq!(first, cookie_name("127.0.0.1:3080"));
        assert!(first.starts_with(COOKIE_PREFIX));
        assert_ne!(first, cookie_name("127.0.0.1:3081"));
        assert_ne!(first, cookie_name("127.0.0.2:3080"));
    }

    #[test]
    fn cookie值是v1开头的三段() {
        let key = decode_secret(FAKE_SECRET).expect("假密钥可解码");
        let cookie = forge_cookie_with("127.0.0.1:3080", &key, 1_700_000_000).expect("自签");
        let (name, value) = cookie.split_once('=').expect("Name=Value");
        assert!(name.starts_with(COOKIE_PREFIX));
        let parts: Vec<&str> = value.split('.').collect();
        assert_eq!(parts.len(), 3, "值应是 v1.body.sig 三段");
        assert_eq!(parts[0], "v1");
        // body 段解回来是那段 JSON。
        let body = URL_SAFE_NO_PAD.decode(parts[1]).expect("body 可解码");
        let body = String::from_utf8(body).expect("body 是 utf8");
        assert!(body.contains("\"authority\":\"127.0.0.1:3080\""));
        assert!(body.contains("\"version\":1"));
    }

    #[test]
    fn 签名依赖密钥且可复现() {
        let key = decode_secret(FAKE_SECRET).expect("假密钥可解码");
        let other = vec![7u8; 32];
        let first = forge_cookie_with("127.0.0.1:3080", &key, 1).expect("自签");
        let again = forge_cookie_with("127.0.0.1:3080", &key, 1).expect("自签");
        let different = forge_cookie_with("127.0.0.1:3080", &other, 1).expect("自签");
        assert_eq!(first, again, "同样输入必须同样输出");
        assert_ne!(first, different, "换密钥必须换签名");
    }

    /// 时间戳必须是**毫秒**。
    ///
    /// 服务端拿 `Date.now()`（毫秒）跟 `issuedAt`/`expiresAt` 比：
    /// 秒级时间戳会被判成「早已过期」，表现为上游一律 401。
    /// 这个坑真实踩过一次，钉一条回归测试。
    #[test]
    fn 时间戳是毫秒不是秒() {
        let now = now_millis();
        assert!(now > 1_600_000_000_000, "now_millis 必须是毫秒级时间戳");
        assert!(now < 4_000_000_000_000, "now_millis 不该是秒级时间戳");
    }

    /// 有效期也在毫秒尺度上：服务端要求
    /// `expiresAt - issuedAt <= cookieMaxAgeDays`（默认 30 天，单位毫秒）。
    #[test]
    fn 有效期是毫秒且不超过服务端上限() {
        let key = decode_secret(FAKE_SECRET).expect("假密钥可解码");
        let issued_at = now_millis();
        let cookie = forge_cookie_with("127.0.0.1:3080", &key, issued_at).expect("自签");
        let (_, value) = cookie.split_once('=').expect("Name=Value");
        let body = URL_SAFE_NO_PAD
            .decode(value.split('.').nth(1).expect("body 段"))
            .expect("body 可解码");
        let body = String::from_utf8(body).expect("body 是 utf8");
        let expires_at: u64 = body
            .split("\"expiresAt\":")
            .nth(1)
            .expect("expiresAt 字段")
            .split(|character: char| !character.is_ascii_digit())
            .next()
            .expect("数字")
            .parse()
            .expect("毫秒数");
        let ttl = expires_at - issued_at;
        assert_eq!(ttl, COOKIE_TTL_MILLISECONDS);
        assert!(
            ttl <= 30 * 24 * 60 * 60 * 1000,
            "有效期不能超过服务端上限（30 天）"
        );
    }

    #[test]
    fn base64url编解码往返() {
        let raw = b"\x00\x01\xfe\xff the quick brown fox";
        let encoded = URL_SAFE_NO_PAD.encode(raw);
        assert!(!encoded.contains('='), "无 padding");
        assert!(
            !encoded.contains('+') && !encoded.contains('/'),
            "url 字母表"
        );
        assert_eq!(URL_SAFE_NO_PAD.decode(&encoded).expect("往返"), raw);
        // 带 padding 的写法也要能读（真实文件里可能有）。
        let padded = base64::engine::general_purpose::STANDARD.encode(raw);
        assert_eq!(decode_secret(&padded), Some(raw.to_vec()));
    }

    #[test]
    fn yaml里取出签名密钥() {
        assert_eq!(
            secret_from_yaml(SAMPLE_CREDENTIALS).as_deref(),
            Some(FAKE_SECRET)
        );
    }

    #[test]
    fn 密钥缺失时取不到() {
        // 没有 records、没有 secret、内容不是 YAML：都必须是 None，
        // 而不是 panic 或空串（空串会拿去签一张必然被拒的 cookie）。
        assert_eq!(secret_from_yaml("version: 1\n"), None);
        assert_eq!(secret_from_yaml("records: {}\n"), None);
        assert_eq!(secret_from_yaml("这不是 YAML: ["), None);
        assert_eq!(secret_from_yaml(""), None);
        assert_eq!(
            secret_from_yaml("records:\n  other:\n    payload:\n      secret: \"\"\n"),
            None
        );
    }

    #[test]
    fn authority归一化只留host与端口() {
        assert_eq!(
            normalize_authority("http://127.0.0.1:3080").as_deref(),
            Some("127.0.0.1:3080")
        );
        assert_eq!(
            normalize_authority("127.0.0.1:3080/").as_deref(),
            Some("127.0.0.1:3080")
        );
        assert_eq!(normalize_authority("127.0.0.1"), None);
        assert_eq!(normalize_authority(""), None);
    }

    #[test]
    fn proc_net只取监听行() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
             0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000 100 0 0 10 0\n\
             1: 0100007F:0BB8 0100007F:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0000 100 0 0 10 0\n";
        let candidates = parse_proc_net(text);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].port, 8080);
        assert_eq!(candidates[0].inode, Some(12345));
    }

    #[test]
    fn netstat只取监听行并带pid() {
        let text = "Active Connections\n\n\
             Proto  Local Address          Foreign Address        State           PID\n\
             TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       984\n\
             TCP    127.0.0.1:3080         0.0.0.0:0              LISTENING       12345\n\
             TCP    127.0.0.1:52000        127.0.0.1:3080         ESTABLISHED     555\n\
             UDP    0.0.0.0:5353           *:*                                    1200\n";
        let candidates = parse_netstat(text);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].port, 135);
        assert_eq!(candidates[0].pid, Some(984));
        assert_eq!(candidates[1].port, 3080);
        assert_eq!(candidates[1].pid, Some(12345));
    }

    #[test]
    fn 候选端口有上限且保得住常用端口() {
        let mut candidates: Vec<Candidate> = (1..400u16)
            .map(|port| Candidate {
                port,
                pid: None,
                inode: None,
            })
            .collect();
        for port in [3080u16, 60000, 60100] {
            candidates.push(Candidate {
                port,
                pid: None,
                inode: None,
            });
        }
        let selected = select_candidates(candidates);
        assert!(selected.len() <= MAX_CANDIDATES);
        // 常用 dsh web 端口永远保留，不会被机器上别的服务挤掉。
        assert!(selected.iter().any(|candidate| candidate.port == 3080));
        // 临时端口段降序：先探最新的。
        let ephemeral: Vec<u16> = selected
            .iter()
            .map(|candidate| candidate.port)
            .filter(|port| *port >= EPHEMERAL_FLOOR)
            .collect();
        assert_eq!(ephemeral, vec![60100, 60000]);
    }

    #[test]
    fn 候选端口去重() {
        let candidates = select_candidates(vec![
            Candidate {
                port: 3080,
                pid: Some(1),
                inode: None,
            },
            Candidate {
                port: 3080,
                pid: None,
                inode: Some(9),
            },
        ]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].port, 3080);
    }

    /// 手工探针：真的去探测本机正在跑的 dsh web。
    ///
    /// 默认跳过——它会连本机端口并用自签 cookie 请求上游：
    /// `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored 手工探测共享实例`
    #[test]
    #[ignore]
    fn 手工探测共享实例() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("运行时");
        let found = runtime.block_on(detect(&[]));
        println!("探测结果：{found:#?}");
        assert!(found.is_some(), "本机应能探测到正在跑的 dsh web");
    }
}

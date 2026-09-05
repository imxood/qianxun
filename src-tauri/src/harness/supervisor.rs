//! 让一个 `dsh web` 进程保持存活且可观察。
//!
//! 千寻对外壳的承诺：本地服务不会悄悄消失。为此这里统一拥有它的
//! 完整生命周期——有界的启动等待、流式输出、崩溃检测、退避重启——
//! 并以一个 UI 能如实渲染的状态机暴露出去。
//!
//! 固定端口（ADR-002）：启动前预检端口可用；就绪行报告的端口必须与
//! 配置一致，不一致视为启动失败——防止「以为在 A 端口、实际在 B 端口」。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use proc_guard::ProcessGuard;
use serde::Serialize;
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, oneshot};

use super::health;
use super::readiness::{self, Ready};
use crate::error::{Error, Result};

/// 冷启动要从磁盘装载上百个插件，超时给得宽：更紧的阈值惩罚的是
/// 慢磁盘而不是真故障。
const READINESS_TIMEOUT: Duration = Duration::from_secs(120);

/// 意外退出后的退避序列；用完即放弃。
const RESTART_DELAYS_MS: [u64; 5] = [500, 1_000, 2_000, 5_000, 10_000];

/// 启动失败时保留的 stderr 行数：足以定位装载器故障，又不至于在
/// 脱离的泵任务里留下无界的进程日志。
const STARTUP_STDERR_LINES: usize = 160;
const STARTUP_STDERR_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// 服务就绪后的健康探测间隔。
const HEALTH_INTERVAL: Duration = Duration::from_secs(10);

/// 单次探测允许的耗时，超过即记一次未命中。
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

/// 连续未命中的容忍上限。DSH 要跑模型回合和工具调用，忙是正常的；
/// 三次未命中等于半分钟完全不应答，那不是忙。
const HEALTH_MISS_LIMIT: u32 = 3;

/// 日志面板保留的行数。
const LOG_HISTORY: usize = 2_000;

/// 日志行来自哪个管道。
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Stream {
    Stdout,
    Stderr,
}

/// supervisor 当前在做什么（前端状态卡的直接数据源）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "phase",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Status {
    Stopped,
    Starting,
    /// `origin` 只含 scheme://host:port（展示与转发用）；DSH 0.1.2 起
    /// 的进程启动 token 单独放 `token`，供 WebView 登录与网关兑换
    /// cookie——两者不混，凭据不进展示字段。旧版 DSH 无 token 时为空串。
    Ready {
        origin: String,
        token: String,
        pid: u32,
    },
    Restarting { attempt: u32, delay_ms: u64 },
    Failed { reason: String },
}

/// 前端应响应的事件。
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    Status(Status),
    Log { stream: Stream, line: String },
}

/// 启动一次 DSH 所需的全部输入。
#[derive(Clone, Debug)]
pub struct LaunchPlan {
    /// 运行 DSH 的 Node 可执行文件。
    pub node: PathBuf,
    /// DSH CLI 入口（私有 prefix 里的 bin.js）。
    pub entry: PathBuf,
    /// 启动的 profile（决定插件栈）。
    pub profile: String,
    /// DSH 工作目录（agent 会话继承）。
    pub workspace: PathBuf,
    /// 绑定地址。回环；不做成设置是有意的——绑到别处等于把一个
    /// 能执行 shell 命令的 agent 暴露给局域网。
    pub host: String,
    /// 监听端口。0 仅在用户显式开启随机 fallback 时出现。
    pub port: u16,
    /// ADR-009：Some = 注入隔离 DSH_HOME；None = 继承系统 ~/.dsh。
    pub dsh_home: Option<PathBuf>,
}

impl LaunchPlan {
    fn to_command(&self) -> Command {
        let mut command = Command::new(&self.node);
        command
            .arg(&self.entry)
            // 用命名子命令而不是 `web` 别名，且放在 profile 自身参数之前：
            // launcher 在第一个不认识的 token 处停止读取自己的旗标。
            .arg("--profile")
            .arg(&self.profile)
            .arg("--no-open")
            .arg("--host")
            .arg(&self.host)
            .arg("--port")
            .arg(self.port.to_string())
            .current_dir(&self.workspace)
            .env("DSH_DESKTOP", "1")
            .env("QIANXUN_VERSION", env!("CARGO_PKG_VERSION"));
        if let Some(home) = &self.dsh_home {
            command.env("DSH_HOME", home);
        }
        #[cfg(windows)]
        {
            // Node 在 Windows 是控制台程序，但输出归千寻的日志面板所有：
            // CREATE_NO_WINDOW 阻止每次启动闪一个控制台窗，不改管道行为。
            command.creation_flags(0x0800_0000);
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command
    }
}

/// 拥有 DSH 进程及其派生的一切。
pub struct Supervisor {
    guard: ProcessGuard,
    events: broadcast::Sender<Event>,
    status: Mutex<Status>,
    log: Mutex<VecDeque<(Stream, String)>>,
    /// 监督循环持有子进程期间为 true，使 start 幂等。
    active: AtomicBool,
    /// stop 置位，监督循环据此区分「有意退出」与「崩溃」。
    stopping: AtomicBool,
}

impl Supervisor {
    pub fn new() -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            guard: ProcessGuard::new().map_err(|cause| Error::ProcessGuard(cause.to_string()))?,
            events: broadcast::channel(512).0,
            status: Mutex::new(Status::Stopped),
            log: Mutex::new(VecDeque::with_capacity(LOG_HISTORY)),
            active: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// 监督循环之外写入一行日志（安装器等也用它汇报）。
    pub fn note(&self, stream: Stream, line: String) {
        self.record(stream, line);
    }

    pub fn status(&self) -> Status {
        self.status_guard().clone()
    }

    /// 最近的 DSH 输出，旧在前。
    pub fn recent_log(&self) -> Vec<(Stream, String)> {
        self.log_guard().iter().cloned().collect()
    }

    /// 启动 DSH 并返回它服务的 origin。
    ///
    /// 第一次尝试内联执行：配置错误的启动要报出真实错误，而不是
    /// 消失在重试循环里。证明能启动之后，监督才转入后台。
    pub async fn start(self: Arc<Self>, plan: LaunchPlan) -> Result<String> {
        if let Status::Ready { origin, .. } = self.status() {
            return Ok(origin);
        }
        if self.active.swap(true, Ordering::SeqCst) {
            return Err(Error::AlreadyStarting);
        }
        if let Err(failure) = fixed_port_available(&plan.host, plan.port) {
            self.active.store(false, Ordering::SeqCst);
            self.publish(Status::Failed {
                reason: failure.to_string(),
            });
            return Err(failure);
        }
        self.stopping.store(false, Ordering::SeqCst);
        self.publish(Status::Starting);

        match Arc::clone(&self).launch_once(&plan).await {
            Ok((child, origin, token)) => {
                let pid = child.id().unwrap_or_default();
                self.publish(Status::Ready {
                    origin: origin.clone(),
                    token: token.clone(),
                    pid,
                });
                tokio::spawn(async move { self.supervise(child, plan).await });
                Ok(origin)
            }
            Err(failure) => {
                self.active.store(false, Ordering::SeqCst);
                self.publish(Status::Failed {
                    reason: failure.to_string(),
                });
                Err(failure)
            }
        }
    }

    /// 停止 DSH 并保持停止。
    pub async fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        // ADR-011：guard 只拥有千寻自己 spawn 的进程树，
        // terminate_all 触及不到也绝不去触及别人的进程。
        let _ = self.guard.terminate_all();
        self.publish(Status::Stopped);
    }

    /// 等到监督任务观察到进程终止。安装器替换 runtime 目录前
    /// 必须等待：监督任务若还能对旧目录重启子进程，rename 会失败。
    pub async fn wait_until_inactive(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.active.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| Error::Install("运行中的 DSH 在替换运行时之前没有停下".to_owned()))
    }

    /// 跑一次启动尝试直到就绪（或失败）。
    async fn launch_once(self: Arc<Self>, plan: &LaunchPlan) -> Result<(Child, String, String)> {
        let mut command = plan.to_command();
        let mut child = self
            .guard
            .spawn(&mut command)
            .map_err(|cause| Error::Spawn(cause.to_string()))?;
        let pid = child.id();

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
            let _ = child.kill().await;
            let _ = child.wait().await;
            if let Some(pid) = pid {
                let _ = self.guard.finish(pid);
            }
            return Err(Error::Readiness("DSH 没有提供诊断管道".to_owned()));
        };
        let (ready_tx, ready_rx) = oneshot::channel();

        tokio::spawn(Arc::clone(&self).pump(stdout, Stream::Stdout, Some(ready_tx), false));
        let mut stderr_task =
            tokio::spawn(Arc::clone(&self).pump(stderr, Stream::Stderr, None, true));

        let outcome = tokio::select! {
            announced = ready_rx => match announced {
                Ok(Ready::At(url)) => match self.validate_fixed_port(&url, plan) {
                    // child 不能在 select 臂里移动：另一条臂的 child.wait()
                    // 还在借用它。这里只产出 (origin, token)，进程在下面接管。
                    Ok(()) => split_ready(&url).map_err(Error::Readiness),
                    Err(reason) => Err(Error::Readiness(reason)),
                },
                Ok(Ready::Rejected(reason)) => Err(Error::Readiness(reason)),
                // 发送端被丢弃只发生在 EOF。
                Err(_) => Err(Error::Readiness(
                    "DSH 在宣布端口之前关闭了输出".to_owned(),
                )),
            },
            exit = child.wait() => Err(Error::Readiness(match exit {
                Ok(status) => format!("DSH 在启动过程中退出（{status}）"),
                Err(cause) => format!("无法等待 DSH 进程：{cause}"),
            })),
            _ = tokio::time::sleep(READINESS_TIMEOUT) => Err(Error::Readiness(format!(
                "DSH 在 {} 秒内没有宣布端口",
                READINESS_TIMEOUT.as_secs()
            ))),
        };

        match outcome {
            Ok((origin, token)) => Ok((child, origin, token)),
            Err(failure) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                if let Some(pid) = pid {
                    if let Err(cause) = self.guard.finish(pid) {
                        self.record(
                            Stream::Stderr,
                            format!("回收失败的 DSH 进程树时出错：{cause}"),
                        );
                    }
                }
                let stderr = match tokio::time::timeout(
                    STARTUP_STDERR_DRAIN_TIMEOUT,
                    &mut stderr_task,
                )
                .await
                {
                    Ok(Ok(stderr)) => stderr,
                    _ => {
                        stderr_task.abort();
                        Vec::new()
                    }
                };
                Err(with_startup_stderr(failure, &stderr))
            }
        }
    }

    /// ADR-002：固定端口下，就绪行报告的端口必须等于配置端口。
    /// 配置为 0（显式开启的随机 fallback）时不校验。
    fn validate_fixed_port(
        &self,
        origin: &str,
        plan: &LaunchPlan,
    ) -> std::result::Result<(), String> {
        if plan.port == 0 {
            return Ok(());
        }
        match readiness::port_of(origin) {
            Some(actual) if actual == plan.port => Ok(()),
            Some(actual) => Err(format!(
                "端口错位：配置为 {}，DSH 实际服务在 {}。请检查端口占用或更换端口",
                plan.port, actual
            )),
            None => Err("就绪地址缺少端口，无法校验".to_owned()),
        }
    }

    /// 把一个管道转发进日志；stdout 同时盯就绪行。
    async fn pump<R>(
        self: Arc<Self>,
        pipe: R,
        stream: Stream,
        mut ready: Option<oneshot::Sender<Ready>>,
        capture_tail: bool,
    ) -> Vec<String>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let mut tail = VecDeque::with_capacity(STARTUP_STDERR_LINES);
        let mut lines = BufReader::new(pipe);
        let mut raw = Vec::new();
        while matches!(
            crate::child_output::next_line(&mut lines, &mut raw).await,
            Ok(true)
        ) {
            let line = String::from_utf8_lossy(&raw).trim_end().to_string();
            if ready.is_some() {
                if let Some(announcement) = readiness::parse(&line) {
                    // take 之后为 None：第二条就绪行被忽略而非视为冲突。
                    if let Some(sender) = ready.take() {
                        let _ = sender.send(announcement);
                    }
                }
            }
            if capture_tail {
                if tail.len() == STARTUP_STDERR_LINES {
                    tail.pop_front();
                }
                tail.push_back(line.clone());
            }
            self.record(stream, line);
        }
        tail.into_iter().collect()
    }

    /// 盯住就绪的 DSH：死了或哑了都拉回来。
    async fn supervise(self: Arc<Self>, first: Child, plan: LaunchPlan) {
        let mut child = first;

        loop {
            let pid = child.id();
            let exit = tokio::select! {
                exit = child.wait() => exit,
                // 哑掉的 DSH 在这里被终结（而不是直接重启），让恢复
                // 走下面唯一的退避路径。
                reason = self.watch_health() => {
                    self.record(Stream::Stderr, format!("DSH 停止应答：{reason}"));
                    let _ = child.kill().await;
                    child.wait().await
                }
            };
            if let Some(pid) = pid {
                if let Err(cause) = self.guard.finish(pid) {
                    self.record(Stream::Stderr, format!("回收 DSH 进程树时出错：{cause}"));
                }
            }
            if self.stopping.load(Ordering::SeqCst) {
                break;
            }

            self.record(
                Stream::Stderr,
                match exit {
                    Ok(status) => format!("DSH 意外退出（{status}）"),
                    Err(cause) => format!("无法等待 DSH 进程：{cause}"),
                },
            );

            match Arc::clone(&self).revive(&plan).await {
                Some(restarted) => child = restarted,
                None => break,
            }
        }

        self.active.store(false, Ordering::SeqCst);
    }

    /// 轮询服务 origin，直到它停止应答才返回。
    ///
    /// 一次未命中不是证据：探测可能输给一次 GC 或磁盘高峰。只有连续
    /// 未命中才算数——每次正常应答都清零计数，只有达到上限才唤醒调用者。
    async fn watch_health(&self) -> String {
        let mut misses = 0u32;

        loop {
            tokio::time::sleep(HEALTH_INTERVAL).await;

            // 重启与下一次就绪之间没有可探测的对象。
            let Status::Ready { origin, .. } = self.status() else {
                misses = 0;
                continue;
            };

            match health::probe(&origin, HEALTH_TIMEOUT).await {
                Ok(()) => misses = 0,
                Err(reason) => {
                    misses += 1;
                    if misses >= HEALTH_MISS_LIMIT {
                        return reason;
                    }
                    self.record(
                        Stream::Stderr,
                        format!("健康探测未命中（{misses}/{HEALTH_MISS_LIMIT}）：{reason}"),
                    );
                }
            }
        }
    }

    /// 沿退避序列尝试拉回；全部用完或用户要求停止则返回 None。
    async fn revive(self: Arc<Self>, plan: &LaunchPlan) -> Option<Child> {
        for (index, &delay_ms) in RESTART_DELAYS_MS.iter().enumerate() {
            self.publish(Status::Restarting {
                attempt: index as u32 + 1,
                delay_ms,
            });
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if self.stopping.load(Ordering::SeqCst) {
                return None;
            }

            match Arc::clone(&self).launch_once(plan).await {
                Ok((child, origin, token)) => {
                    let pid = child.id().unwrap_or_default();
                    self.publish(Status::Ready { origin, token, pid });
                    return Some(child);
                }
                Err(failure) => self.record(Stream::Stderr, format!("重启失败：{failure}")),
            }
        }

        self.publish(Status::Failed {
            reason: format!("DSH 重启 {} 次后仍未能恢复", RESTART_DELAYS_MS.len()),
        });
        None
    }

    fn publish(&self, status: Status) {
        *self.status_guard() = status.clone();
        let _ = self.events.send(Event::Status(status));
    }

    fn record(&self, stream: Stream, line: String) {
        // 外壳日志留一份（无界增长由 logging 模块的轮转约束）。
        crate::logging::log(
            match stream {
                Stream::Stdout => "info",
                Stream::Stderr => "warn",
            },
            &line,
        );
        {
            let mut log = self.log_guard();
            if log.len() == LOG_HISTORY {
                log.pop_front();
            }
            log.push_back((stream, line.clone()));
        }
        let _ = self.events.send(Event::Log { stream, line });
    }

    /// 无关的 panic 之后，状态簿记仍要可用。
    fn status_guard(&self) -> MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 内存日志只是一个有界队列，中毒也能恢复。
    fn log_guard(&self) -> MutexGuard<'_, VecDeque<(Stream, String)>> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn with_startup_stderr(failure: Error, stderr: &[String]) -> Error {
    if stderr.is_empty() {
        return failure;
    }
    Error::Readiness(format!("{failure}\n{}", stderr.join("\n")))
}

/// 拆分就绪 URL：对外 origin（展示/转发）+ 启动 token（WebView/网关用）。
fn split_ready(url: &str) -> std::result::Result<(String, String), String> {
    let origin = readiness::origin_of(url)
        .ok_or_else(|| "就绪地址无法解析 origin".to_owned())?;
    let token = readiness::token_of(url).unwrap_or_default();
    Ok((origin, token))
}

/// 固定端口预检：在 Node 启动之前就拒绝被占用的端口，
/// 错误信息直接指向设置页，而不是让用户读 EADDRINUSE。
fn fixed_port_available(host: &str, port: u16) -> Result<()> {
    if port == 0 {
        return Ok(());
    }
    std::net::TcpListener::bind((host, port))
        .map(drop)
        .map_err(|cause| {
            Error::Readiness(format!(
                "固定端口 {host}:{port} 被占用：{cause}。请在设置里更换端口；\
                 保留端口 10000 常被其他 DSH 实例使用，请避开"
            ))
        })
}

impl Drop for Supervisor {
    /// 阻止监督循环复活一个外壳已不再拥有的 DSH。
    /// 进程树本身的回收是 guard 的职责——无论这里跑不跑得到。
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{self, AssertUnwindSafe};

    use super::*;

    #[test]
    fn 中毒后的状态簿记仍可读() {
        let supervisor = Supervisor::new().expect("process guard");
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _held = supervisor.status.lock().expect("initial lock");
            panic!("poison status bookkeeping");
        }));

        assert_eq!(supervisor.status(), Status::Stopped);
        supervisor.publish(Status::Starting);
        assert_eq!(supervisor.status(), Status::Starting);
    }

    #[test]
    fn 中毒后的内存日志仍接受诊断() {
        let supervisor = Supervisor::new().expect("process guard");
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _held = supervisor.log.lock().expect("initial lock");
            panic!("poison in-memory log bookkeeping");
        }));

        supervisor.note(Stream::Stderr, "recoverable diagnostic".into());
        assert!(supervisor
            .recent_log()
            .iter()
            .any(|(_, line)| line == "recoverable diagnostic"));
    }

    #[test]
    fn 随机端口不做预检绑定() {
        assert!(fixed_port_available("not-a-host", 0).is_ok());
    }

    #[test]
    fn 被占用的固定端口在node启动前被拒绝() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let failure = fixed_port_available("127.0.0.1", port).unwrap_err();
        assert!(failure.to_string().contains(&port.to_string()));
        assert!(failure.to_string().contains("设置"));
    }

    #[test]
    fn 端口错位被固定端口校验拒绝() {
        let supervisor = Supervisor::new().expect("process guard");
        let plan = LaunchPlan {
            node: PathBuf::from("node"),
            entry: PathBuf::from("bin.js"),
            profile: "web".into(),
            workspace: PathBuf::from("."),
            host: "127.0.0.1".into(),
            port: 17300,
            dsh_home: None,
        };
        assert!(supervisor
            .validate_fixed_port("http://127.0.0.1:17300", &plan)
            .is_ok());
        assert!(supervisor
            .validate_fixed_port("http://127.0.0.1:9999", &plan)
            .unwrap_err()
            .contains("错位"));
        let random = LaunchPlan { port: 0, ..plan };
        assert!(supervisor
            .validate_fixed_port("http://127.0.0.1:42424", &random)
            .is_ok());
    }

    #[test]
    fn 启动命令注入环境与参数次序() {
        let dsh_home = PathBuf::from("data").join("dsh-home");
        let plan = LaunchPlan {
            node: PathBuf::from("node"),
            entry: PathBuf::from("dsh/bin.js"),
            profile: "web".into(),
            workspace: PathBuf::from("workspace"),
            host: "127.0.0.1".into(),
            port: 17300,
            dsh_home: Some(dsh_home.clone()),
        };
        let command = plan.to_command();
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            [
                "dsh/bin.js",
                "--profile",
                "web",
                "--no-open",
                "--host",
                "127.0.0.1",
                "--port",
                "17300",
            ]
        );

        let environment = command
            .as_std()
            .get_envs()
            .filter_map(|(name, value)| {
                value.map(|value| {
                    (
                        name.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment.get("DSH_DESKTOP").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            environment.get("DSH_HOME").map(String::as_str),
            Some(dsh_home.to_string_lossy().as_ref())
        );
        assert_eq!(
            environment.get("QIANXUN_VERSION").map(String::as_str),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}

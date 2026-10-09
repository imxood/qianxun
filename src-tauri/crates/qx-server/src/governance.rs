//! 资源治理（docs/19 §5，v4 定案）：流量分级准入 + 资源三轴硬闸 + 压力按序降级。
//!
//! 三类流量（自动识别）：
//! - **interactive**：WS 升级 / SSE——硬上限 8/实例、16 全局，降级序中**永不主动断**；
//! - **page**：普通请求——令牌桶 40 次/s 突发 40（整页加载一次通过），等令牌 ≤1s 后 503；
//! - **bulk**：请求体 >256KiB 或单流累计 >4MiB——并发 2/实例 + 字节慢桶 512KiB/s，最先被 shed。
//!
//! 带宽轴：每实例 2MiB/s 持续（突发 8MiB）、全局 4MiB/s；全局桶饥饿超阈值进入压力态，
//! 按 bulk → page 顺序 shed（interactive 永不）。内存/CPU 轴由 systemd cgroup 承担（§5.2）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 流量分类。请求期识别见 `Class::of_request`；单流累计字节的动态降格见 `Guard::add_bytes`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Interactive,
    Page,
    Bulk,
}

impl Class {
    pub fn of_request(upgrade: bool, body_len: Option<u64>) -> Self {
        if upgrade {
            Class::Interactive
        } else if body_len.is_some_and(|len| len > 256 * 1024) {
            Class::Bulk
        } else {
            Class::Page
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Class::Interactive => "interactive",
            Class::Page => "page",
            Class::Bulk => "bulk",
        }
    }
}

/// 令牌桶（单调时钟驱动，纳秒精度）。
#[derive(Debug, Clone)]
struct Bucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl Bucket {
    fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec,
            last: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
            self.last = now;
        }
    }

    /// 消费 `n`；返回按当前补充速率还差多久（None = 成功）。
    fn take(&mut self, n: f64) -> Option<Duration> {
        self.refill();
        if self.tokens >= n {
            self.tokens -= n;
            return None;
        }
        let missing = n - self.tokens;
        Some(Duration::from_secs_f64(
            missing / self.refill_per_sec.max(1e-9),
        ))
    }

    fn level(&mut self) -> f64 {
        self.refill();
        self.tokens
    }
}

/// 治理参数（默认即生产值，docs/19 §5.1；测试可注入小值）。
#[derive(Clone, Debug)]
pub struct GovConfig {
    pub max_interactive_per_instance: usize,
    pub max_interactive_global: usize,
    pub page_rate_per_sec: f64,
    pub page_burst: f64,
    pub page_wait: Duration,
    pub max_page_inflight: usize,
    pub bulk_concurrent: usize,
    /// bulk 慢桶：字节/秒 + 突发字节。
    pub bulk_bytes_per_sec: f64,
    pub bulk_burst_bytes: f64,
    pub bw_instance_per_sec: f64,
    pub bw_instance_burst: f64,
    pub bw_global_per_sec: f64,
    pub bw_global_burst: f64,
    /// 单流累计字节降格阈值。
    pub bulk_by_bytes: u64,
    /// 压力判定：全局带宽桶连续饥饿时长。
    pub starve_threshold: Duration,
}

impl Default for GovConfig {
    fn default() -> Self {
        Self {
            max_interactive_per_instance: 8,
            max_interactive_global: 16,
            page_rate_per_sec: 40.0,
            page_burst: 40.0,
            page_wait: Duration::from_secs(1),
            max_page_inflight: 64,
            bulk_concurrent: 2,
            bulk_bytes_per_sec: 512.0 * 1024.0,
            bulk_burst_bytes: 4.0 * 1024.0 * 1024.0,
            bw_instance_per_sec: 2.0 * 1024.0 * 1024.0,
            bw_instance_burst: 8.0 * 1024.0 * 1024.0,
            bw_global_per_sec: 4.0 * 1024.0 * 1024.0,
            bw_global_burst: 4.0 * 1024.0 * 1024.0,
            bulk_by_bytes: 4 * 1024 * 1024,
            starve_threshold: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Default)]
struct InstanceGov {
    interactive: usize,
    page_inflight: usize,
    bulk: usize,
    page_bucket: Option<Bucket>,
    bulk_bucket: Option<Bucket>,
    bw_bucket: Option<Bucket>,
}

/// 拒绝原因（web 层映射到 HTTP）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// 503 + Retry-After。
    Busy { retry_after_secs: u32 },
    /// 503（降级序主动让路）。
    Shed,
}

#[derive(Debug)]
struct Inner {
    cfg: GovConfig,
    instances: HashMap<String, InstanceGov>,
    global_interactive: usize,
    global_bw: Bucket,
    starve_since: Option<Instant>,
    manual_pressure: bool,
    /// (实例, 类) → [admitted, rejected, shed, bytes]（/metrics 用，无设备维度）。
    counters: HashMap<(String, Class), [AtomicU64; 4]>,
    /// (设备, 实例, 类) → [bytes, rejected, shed]：待落库的小时桶。
    ///
    /// 刻意停在内存里（docs/20 §4.3）：SQLite 只有一把互斥连接，反代每个请求
    /// 都写一行会把单连接变成瓶颈。落库节奏由 web 层 ~60s 拉一次。
    pending: HashMap<(String, String, Class), [u64; 3]>,
    /// 在途会话（设备页展开的「当前会话」）。
    sessions: HashMap<u64, Session>,
    next_slot: u64,
}

/// 一条在途会话的只读快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionView {
    /// 设备 id；未知来源（实例侧自注册）时为 None。
    pub device: Option<String>,
    pub instance: String,
    pub class: Class,
    pub seconds: u64,
    pub bytes: u64,
}

/// 在途会话的内部状态（`SessionView` 的可写版）。
#[derive(Debug)]
struct Session {
    device: Option<String>,
    instance: String,
    class: Class,
    started: Instant,
    bytes: u64,
}

/// 单实例的实时水位（控制台概览「水位」四条）。
#[derive(Clone, Debug, PartialEq)]
pub struct LevelView {
    pub instance: String,
    pub interactive: usize,
    pub page: usize,
    pub bulk: usize,
    /// 带宽桶余量 0..1。
    pub bw: f64,
    /// 页面令牌桶余量 0..1。
    pub page_tokens: f64,
}

/// 待落库的一格计量（按落库时刻归小时桶）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingMetric {
    pub device: Option<String>,
    pub instance: String,
    pub class: Class,
    pub bytes: u64,
    pub rejected: u64,
    pub shed: u64,
}

/// 硬闸上限（控制台水位条的分母）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub interactive: usize,
    pub page: usize,
    pub bulk: usize,
}

#[derive(Debug)]
pub struct Governance {
    inner: Mutex<Inner>,
}

/// 一次已放行流量的记账句柄；Drop 自动归还配额，禁止克隆（配额语义唯一）。
#[derive(Debug)]
pub struct Guard {
    gov: Arc<Governance>,
    instance: String,
    /// 归因到设备（docs/20 §4.2）。实例自注册无设备，归 None（库内记 `-`）。
    device: Option<String>,
    class: Class,
    bytes: u64,
    slot: u64,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut guard = self.gov.inner.lock().expect("治理锁");
        let inner = &mut *guard;
        inner.sessions.remove(&self.slot);
        // 先经实例表归还，再动全局计数——两段借用不交叉。
        let global_release = if let Some(state) = inner.instances.get_mut(&self.instance) {
            match self.class {
                Class::Interactive => {
                    state.interactive = state.interactive.saturating_sub(1);
                    true
                }
                Class::Page => {
                    state.page_inflight = state.page_inflight.saturating_sub(1);
                    false
                }
                Class::Bulk => {
                    state.bulk = state.bulk.saturating_sub(1);
                    false
                }
            }
        } else {
            false
        };
        if global_release {
            inner.global_interactive = inner.global_interactive.saturating_sub(1);
        }
    }
}

impl Guard {
    pub fn class(&self) -> Class {
        self.class
    }

    /// 归因设备（供控制台归因；代理路径在鉴权后才知道设备）。
    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    /// 响应头识别 SSE → 升格 interactive（docs/19 §5.1）。
    pub fn promote_interactive(&mut self) {
        if self.class == Class::Interactive {
            return;
        }
        let mut inner = self.gov.inner.lock().expect("治理锁");
        bump(&mut inner, &self.instance, Class::Interactive, 0);
        let state = inner.instances.entry(self.instance.clone()).or_default();
        match self.class {
            Class::Page => state.page_inflight = state.page_inflight.saturating_sub(1),
            Class::Bulk => state.bulk = state.bulk.saturating_sub(1),
            Class::Interactive => {}
        }
        state.interactive += 1;
        inner.global_interactive += 1;
        // 换类时把在途会话的类改掉——设备页看到的必须是当前占用的档位。
        if let Some(session) = inner.sessions.get_mut(&self.slot) {
            session.class = Class::Interactive;
        }
        self.class = Class::Interactive;
    }

    /// 流经字节记账：带宽轴（实例/全局）+ bulk 慢桶 + 累计降格。
    /// 返回建议的整形延迟（None = 立即放行；web 层的流泵按此 sleep）。
    pub fn add_bytes(&mut self, chunk: u64) -> Option<Duration> {
        let bytes = chunk as f64;
        let mut delay: Option<Duration> = None;
        let threshold;
        {
            let mut guard = self.gov.inner.lock().expect("治理锁");
            let inner = &mut *guard;
            threshold = inner.cfg.bulk_by_bytes;
            let cfg = inner.cfg.clone();
            let state = inner.instances.entry(self.instance.clone()).or_default();
            ensure_buckets(state, &cfg);
            // 带宽轴：实例桶 + 全局桶（取更长的延迟）。
            delay = max_delay(delay, state.bw_bucket.as_mut().expect("桶已建").take(bytes));
            delay = max_delay(delay, inner.global_bw.take(bytes));
            if inner.global_bw.level() <= 0.0 {
                if inner.starve_since.is_none() {
                    inner.starve_since = Some(Instant::now());
                }
            } else {
                inner.starve_since = None;
            }
            // bulk 慢桶：只对本类生效（docs/19 §5.1）。
            if self.class == Class::Bulk {
                delay = max_delay(
                    delay,
                    state.bulk_bucket.as_mut().expect("桶已建").take(bytes),
                );
            }
            if let Some(counter) = inner.counters.get_mut(&(self.instance.clone(), self.class)) {
                counter[3].fetch_add(chunk, Ordering::Relaxed);
            }
            // 计量归因到设备（单锁内一起做，热路径不额外加锁）。
            let key = (
                self.device.clone().unwrap_or_else(|| "-".to_owned()),
                self.instance.clone(),
                self.class,
            );
            let slot = inner.pending.entry(key).or_insert([0, 0, 0]);
            slot[0] = slot[0].saturating_add(chunk);
            if let Some(session) = inner.sessions.get_mut(&self.slot) {
                session.bytes = self.bytes.saturating_add(chunk);
            }
        }
        self.bytes = self.bytes.saturating_add(chunk);
        if self.bytes > threshold && self.class != Class::Bulk {
            self.demote_bulk();
        }
        delay
    }

    /// 单流累计超阈值 → 降格 bulk。
    pub fn demote_bulk(&mut self) {
        if self.class == Class::Bulk {
            return;
        }
        let mut guard = self.gov.inner.lock().expect("治理锁");
        bump(&mut guard, &self.instance, Class::Bulk, 0);
        let inner = &mut *guard;
        let global_release = if let Some(state) = inner.instances.get_mut(&self.instance) {
            match self.class {
                Class::Page => {
                    state.page_inflight = state.page_inflight.saturating_sub(1);
                    false
                }
                Class::Interactive => {
                    state.interactive = state.interactive.saturating_sub(1);
                    true
                }
                Class::Bulk => false,
            }
        } else {
            false
        };
        if global_release {
            inner.global_interactive = inner.global_interactive.saturating_sub(1);
        }
        if let Some(state) = inner.instances.get_mut(&self.instance) {
            state.bulk += 1;
        }
        if let Some(session) = inner.sessions.get_mut(&self.slot) {
            session.class = Class::Bulk;
        }
        self.class = Class::Bulk;
    }
}

impl Governance {
    pub fn new(cfg: GovConfig) -> Arc<Self> {
        let global = Bucket::new(cfg.bw_global_burst, cfg.bw_global_per_sec);
        Arc::new(Self {
            inner: Mutex::new(Inner {
                cfg,
                instances: HashMap::new(),
                global_interactive: 0,
                global_bw: global,
                starve_since: None,
                manual_pressure: false,
                counters: HashMap::new(),
                pending: HashMap::new(),
                sessions: HashMap::new(),
                next_slot: 1,
            }),
        })
    }

    /// 登记一条在途会话，返回它的槽位号（Guard 靠它归还/改档）。
    fn open_session(inner: &mut Inner, device: Option<&str>, instance: &str, class: Class) -> u64 {
        let slot = inner.next_slot;
        inner.next_slot = inner.next_slot.wrapping_add(1).max(1);
        inner.sessions.insert(
            slot,
            Session {
                device: device.map(str::to_owned),
                instance: instance.to_owned(),
                class,
                started: Instant::now(),
                bytes: 0,
            },
        );
        slot
    }

    /// 主入口：放行或拒绝（docs/19 §5.1 准入语义）。
    ///
    /// `device` 是归因维度（docs/20 §4.2）：准入判定仍是纯实例口径，但计量与
    /// 限流计数要落到具体设备，否则设备页的「24h 流量/占配额/当前会话」都是编的。
    pub async fn admit(
        self: &Arc<Self>,
        instance: &str,
        device: Option<&str>,
        class: Class,
    ) -> Result<Guard, Reject> {
        match class {
            Class::Page => self.admit_page(instance, device).await,
            Class::Interactive => {
                self.precheck(instance, device, class)?;
                let mut inner = self.inner.lock().expect("治理锁");
                let state = inner.instances.entry(instance.to_owned()).or_default();
                state.interactive += 1;
                inner.global_interactive += 1;
                bump(&mut inner, instance, class, 0);
                let slot = Self::open_session(&mut inner, device, instance, class);
                Ok(Guard {
                    gov: self.clone(),
                    instance: instance.to_owned(),
                    device: device.map(str::to_owned),
                    class,
                    bytes: 0,
                    slot,
                })
            }
            Class::Bulk => {
                self.precheck(instance, device, class)?;
                let mut inner = self.inner.lock().expect("治理锁");
                let cfg = inner.cfg.clone();
                let state = inner.instances.entry(instance.to_owned()).or_default();
                ensure_buckets(state, &cfg);
                state.bulk += 1;
                bump(&mut inner, instance, class, 0);
                let slot = Self::open_session(&mut inner, device, instance, class);
                Ok(Guard {
                    gov: self.clone(),
                    instance: instance.to_owned(),
                    device: device.map(str::to_owned),
                    class,
                    bytes: 0,
                    slot,
                })
            }
        }
    }

    fn precheck(&self, instance: &str, device: Option<&str>, class: Class) -> Result<(), Reject> {
        let mut inner = self.inner.lock().expect("治理锁");
        let pressure = inner.manual_pressure || {
            if let Some(since) = inner.starve_since {
                since.elapsed() >= inner.cfg.starve_threshold
            } else {
                false
            }
        };
        if pressure && matches!(class, Class::Bulk | Class::Page) {
            bump(&mut inner, instance, class, 2);
            bump_device(&mut inner, device, instance, class, 2);
            return Err(Reject::Shed);
        }
        let cfg = inner.cfg.clone();
        let state = inner.instances.entry(instance.to_owned()).or_default();
        ensure_buckets(state, &cfg);
        let reject = match class {
            Class::Bulk if state.bulk >= cfg.bulk_concurrent => Some(Reject::Busy {
                retry_after_secs: 2,
            }),
            Class::Page if state.page_inflight >= cfg.max_page_inflight => Some(Reject::Busy {
                retry_after_secs: 1,
            }),
            Class::Interactive
                if state.interactive >= cfg.max_interactive_per_instance
                    || inner.global_interactive >= cfg.max_interactive_global =>
            {
                Some(Reject::Busy {
                    retry_after_secs: 2,
                })
            }
            _ => None,
        };
        match reject {
            Some(reject) => {
                bump(&mut inner, instance, class, 1);
                bump_device(&mut inner, device, instance, class, 1);
                Err(reject)
            }
            None => Ok(()),
        }
    }

    async fn admit_page(
        self: &Arc<Self>,
        instance: &str,
        device: Option<&str>,
    ) -> Result<Guard, Reject> {
        self.precheck(instance, device, Class::Page)?;
        let (wait, refill_step) = {
            let inner = self.inner.lock().expect("治理锁");
            (
                inner.cfg.page_wait,
                inner.cfg.page_rate_per_sec.recip().max(0.005),
            )
        };
        let deadline = Instant::now() + wait;
        loop {
            // 先查截止再取令牌：sleep 醒来若已超时，不得消费新充的令牌。
            if Instant::now() >= deadline {
                let mut inner = self.inner.lock().expect("治理锁");
                bump(&mut inner, instance, Class::Page, 1);
                bump_device(&mut inner, device, instance, Class::Page, 1);
                return Err(Reject::Busy {
                    retry_after_secs: 1,
                });
            }
            let admitted = {
                let mut inner = self.inner.lock().expect("治理锁");
                let cfg = inner.cfg.clone();
                let state = inner.instances.entry(instance.to_owned()).or_default();
                ensure_buckets(state, &cfg);
                if state
                    .page_bucket
                    .as_mut()
                    .expect("桶已建")
                    .take(1.0)
                    .is_none()
                {
                    state.page_inflight += 1;
                    true
                } else {
                    false
                }
            };
            if admitted {
                let mut inner = self.inner.lock().expect("治理锁");
                bump(&mut inner, instance, Class::Page, 0);
                let slot = Self::open_session(&mut inner, device, instance, Class::Page);
                drop(inner);
                return Ok(Guard {
                    gov: self.clone(),
                    instance: instance.to_owned(),
                    device: device.map(str::to_owned),
                    class: Class::Page,
                    bytes: 0,
                    slot,
                });
            }
            tokio::time::sleep(Duration::from_secs_f64(refill_step)).await;
        }
    }

    /// 测试/运维：手动压力位（模拟全局带宽饥饿）。
    pub fn set_pressure(&self, on: bool) {
        self.inner.lock().expect("治理锁").manual_pressure = on;
    }

    /// 指标快照（/metrics）：(实例, 类) → (admitted, rejected, shed, bytes)。
    pub fn snapshot_counters(&self) -> Vec<(String, Class, u64, u64, u64, u64)> {
        let inner = self.inner.lock().expect("治理锁");
        let mut rows: Vec<_> = inner
            .counters
            .iter()
            .map(|((instance, class), counter)| {
                (
                    instance.clone(),
                    *class,
                    counter[0].load(Ordering::Relaxed),
                    counter[1].load(Ordering::Relaxed),
                    counter[2].load(Ordering::Relaxed),
                    counter[3].load(Ordering::Relaxed),
                )
            })
            .collect();
        rows.sort_by(|a, b| (&a.0, a.1.as_str()).cmp(&(&b.0, b.1.as_str())));
        rows
    }

    /// 硬闸上限（控制台「水位」的分母）。直接读配置，不另存一份——
    /// 两份数字迟早会对不上，而水位条分母错位是最难被发现的那种错。
    pub fn caps(&self) -> Caps {
        let inner = self.inner.lock().expect("治理锁");
        Caps {
            interactive: inner.cfg.max_interactive_global,
            page: inner.cfg.max_page_inflight,
            bulk: inner.cfg.bulk_concurrent,
        }
    }

    /// 实时水位（控制台概览「水位」四条）：交互会话数 / 页面在途 / 批量在途 / 带宽余量。
    pub fn snapshot_levels(&self) -> Vec<LevelView> {
        let mut inner = self.inner.lock().expect("治理锁");
        let cfg = inner.cfg.clone();
        let mut rows: Vec<LevelView> = inner
            .instances
            .iter_mut()
            .map(|(instance, state)| {
                let bw = state
                    .bw_bucket
                    .as_mut()
                    .map(|bucket| {
                        let level = bucket.level();
                        if cfg.bw_instance_burst > 0.0 {
                            level / cfg.bw_instance_burst
                        } else {
                            0.0
                        }
                    })
                    .unwrap_or(1.0)
                    .clamp(0.0, 1.0);
                let page_tokens = state
                    .page_bucket
                    .as_mut()
                    .map(|bucket| {
                        let level = bucket.level();
                        if cfg.page_burst > 0.0 {
                            level / cfg.page_burst
                        } else {
                            0.0
                        }
                    })
                    .unwrap_or(1.0)
                    .clamp(0.0, 1.0);
                LevelView {
                    instance: instance.clone(),
                    interactive: state.interactive,
                    page: state.page_inflight,
                    bulk: state.bulk,
                    bw,
                    page_tokens,
                }
            })
            .collect();
        rows.sort_by(|a, b| a.instance.cmp(&b.instance));
        rows
    }

    /// 在途会话快照（设备页展开的「当前会话」；按设备分组由调用方做）。
    pub fn live_sessions(&self) -> Vec<SessionView> {
        let inner = self.inner.lock().expect("治理锁");
        let mut rows: Vec<SessionView> = inner
            .sessions
            .values()
            .map(|session| SessionView {
                device: session.device.clone(),
                instance: session.instance.clone(),
                class: session.class,
                seconds: session.started.elapsed().as_secs(),
                bytes: session.bytes,
            })
            .collect();
        rows.sort_by(|a, b| {
            (&a.device, &a.instance, a.class.as_str()).cmp(&(
                &b.device,
                &b.instance,
                b.class.as_str(),
            ))
        });
        rows
    }

    /// 取出并清空待落库的计量（web 层 ~60s 拉一次）。
    pub fn drain_pending(&self) -> Vec<PendingMetric> {
        let mut inner = self.inner.lock().expect("治理锁");
        let drained = inner
            .pending
            .drain()
            .map(|((device, instance, class), values)| PendingMetric {
                device: if device == "-" { None } else { Some(device) },
                instance,
                class,
                bytes: values[0],
                rejected: values[1],
                shed: values[2],
            })
            .collect();
        drained
    }
}

/// 设备维度的计量累加（[bytes, rejected, shed]）。未知设备归到 `-`，
/// 这样「无来源流量」仍在库里可见，而不是悄悄丢掉。
fn bump_device(inner: &mut Inner, device: Option<&str>, instance: &str, class: Class, slot: usize) {
    let key = (device.unwrap_or("-").to_owned(), instance.to_owned(), class);
    inner.pending.entry(key).or_insert([0, 0, 0])[slot] += 1;
}

fn ensure_buckets(state: &mut InstanceGov, cfg: &GovConfig) {
    if state.page_bucket.is_none() {
        state.page_bucket = Some(Bucket::new(cfg.page_burst, cfg.page_rate_per_sec));
    }
    if state.bulk_bucket.is_none() {
        state.bulk_bucket = Some(Bucket::new(cfg.bulk_burst_bytes, cfg.bulk_bytes_per_sec));
    }
    if state.bw_bucket.is_none() {
        state.bw_bucket = Some(Bucket::new(cfg.bw_instance_burst, cfg.bw_instance_per_sec));
    }
}

/// 两个可选延迟取更长（整形建议取最紧的桶）。
fn max_delay(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (some, None) | (None, some) => some,
    }
}

fn bump(inner: &mut Inner, instance: &str, class: Class, slot: usize) {
    let counter = inner
        .counters
        .entry((instance.to_owned(), class))
        .or_insert_with(|| [0, 0, 0, 0].map(AtomicU64::new));
    counter[slot].fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gov_small_page() -> Arc<Governance> {
        let cfg = GovConfig {
            page_rate_per_sec: 1.0,
            page_burst: 2.0,
            page_wait: Duration::from_millis(50),
            ..GovConfig::default()
        };
        Governance::new(cfg)
    }

    #[tokio::test]
    async fn page突发全过后限流() {
        let gov = gov_small_page();
        let first = gov
            .admit("i1", None, Class::Page)
            .await
            .expect("突发额度内放行");
        let second = gov
            .admit("i1", None, Class::Page)
            .await
            .expect("突发额度内放行");
        let third = gov.admit("i1", None, Class::Page).await;
        assert_eq!(
            third.unwrap_err(),
            Reject::Busy {
                retry_after_secs: 1
            },
            "桶空且等待超时"
        );
        drop((first, second));
    }

    #[tokio::test]
    async fn page突发40整页通过() {
        let gov = Governance::new(GovConfig::default());
        let mut guards = Vec::new();
        for _ in 0..40 {
            guards.push(
                gov.admit("i1", None, Class::Page)
                    .await
                    .expect("整页加载一次通过"),
            );
        }
    }

    #[tokio::test]
    async fn bulk并发上限() {
        let gov = Governance::new(GovConfig::default());
        let _a = gov.admit("i1", None, Class::Bulk).await.expect("第 1 条");
        let _b = gov.admit("i1", None, Class::Bulk).await.expect("第 2 条");
        let c = gov.admit("i1", None, Class::Bulk).await;
        assert_eq!(
            c.unwrap_err(),
            Reject::Busy {
                retry_after_secs: 2
            }
        );
    }

    #[tokio::test]
    async fn interactive硬上限且压力下不shed() {
        let gov = Governance::new(GovConfig::default());
        let mut guards = Vec::new();
        for _ in 0..8 {
            guards.push(
                gov.admit("i1", None, Class::Interactive)
                    .await
                    .expect("8 条内放行"),
            );
        }
        assert!(
            gov.admit("i1", None, Class::Interactive).await.is_err(),
            "第 9 条拒绝"
        );
        gov.set_pressure(true);
        assert!(
            gov.admit("i2", None, Class::Interactive).await.is_ok(),
            "压力下 interactive 仍放行"
        );
    }

    #[tokio::test]
    async fn 压力降级序bulk先死() {
        let gov = Governance::new(GovConfig::default());
        gov.set_pressure(true);
        assert_eq!(
            gov.admit("i1", None, Class::Bulk).await.unwrap_err(),
            Reject::Shed
        );
        assert_eq!(
            gov.admit("i1", None, Class::Page).await.unwrap_err(),
            Reject::Shed
        );
        gov.set_pressure(false);
        assert!(
            gov.admit("i1", None, Class::Page).await.is_ok(),
            "压力解除自动恢复"
        );
    }

    #[tokio::test]
    async fn guard升降格与字节降格() {
        let gov = Governance::new(GovConfig::default());
        let mut guard = gov.admit("i1", None, Class::Page).await.expect("page 起步");
        guard.promote_interactive();
        assert_eq!(guard.class(), Class::Interactive);
        guard.demote_bulk();
        assert_eq!(guard.class(), Class::Bulk);

        let mut guard = gov.admit("i2", None, Class::Page).await.expect("page 起步");
        guard.add_bytes(5 * 1024 * 1024);
        assert_eq!(guard.class(), Class::Bulk, "单流累计超 4MiB 降格");
    }

    #[tokio::test]
    async fn bulk慢桶返回整形延迟() {
        let cfg = GovConfig {
            bulk_burst_bytes: 1024.0,
            bulk_bytes_per_sec: 1024.0,
            ..GovConfig::default()
        };
        let gov = Governance::new(cfg);
        let mut guard = gov.admit("i1", None, Class::Bulk).await.expect("放行");
        assert_eq!(guard.add_bytes(512), None, "突发内立即放行");
        let delay = guard.add_bytes(2 * 1024);
        assert!(delay.is_some(), "超出突发 → 建议延迟整形");
    }

    #[test]
    fn 请求分类() {
        assert_eq!(Class::of_request(true, None), Class::Interactive);
        assert_eq!(Class::of_request(false, Some(300 * 1024)), Class::Bulk);
        assert_eq!(Class::of_request(false, None), Class::Page);
    }

    /// 设备归因：设备页的 24h 流量 / 占配额 / 当前会话全部靠这条链。
    #[tokio::test]
    async fn 计量归因到设备且随流结束退出() {
        let gov = Governance::new(GovConfig::default());
        {
            let mut guard = gov
                .admit("i1", Some("dev-a"), Class::Page)
                .await
                .expect("放行");
            guard.add_bytes(4096);
            // 在途会话：设备页展开时看到的就是它。
            let sessions = gov.live_sessions();
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].device.as_deref(), Some("dev-a"));
            assert_eq!(sessions[0].instance, "i1");
            assert_eq!(sessions[0].bytes, 4096);
            // 另一台设备的流量不能记在这台头上。
            assert_eq!(gov.drain_pending().len(), 1);
        }
        assert!(gov.live_sessions().is_empty(), "Guard 释放后会话消失");
    }

    #[tokio::test]
    async fn 限流也归因到设备() {
        let gov = Governance::new(GovConfig {
            max_page_inflight: 1,
            ..GovConfig::default()
        });
        let _first = gov
            .admit("i1", Some("dev-a"), Class::Page)
            .await
            .expect("放行");
        assert!(gov.admit("i1", Some("dev-b"), Class::Page).await.is_err());
        let pending = gov.drain_pending();
        let dev_b: Vec<_> = pending
            .iter()
            .filter(|item| item.device.as_deref() == Some("dev-b"))
            .collect();
        assert_eq!(dev_b.len(), 1);
        assert_eq!(dev_b[0].rejected, 1);
        assert!(dev_b[0].bytes == 0, "被拒的流不产生字节");
    }

    #[tokio::test]
    async fn 未知来源的流量仍可见() {
        let gov = Governance::new(GovConfig::default());
        let mut guard = gov.admit("i1", None, Class::Page).await.expect("放行");
        guard.add_bytes(128);
        let pending = gov.drain_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].device, None, "无设备归因记 None，落库时写 `-`");
        assert_eq!(pending[0].bytes, 128);
    }

    #[tokio::test]
    async fn 水位与上限来自同一份配置() {
        let gov = Governance::new(GovConfig {
            max_page_inflight: 7,
            ..GovConfig::default()
        });
        let _page = gov
            .admit("i1", Some("dev-a"), Class::Page)
            .await
            .expect("放行");
        let levels = gov.snapshot_levels();
        assert_eq!(levels[0].page, 1);
        assert_eq!(gov.caps().page, 7, "水位分母 = 配置，不另存一份");
    }
}

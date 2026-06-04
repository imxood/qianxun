// ─── 自动重连 (Stage 5) ──────────────────────────────────────

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tracing::debug;

use super::daemon_client::DaemonClient;

/// 退避表: 第 1..4 次重试间隔, 第 5+ 次都用最后一个 (30s 上限).
pub const RECONNECT_BACKOFF: &[Duration] = &[
    Duration::from_secs(3),
    Duration::from_secs(6),
    Duration::from_secs(12),
    Duration::from_secs(30),
];

/// 给定当前 attempt (1 = 第 1 次失败), 返回下次重试的等待时间.
///
/// - attempt=1 → 3s (1st failure, BACKOFF[0])
/// - attempt=2 → 6s
/// - attempt=3 → 12s
/// - attempt=4+ → 30s (cap)
pub fn next_backoff(attempt: u32) -> Duration {
    let idx = (attempt as usize)
        .saturating_sub(1)
        .min(RECONNECT_BACKOFF.len() - 1);
    RECONNECT_BACKOFF[idx]
}

/// 客户端到 daemon 的连接状态 (供 UI/上层订阅).
#[derive(Debug, Clone, PartialEq)]
pub enum ReconnectState {
    /// 最近一次 health() 成功.
    Connected,
    /// health() 失败, 正在退避等待下次重试.
    Reconnecting {
        attempt: u32,
        next_retry_in: Duration,
    },
    /// 重连耗尽或取消, 进入离线状态.
    Offline { last_error: String },
}

impl ReconnectState {
    /// 人类可读摘要 (供 TUI 状态栏打印).
    pub fn label(&self) -> String {
        match self {
            ReconnectState::Connected => "connected".to_string(),
            ReconnectState::Reconnecting { attempt, next_retry_in } => {
                format!("reconnecting (attempt={}, next in {}s)", attempt, next_retry_in.as_secs())
            }
            ReconnectState::Offline { last_error } => {
                format!("offline: {last_error}")
            }
        }
    }
}

/// 内部共享状态: 当前 attempt 计数 + 最近一次错误.
#[derive(Debug, Default)]
pub struct ReconnectTracker {
    /// 连续失败次数. 成功 health() 时清零.
    attempt: u32,
    /// 最近一次错误信息 (供 Offline 状态用).
    last_error: Option<String>,
    /// 当前是否处于 "Reconnecting" 状态 (避免重复触发回调).
    in_reconnect: bool,
}

impl ReconnectTracker {
    fn new() -> Self {
        Self {
            attempt: 0,
            last_error: None,
            in_reconnect: false,
        }
    }
}

impl DaemonClient {
    /// 启动后台自动重连循环. `on_state` 在状态变化时被调用 (Connected / Reconnecting / Offline).
    ///
    /// 行为:
    /// - 后台 task 每 1s 跑一次 `health()` (轻量探测, 3s 超时)
    /// - 成功: 累计 attempt 清零, 触发 `Connected` 回调
    /// - 失败: attempt++, 计算 next_backoff, 触发 `Reconnecting` 回调
    ///   (注意: 实际等待是 **增量累加** 的 — 每次 loop tick 重新判断, 避免
    ///   调度器抖动导致 backoff 漂移)
    /// - 等待时长达到 next_backoff 后, 下一次 health() 失败时 attempt 再 ++
    /// - 在 4 次失败后, next_backoff 保持 30s, 状态稳定在 Reconnecting
    ///   (offline 状态需要 `stop_reconnect_loop` 主动取消, 或 attempt 上限触发,
    ///   Stage 5 不实现 attempt 上限)
    ///
    /// 取消: drop 返回的 `JoinHandle` 不会真的停止 task; 改成
    /// `stop_reconnect_loop` 显式置标志. (简化: 把 on_state 包装到 Arc<Mutex<bool>>,
    /// task 每 tick 查一次.) Stage 5 用 `tokio::sync::Notify` 实现 stop signal.
    pub fn start_reconnect_loop(
        &self,
        on_state: impl Fn(ReconnectState) + Send + Sync + 'static,
    ) -> ReconnectHandle {
        let client = self.clone();
        let tracker = Arc::new(Mutex::new(ReconnectTracker::new()));
        let stop = Arc::new(tokio::sync::Notify::new());
        let stop_for_task = stop.clone();
        let on_state: Arc<dyn Fn(ReconnectState) + Send + Sync + 'static> = Arc::new(on_state);
        let on_state_for_task = on_state.clone();

        let join = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            // 第一次 tick 立即触发, 避免 1s 延迟
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = stop_for_task.notified() => {
                        debug!("[client::reconnect] stop signal received, exiting loop");
                        return;
                    }
                    _ = interval.tick() => {
                        // 测 health, 短超时
                        let probe = tokio::time::timeout(
                            Duration::from_secs(3),
                            client.health(),
                        ).await;
                        let mut t = tracker.lock().await;
                        match probe {
                            Ok(Ok(h)) if h.status == "ok" => {
                                // 成功: 重置 attempt, 通知 connected
                                if t.attempt > 0 || t.in_reconnect {
                                    debug!("[client::reconnect] health ok after {} attempt(s)", t.attempt);
                                    t.attempt = 0;
                                    t.last_error = None;
                                    t.in_reconnect = false;
                                    drop(t);
                                    on_state_for_task(ReconnectState::Connected);
                                }
                            }
                            Ok(Ok(h)) => {
                                // health 返 200 但 status != ok
                                t.attempt = t.attempt.saturating_add(1);
                                t.last_error = Some(format!("daemon status={}", h.status));
                                t.in_reconnect = true;
                                let next = next_backoff(t.attempt);
                                debug!("[client::reconnect] unhealthy: {} (attempt={})", h.status, t.attempt);
                                drop(t);
                                on_state_for_task(ReconnectState::Reconnecting {
                                    attempt: t_after(&tracker).await,
                                    next_retry_in: next,
                                });
                            }
                            Ok(Err(e)) => {
                                t.attempt = t.attempt.saturating_add(1);
                                t.last_error = Some(e.to_string());
                                t.in_reconnect = true;
                                let next = next_backoff(t.attempt);
                                debug!("[client::reconnect] health error: {e} (attempt={})", t.attempt);
                                drop(t);
                                on_state_for_task(ReconnectState::Reconnecting {
                                    attempt: t_after(&tracker).await,
                                    next_retry_in: next,
                                });
                            }
                            Err(_) => {
                                t.attempt = t.attempt.saturating_add(1);
                                t.last_error = Some("health timeout (>3s)".to_string());
                                t.in_reconnect = true;
                                let next = next_backoff(t.attempt);
                                debug!("[client::reconnect] health timeout (attempt={})", t.attempt);
                                drop(t);
                                on_state_for_task(ReconnectState::Reconnecting {
                                    attempt: t_after(&tracker).await,
                                    next_retry_in: next,
                                });
                            }
                        }
                    }
                }
            }
        });

        ReconnectHandle {
            join: Some(join),
            stop,
        }
    }
}

/// 辅助: 重新获取 attempt (因为上面已经 drop 了 guard).
async fn t_after(tracker: &Arc<Mutex<ReconnectTracker>>) -> u32 {
    tracker.lock().await.attempt
}

/// 自动重连循环的 handle — drop 时**不会**自动停, 需调 `stop()` 显式停止.
pub struct ReconnectHandle {
    join: Option<tokio::task::JoinHandle<()>>,
    stop: Arc<tokio::sync::Notify>,
}

impl ReconnectHandle {
    /// 显式停止重连循环. 任务在下一次 tick 之前退出.
    pub fn stop(&mut self) {
        self.stop.notify_waiters();
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

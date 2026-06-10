//! 缺口 05: 后台异步任务管理.
//!
//! ## 设计要点
//!
//! - **TaskStatus** (5 变体): Pending / Running / Paused / Cancelled / Done
//! - **BackgroundTaskManager**:
//!   - FIFO 队列 + 状态机
//!   - 并发上限 5 (常量 `MAX_CONCURRENT`)
//!   - 超出上限的任务入 Pending 队列, 前面 Done 后自动 promote
//!   - 支持 cancel (任意状态) / resume (Paused → Running)
//! - **TaskInfo** — 任务元信息 (id / kind / opts / status / created_at / updated_at / result)
//! - **TaskKind** — 任务类型 enum (e.g. `IndexBuild` / `MemoryFlush` / `SkillReload` / `LongPrompt`)
//! - **SseEvent 联动**: 4 个新变体 (Started/Updated/Cancelled/Completed)
//!
//! ## 不做什么
//!
//! - 不做 SQLite 持久化 (Stage 6 接入 `qianxun-runtime/src/persistence.rs`)
//! - 不做 task 重试 / 退避策略 (cancel 终态)
//! - 不做分布式 (单进程内存队列)
//!
//! ## 调用方
//!
//! - `RuntimeApi::start_background_task` 创建并入队
//! - 5 状态转换: Pending → Running → Done / Cancelled, Paused ↔ Running
//! - 真实业务 (e.g. IndexBuild) 调 `manager.update_progress` 报告进度

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

// ─── 常量 ──────────────────────────────────────────────────

/// 最大并发任务数.
pub const MAX_CONCURRENT: usize = 5;

// ─── TaskStatus ────────────────────────────────────────────

/// 任务状态机.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 等待并发槽位
    Pending,
    /// 正在跑
    Running,
    /// 用户暂停
    Paused,
    /// 用户取消 (终态)
    Cancelled,
    /// 跑完 (终态)
    Done,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
            Self::Done => "done",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Cancelled | Self::Done)
    }
}

// ─── TaskKind ──────────────────────────────────────────────

/// 任务类型 (Stage 5 起步: 4 种, 后续可扩).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// 索引构建 (代码/Memory 索引)
    IndexBuild,
    /// 记忆压缩
    MemoryFlush,
    /// 技能重载
    SkillReload,
    /// 长 prompt 后台执行
    LongPrompt,
    /// 通用自定义
    Custom(String),
}

impl TaskKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::IndexBuild => "index_build",
            Self::MemoryFlush => "memory_flush",
            Self::SkillReload => "skill_reload",
            Self::LongPrompt => "long_prompt",
            Self::Custom(s) => s,
        }
    }
}

// ─── TaskInfo ──────────────────────────────────────────────

/// 任务元信息.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub task_id: String,
    pub task_kind: TaskKind,
    /// 任务参数 (e.g. index_build 的路径, long_prompt 的 session_id)
    pub opts: serde_json::Value,
    pub status: TaskStatus,
    pub created_at: i64,
    pub updated_at: i64,
    /// 0.0 ~ 1.0 进度
    pub progress: Option<f64>,
    /// 终态时填 result
    pub result: Option<serde_json::Value>,
    /// 取消原因
    pub cancel_reason: Option<String>,
}

// ─── BackgroundTaskManager ────────────────────────────────

/// 后台任务管理器 (FIFO + 状态机).
pub struct BackgroundTaskManager {
    /// 活跃任务: task_id → TaskInfo
    tasks: Mutex<HashMap<String, TaskInfo>>,
    /// FIFO 等待队列 (pending task ids)
    pending_queue: Mutex<VecDeque<String>>,
    /// 当前 Running 数
    running_count: AtomicU64,
    /// 全局 task id 计数器 (e2e 验证用)
    task_counter: AtomicU64,
}

impl BackgroundTaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            pending_queue: Mutex::new(VecDeque::new()),
            running_count: AtomicU64::new(0),
            task_counter: AtomicU64::new(0),
        }
    }

    /// 启动一个新后台任务.
    ///
    /// - 如果当前 Running < 5 → 立即 Running
    /// - 否则 → Pending (入 FIFO)
    pub async fn start(
        &self,
        kind: TaskKind,
        opts: serde_json::Value,
    ) -> TaskInfo {
        let task_id = format!("bgt_{}", self.task_counter.fetch_add(1, Ordering::Relaxed));
        let now = current_unix_ms();
        let mut info = TaskInfo {
            task_id: task_id.clone(),
            task_kind: kind,
            opts,
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
            progress: Some(0.0),
            result: None,
            cancel_reason: None,
        };

        let mut tasks = self.tasks.lock().await;
        let mut queue = self.pending_queue.lock().await;
        let running = self.running_count.load(Ordering::Relaxed) as usize;

        if running < MAX_CONCURRENT {
            info.status = TaskStatus::Running;
            self.running_count.fetch_add(1, Ordering::Relaxed);
            tracing::info!(task_id = %info.task_id, "[bgt] task started (running={})", running + 1);
        } else {
            queue.push_back(task_id.clone());
            tracing::info!(task_id = %info.task_id, "[bgt] task queued (queue len={})", queue.len());
        }
        tasks.insert(task_id, info.clone());
        info
    }

    /// 拿 task 详情.
    pub async fn get(&self, task_id: &str) -> Option<TaskInfo> {
        self.tasks.lock().await.get(task_id).cloned()
    }

    /// 列所有 task, 可选 status 过滤.
    pub async fn list(&self, filter: Option<TaskStatus>) -> Vec<TaskInfo> {
        let tasks = self.tasks.lock().await;
        let mut out: Vec<TaskInfo> = tasks
            .values()
            .filter(|t| filter.is_none_or(|f| t.status == f))
            .cloned()
            .collect();
        // 按 created_at 升序 (FIFO 顺序)
        out.sort_by_key(|t| t.created_at);
        out
    }

    /// 取消任务 (任意状态都可, 终态返回 Err).
    pub async fn cancel(&self, task_id: &str, reason: &str) -> Result<(), BgtError> {
        let mut tasks = self.tasks.lock().await;
        let info = tasks.get_mut(task_id).ok_or(BgtError::NotFound)?;
        if info.status.is_terminal() {
            return Err(BgtError::AlreadyTerminal(info.status));
        }
        let prev = info.status;
        info.status = TaskStatus::Cancelled;
        info.cancel_reason = Some(reason.to_string());
        info.updated_at = current_unix_ms();
        // Running → Cancelled: 释放并发槽位 + promote queue
        if prev == TaskStatus::Running {
            self.running_count.fetch_sub(1, Ordering::Relaxed);
        }
        // 如果是 Pending 还要从 queue 移除
        if prev == TaskStatus::Pending {
            let mut queue = self.pending_queue.lock().await;
            queue.retain(|id| id != task_id);
        }
        Ok(())
    }

    /// 恢复 Paused 任务 → Running.
    pub async fn resume(&self, task_id: &str) -> Result<(), BgtError> {
        let mut tasks = self.tasks.lock().await;
        let info = tasks.get_mut(task_id).ok_or(BgtError::NotFound)?;
        if info.status != TaskStatus::Paused {
            return Err(BgtError::InvalidStateTransition {
                from: info.status,
                to: TaskStatus::Running,
            });
        }
        info.status = TaskStatus::Running;
        info.updated_at = current_unix_ms();
        self.running_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// 暂停 Running 任务 → Paused.
    pub async fn pause(&self, task_id: &str) -> Result<(), BgtError> {
        let mut tasks = self.tasks.lock().await;
        let info = tasks.get_mut(task_id).ok_or(BgtError::NotFound)?;
        if info.status != TaskStatus::Running {
            return Err(BgtError::InvalidStateTransition {
                from: info.status,
                to: TaskStatus::Paused,
            });
        }
        info.status = TaskStatus::Paused;
        info.updated_at = current_unix_ms();
        self.running_count.fetch_sub(1, Ordering::Relaxed);
        // 释放出的槽位由 promote_pending 接管 (业务调用方负责)
        Ok(())
    }

    /// 标记任务完成, 释放并发槽位, 自动 promote queue.
    pub async fn complete(
        &self,
        task_id: &str,
        result: serde_json::Value,
    ) -> Result<(), BgtError> {
        let mut tasks = self.tasks.lock().await;
        let info = tasks.get_mut(task_id).ok_or(BgtError::NotFound)?;
        if info.status.is_terminal() {
            return Err(BgtError::AlreadyTerminal(info.status));
        }
        let prev = info.status;
        info.status = TaskStatus::Done;
        info.result = Some(result);
        info.progress = Some(1.0);
        info.updated_at = current_unix_ms();
        if prev == TaskStatus::Running {
            self.running_count.fetch_sub(1, Ordering::Relaxed);
        }
        drop(tasks);
        // 自动 promote FIFO head
        self.promote_pending().await;
        Ok(())
    }

    /// 更新进度 (Running 任务用).
    pub async fn update_progress(
        &self,
        task_id: &str,
        progress: f64,
        message: Option<String>,
    ) -> Result<(), BgtError> {
        let mut tasks = self.tasks.lock().await;
        let info = tasks.get_mut(task_id).ok_or(BgtError::NotFound)?;
        if info.status != TaskStatus::Running {
            return Err(BgtError::InvalidStateTransition {
                from: info.status,
                to: TaskStatus::Running,
            });
        }
        info.progress = Some(progress.clamp(0.0, 1.0));
        info.updated_at = current_unix_ms();
        let _ = message; // 暂不存 message
        Ok(())
    }

    /// 把 FIFO head 提升到 Running (有空闲槽位时).
    async fn promote_pending(&self) {
        let running = self.running_count.load(Ordering::Relaxed) as usize;
        if running >= MAX_CONCURRENT {
            return;
        }
        let mut queue = self.pending_queue.lock().await;
        let next_id = match queue.pop_front() {
            Some(id) => id,
            None => return,
        };
        drop(queue);
        let mut tasks = self.tasks.lock().await;
        if let Some(info) = tasks.get_mut(&next_id) {
            if info.status == TaskStatus::Pending {
                info.status = TaskStatus::Running;
                info.updated_at = current_unix_ms();
                self.running_count.fetch_add(1, Ordering::Relaxed);
                tracing::info!(task_id = %next_id, "[bgt] promoted pending → running");
            }
        }
    }

    /// 当前 Running 数 (e2e 验证用).
    pub fn running_count(&self) -> u64 {
        self.running_count.load(Ordering::Relaxed)
    }

    /// 当前 pending queue 长度.
    pub async fn queue_len(&self) -> usize {
        self.pending_queue.lock().await.len()
    }

    /// 测试辅助: 强制把 running_count 设回 0 (清状态).
    #[cfg(test)]
    pub fn reset_running(&self) {
        self.running_count.store(0, Ordering::Relaxed);
    }
}

impl Default for BackgroundTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─── BgtError ──────────────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum BgtError {
    #[error("task not found")]
    NotFound,
    #[error("task already in terminal state: {0:?}")]
    AlreadyTerminal(TaskStatus),
    #[error("invalid state transition: {from:?} → {to:?}")]
    InvalidStateTransition {
        from: TaskStatus,
        to: TaskStatus,
    },
}

// ─── helper ────────────────────────────────────────────────

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ─── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── TaskStatus ──

    #[test]
    fn test_task_status_as_str() {
        assert_eq!(TaskStatus::Pending.as_str(), "pending");
        assert_eq!(TaskStatus::Running.as_str(), "running");
        assert_eq!(TaskStatus::Paused.as_str(), "paused");
        assert_eq!(TaskStatus::Cancelled.as_str(), "cancelled");
        assert_eq!(TaskStatus::Done.as_str(), "done");
    }

    #[test]
    fn test_task_status_is_terminal() {
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(TaskStatus::Done.is_terminal());
        assert!(!TaskStatus::Pending.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(!TaskStatus::Paused.is_terminal());
    }

    // ── start / get / list ──

    #[tokio::test]
    async fn test_start_first_task_runs_immediately() {
        let m = BackgroundTaskManager::new();
        let info = m.start(TaskKind::IndexBuild, json!({})).await;
        assert_eq!(info.status, TaskStatus::Running);
        assert_eq!(m.running_count(), 1);

        let fetched = m.get(&info.task_id).await.unwrap();
        assert_eq!(fetched.task_id, info.task_id);
        assert_eq!(fetched.status, TaskStatus::Running);
    }

    #[tokio::test]
    async fn test_start_5_concurrent_all_running() {
        let m = BackgroundTaskManager::new();
        let mut ids = Vec::new();
        for _ in 0..MAX_CONCURRENT {
            let info = m.start(TaskKind::IndexBuild, json!({})).await;
            ids.push(info.task_id);
            assert_eq!(info.status, TaskStatus::Running);
        }
        assert_eq!(m.running_count(), MAX_CONCURRENT as u64);
        assert_eq!(m.queue_len().await, 0);
    }

    #[tokio::test]
    async fn test_start_6th_task_queued_as_pending() {
        let m = BackgroundTaskManager::new();
        for _ in 0..MAX_CONCURRENT {
            let _ = m.start(TaskKind::IndexBuild, json!({})).await;
        }
        // 第 6 个
        let info = m.start(TaskKind::LongPrompt, json!({})).await;
        assert_eq!(info.status, TaskStatus::Pending);
        assert_eq!(m.queue_len().await, 1);
        assert_eq!(m.running_count(), MAX_CONCURRENT as u64);
    }

    #[tokio::test]
    async fn test_complete_promotes_pending_fifo() {
        let m = BackgroundTaskManager::new();
        let mut ids = Vec::new();
        for _ in 0..MAX_CONCURRENT {
            let info = m.start(TaskKind::IndexBuild, json!({})).await;
            ids.push(info.task_id);
        }
        let queued = m.start(TaskKind::LongPrompt, json!({})).await;
        assert_eq!(m.queue_len().await, 1);

        // 完成第一个 → 应该 promote queued
        m.complete(&ids[0], json!({"ok": true})).await.unwrap();
        assert_eq!(m.running_count(), MAX_CONCURRENT as u64); // 5 个仍跑
        let q = m.get(&queued.task_id).await.unwrap();
        assert_eq!(q.status, TaskStatus::Running);
        assert_eq!(m.queue_len().await, 0);
    }

    // ── cancel ──

    #[tokio::test]
    async fn test_cancel_running_task() {
        let m = BackgroundTaskManager::new();
        let info = m.start(TaskKind::IndexBuild, json!({})).await;
        m.cancel(&info.task_id, "user requested").await.unwrap();
        let fetched = m.get(&info.task_id).await.unwrap();
        assert_eq!(fetched.status, TaskStatus::Cancelled);
        assert_eq!(fetched.cancel_reason, Some("user requested".to_string()));
        assert_eq!(m.running_count(), 0); // 释放槽位
    }

    #[tokio::test]
    async fn test_cancel_pending_removes_from_queue() {
        let m = BackgroundTaskManager::new();
        for _ in 0..MAX_CONCURRENT {
            let _ = m.start(TaskKind::IndexBuild, json!({})).await;
        }
        let queued = m.start(TaskKind::LongPrompt, json!({})).await;
        assert_eq!(m.queue_len().await, 1);

        m.cancel(&queued.task_id, "no longer needed").await.unwrap();
        assert_eq!(m.queue_len().await, 0);
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_returns_not_found() {
        let m = BackgroundTaskManager::new();
        let r = m.cancel("nonexistent", "x").await;
        assert!(matches!(r, Err(BgtError::NotFound)));
    }

    #[tokio::test]
    async fn test_cancel_already_terminal_errors() {
        let m = BackgroundTaskManager::new();
        let info = m.start(TaskKind::IndexBuild, json!({})).await;
        m.cancel(&info.task_id, "first").await.unwrap();
        let r = m.cancel(&info.task_id, "second").await;
        assert!(matches!(r, Err(BgtError::AlreadyTerminal(_))));
    }

    // ── pause / resume ──

    #[tokio::test]
    async fn test_pause_running_then_resume() {
        let m = BackgroundTaskManager::new();
        let info = m.start(TaskKind::IndexBuild, json!({})).await;
        m.pause(&info.task_id).await.unwrap();
        let p = m.get(&info.task_id).await.unwrap();
        assert_eq!(p.status, TaskStatus::Paused);
        assert_eq!(m.running_count(), 0);

        m.resume(&info.task_id).await.unwrap();
        let r = m.get(&info.task_id).await.unwrap();
        assert_eq!(r.status, TaskStatus::Running);
        assert_eq!(m.running_count(), 1);
    }

    #[tokio::test]
    async fn test_resume_non_paused_errors() {
        let m = BackgroundTaskManager::new();
        let info = m.start(TaskKind::IndexBuild, json!({})).await;
        let r = m.resume(&info.task_id).await;
        assert!(matches!(r, Err(BgtError::InvalidStateTransition { .. })));
    }

    // ── list filter ──

    #[tokio::test]
    async fn test_list_filter_by_status() {
        let m = BackgroundTaskManager::new();
        let a = m.start(TaskKind::IndexBuild, json!({})).await;
        let _b = m.start(TaskKind::LongPrompt, json!({})).await;
        m.cancel(&a.task_id, "x").await.unwrap();

        let all = m.list(None).await;
        assert_eq!(all.len(), 2);

        let running = m.list(Some(TaskStatus::Running)).await;
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].status, TaskStatus::Running);

        let cancelled = m.list(Some(TaskStatus::Cancelled)).await;
        assert_eq!(cancelled.len(), 1);
    }

    // ── update_progress ──

    #[tokio::test]
    async fn test_update_progress_clamps_to_0_1() {
        let m = BackgroundTaskManager::new();
        let info = m.start(TaskKind::IndexBuild, json!({})).await;
        m.update_progress(&info.task_id, 1.5, None).await.unwrap();
        let p = m.get(&info.task_id).await.unwrap();
        assert_eq!(p.progress, Some(1.0));

        m.update_progress(&info.task_id, -0.5, None).await.unwrap();
        let p = m.get(&info.task_id).await.unwrap();
        assert_eq!(p.progress, Some(0.0));
    }

    #[tokio::test]
    async fn test_update_progress_on_non_running_errors() {
        let m = BackgroundTaskManager::new();
        let info = m.start(TaskKind::IndexBuild, json!({})).await;
        m.cancel(&info.task_id, "x").await.unwrap();
        let r = m.update_progress(&info.task_id, 0.5, None).await;
        assert!(r.is_err());
    }

    // ── TaskKind ──

    #[test]
    fn test_task_kind_as_str() {
        assert_eq!(TaskKind::IndexBuild.as_str(), "index_build");
        assert_eq!(TaskKind::MemoryFlush.as_str(), "memory_flush");
        assert_eq!(TaskKind::SkillReload.as_str(), "skill_reload");
        assert_eq!(TaskKind::LongPrompt.as_str(), "long_prompt");
        assert_eq!(TaskKind::Custom("my_task".into()).as_str(), "my_task");
    }

    // ── MAX_CONCURRENT 常量 ──

    #[test]
    fn test_max_concurrent_is_5() {
        assert_eq!(MAX_CONCURRENT, 5);
    }
}

// 引入 serde_json::json 宏 (在测试模块用)
#[allow(unused_imports)]
use serde_json::json;

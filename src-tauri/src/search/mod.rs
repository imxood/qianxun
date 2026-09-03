//! 搜索域（M2，架构 §4.9）：文件名搜索与内容搜索双页的 Rust 侧。
//!
//! 底座是 fff-search 的常驻 FilePicker：后台线程扫描根目录、watch 增量
//! 维护、fuzzy/打分一次到位。文件名搜索与内容搜索共享同一份索引——
//! 换根目录 = 重建 FilePicker，由世代号让旧查询作废。

pub mod commands;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fff_search::SharedFilePicker;

/// 搜索域进程级状态：与设置、托管并排挂在 AppState 上。
pub struct SearchState {
    /// fff 的共享 picker：Arc<RwLock<Option<FilePicker>>>。
    picker: SharedFilePicker,
    /// 当前索引根目录（绝对路径规范形）。
    root: Mutex<Option<PathBuf>>,
    /// 根目录世代：每次换根 +1，旧扫描/旧查询据此作废。
    pub generation: AtomicU64,
    /// 内容搜索的取消令牌：新搜索自动取消上一次（UI 天然语义）。
    abort: Mutex<Option<Arc<AtomicBool>>>,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            picker: SharedFilePicker::default(),
            root: Mutex::new(None),
            generation: AtomicU64::new(0),
            abort: Mutex::new(None),
        }
    }

    /// 当前根目录（快照）。
    pub fn root(&self) -> Option<PathBuf> {
        self.root.lock().unwrap().clone()
    }

    /// 更换根目录记录。
    pub fn set_root(&self, root: Option<PathBuf>) {
        *self.root.lock().unwrap() = root;
    }

    /// fff 的共享 picker 引用。
    pub fn picker(&self) -> &SharedFilePicker {
        &self.picker
    }

    /// 世代号：换根时 +1，旧查询据此作废。
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// 令牌接力：替换并取消旧令牌，返回新令牌。
    pub fn rotate_abort(&self) -> Arc<AtomicBool> {
        let fresh = Arc::new(AtomicBool::new(false));
        let mut slot = self.abort.lock().unwrap();
        if let Some(previous) = slot.take() {
            previous.store(true, Ordering::Relaxed);
        }
        *slot = Some(Arc::clone(&fresh));
        fresh
    }

    /// 外部取消（取消按钮）。
    pub fn cancel_search(&self) {
        if let Some(token) = self.abort.lock().unwrap().as_ref() {
            token.store(true, Ordering::Relaxed);
        }
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

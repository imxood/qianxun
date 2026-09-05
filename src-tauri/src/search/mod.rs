//! 搜索域（M2，架构 §4.9）：文件名搜索与内容搜索双页的 Rust 侧。
//!
//! 底座是 fff-search 的常驻 FilePicker：后台线程扫描根目录、watch 增量
//! 维护、fuzzy/打分一次到位。文件名搜索与内容搜索共享同一份索引——
//! 换根目录 = 切换 FilePicker，由世代号让旧查询作废。
//!
//! 暖缓存：换下来的根索引不销毁（watcher 持有同 Arc 继续扫描+增量），
//! 切回即达、无需重扫。fff 的 watcher 绑定 picker 的 Arc 身份，因此
//! 缓存必须整 Arc 保存、整 Arc 换回，不能只换内层 Option。

pub mod commands;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fff_search::SharedFilePicker;

/// 暖缓存容量（不含当前激活根）：C:/D: 来回切换零重建；
/// 超出容量按 LRU 淘汰（drop 即释放该根索引的内存）。
const WARM_CACHE_CAP: usize = 2;

/// 搜索域进程级状态：与设置、托管并排挂在 AppState 上。
pub struct SearchState {
    /// 当前激活的 fff 共享 picker：Arc<RwLock<Option<FilePicker>>>。
    /// Mutex 只护 Arc 指针交换（暖切换），内层 RwLock 才是读索引的锁。
    picker: Mutex<SharedFilePicker>,
    /// 当前索引根目录（绝对路径规范形）。
    root: Mutex<Option<PathBuf>>,
    /// 根目录世代：每次换根 +1，旧扫描/旧查询据此作废。
    pub generation: AtomicU64,
    /// 内容搜索的取消令牌：新搜索自动取消上一次（UI 天然语义）。
    abort: Mutex<Option<Arc<AtomicBool>>>,
    /// 换下来的根索引暖缓存（LRU：头最旧）。(根, picker Arc) 整体保存。
    warm: Mutex<VecDeque<(PathBuf, SharedFilePicker)>>,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            picker: Mutex::new(SharedFilePicker::default()),
            root: Mutex::new(None),
            generation: AtomicU64::new(0),
            abort: Mutex::new(None),
            warm: Mutex::new(VecDeque::new()),
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

    /// 当前激活的共享 picker（Arc 克隆，代价 O(1)）。
    pub fn picker(&self) -> SharedFilePicker {
        self.picker.lock().unwrap().clone()
    }

    /// 替换激活 picker：整 Arc 换入（暖切换换回、或建新索引后换入）——
    /// watcher 持有同一 Arc 的弱引用，整 Arc 换入即无缝接管。
    pub fn set_picker(&self, picker: SharedFilePicker) {
        *self.picker.lock().unwrap() = picker;
    }

    /// 世代号：换根时 +1，旧查询据此作废。
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// 暖缓存命中：取出该根的 picker（有则从缓存移除并返回）。
    pub fn take_warm(&self, root: &Path) -> Option<SharedFilePicker> {
        let mut warm = self.warm.lock().unwrap();
        let index = warm.iter().position(|(path, _)| path == root)?;
        let (_, picker) = warm.remove(index)?;
        Some(picker)
    }

    /// 把换下来的激活 picker 存入暖缓存（LRU 满则淘汰最旧；
    /// drop 淘汰项即释放该根索引内存，其 watcher 随 Arc 归零自动退场）。
    pub fn stash_warm(&self, root: PathBuf, picker: SharedFilePicker) {
        let mut warm = self.warm.lock().unwrap();
        if let Some(slot) = warm.iter().position(|(path, _)| *path == root) {
            warm.remove(slot);
        }
        warm.push_back((root, picker));
        while warm.len() > WARM_CACHE_CAP {
            warm.pop_front();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 暖缓存语义：命中即移除（切回激活位）、未命中 None、
    /// 容量 LRU（超过 WARM_CACHE_CAP 淘汰最旧）。
    #[test]
    fn 暖缓存命中移除与_lru_淘汰() {
        let state = SearchState::new();
        let a = SharedFilePicker::default();
        let b = SharedFilePicker::default();
        let c = SharedFilePicker::default();
        let d = SharedFilePicker::default();

        // 模拟：激活 C:\，切到 D:\（C 进缓存）→ 再切 E:\（D 进缓存）
        state.stash_warm(PathBuf::from("C:\\"), a);
        state.stash_warm(PathBuf::from("D:\\"), b);

        // 命中即移除：再取一次应为 None。
        assert!(state.take_warm(Path::new("C:\\")).is_some());
        assert!(state.take_warm(Path::new("C:\\")).is_none());
        assert!(state.take_warm(Path::new("不存在:\\")).is_none());

        // LRU：已有 D:\（1 项），连进 E:\ F:\ 后 D:\ 应被淘汰，
        // 保留的是最新的 E:\ 与 F:\。
        state.stash_warm(PathBuf::from("E:\\"), c);
        state.stash_warm(PathBuf::from("F:\\"), d);
        assert!(state.take_warm(Path::new("D:\\")).is_none());
        assert!(state.take_warm(Path::new("E:\\")).is_some());
        assert!(state.take_warm(Path::new("F:\\")).is_some());
    }
}

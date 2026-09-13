//! Disk space scanner: parallel walk of an arbitrary directory with
//! streaming progress callbacks and cancellation support.
//!
//! Design goals (qianxun 磁盘扫描页):
//! - any directory can be scanned (drive root, project dir, whatever);
//! - the walk runs on the existing background rayon pool, workers push
//!   discovered entries into a shared buffer, the caller flushes at its own
//!   cadence (qianxun flushes every 100ms) — the scanner never blocks on UI;
//! - progress ticks fire every 512 entries *globally* (not per worker), so
//!   even small scans produce frames, and additionally carry the scan root's
//!   direct children with their *partial* accumulated sizes (maintained
//!   incrementally per file, O(1) amortised) — the frontend renders a
//!   provisional treemap that grows while the walk is still running
//!   ("边扫边长");
//! - after the walk finishes, recursive directory sizes are computed
//!   bottom-up (leaf-first sort), the full entry tree is assembled and the
//!   largest files are picked via `select_nth` (O(n));
//! - `cancelled` is a shared `AtomicBool`: the walker checks it per entry and
//!   stops early, returning whatever was collected so far.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::parallelism::BACKGROUND_THREAD_POOL;

/// 一次进度快照里直接子项的数量上限（降序截断；巨根也不会撑爆 IPC 帧）。
const TOP_CHILDREN_LIMIT: usize = 200;
/// 全局每发现多少条目触发一次进度帧（跨线程计数，小目录也有帧）。
const TICK_EVERY: usize = 512;
/// 扫描结束时挑选的最大文件个数。
pub const LARGEST_FILES_LIMIT: usize = 10;

/// One discovered entry: a file, or a directory with its direct children
/// filled in after the walk (`children` stays empty while streaming).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskEntry {
    /// Directory path this entry lives in ("" for entries at the scan root).
    pub parent: String,
    /// Absolute path.
    pub path: String,
    /// File name / directory name.
    pub name: String,
    /// Recursive size in bytes (exact after the walk; files know it immediately).
    pub size: u64,
    pub is_dir: bool,
    /// Direct children (only populated by [`DiskScanResult::tree_root`]).
    pub children: Vec<DiskEntry>,
}

impl DiskEntry {
    fn new(parent: String, path: PathBuf, name: String, size: u64, is_dir: bool) -> Self {
        Self {
            parent,
            path: path.to_string_lossy().into_owned(),
            name,
            size,
            is_dir,
            children: Vec::new(),
        }
    }
}

/// 流式进度快照。遍历未结束时所有数值都是部分值。
#[derive(Debug, Clone)]
pub struct ScanTick {
    /// 已发现文件数。
    pub files: u64,
    /// 已发现目录数（不含扫描根）。
    pub dirs: u64,
    /// 已发现文件的字节和（增量维护，tick 不再全量求和）。
    pub bytes: u64,
    /// 因权限 / 系统错误被跳过的条目数（大小未知，不计入 bytes）。
    pub skipped: u64,
    /// 扫描根直接子项的即时（部分）占用，降序，截断 [`TOP_CHILDREN_LIMIT`]。
    pub top_children: Vec<PartialChild>,
}

/// 扫描根直接子项的部分占用快照（「边扫边长」的数据源）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialChild {
    pub name: String,
    pub path: String,
    /// 遍历中 = 已落进该子树的文件字节和；遍历结束后为精确值。
    pub size: u64,
    pub is_dir: bool,
}

/// 占用最大的单个文件（扫描结束时选 TOP N，降序）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

/// Final scan outcome. `entries` is the full flat list (any depth);
/// `tree_root` is the scan root with children nested recursively.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskScanResult {
    /// Directory the scan was started from.
    pub root: String,
    /// True if the walk stopped early due to cancellation.
    pub cancelled: bool,
    /// Discovered files count.
    pub file_count: usize,
    /// Discovered directory count (excluding the scan root).
    pub dir_count: usize,
    /// Recursive size of the scan root.
    pub total_size: u64,
    /// Entries skipped due to permission / OS errors (sizes unknown).
    pub skipped_count: usize,
    /// Largest files (descending, capped at [`LARGEST_FILES_LIMIT`]).
    pub largest_files: Vec<LargeFile>,
    /// Flat list of every entry.
    pub entries: Vec<DiskEntry>,
    /// Scan root entry with children assembled (sizes are recursive).
    pub tree_root: DiskEntry,
}

fn dir_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Shared accumulator for entries discovered by the parallel walker.
#[derive(Default)]
struct Collected {
    dirs: Vec<(PathBuf, String)>,
    files: Vec<(PathBuf, String, u64)>,
    file_count: usize,
    dir_count: usize,
    skipped: usize,
    /// 已发现条目总数（全局 tick 节奏用）。
    seen: usize,
    /// 已发现文件字节和（增量维护）。
    bytes_so_far: u64,
    /// 扫描根直接子项 → 部分占用（每发现一个条目 O(1) 记到顶层祖先）。
    root_children: HashMap<String, PartialChild>,
}

impl Collected {
    /// 把一个条目记到扫描根的直接子项桶上：`C:\root\a\b\f.txt` 记到 `a`。
    /// 目录只占位（大小随文件积累），文件累加字节。
    fn note_root_child(&mut self, root_key: &str, key: &str, size: u64, is_dir: bool) {
        let Some((child_key, seg)) = direct_child_key(root_key, key) else {
            return;
        };
        let entry = self
            .root_children
            .entry(child_key.clone())
            .or_insert_with(|| PartialChild {
                name: seg,
                path: child_key,
                size: 0,
                is_dir,
            });
        entry.size += size;
        if is_dir {
            entry.is_dir = true;
        }
    }

    /// 直接子项快照：降序 + 截断。目录大小此刻只含已发现文件（部分值）。
    fn top_children(&self) -> Vec<PartialChild> {
        let mut tops: Vec<PartialChild> = self.root_children.values().cloned().collect();
        tops.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        tops.truncate(TOP_CHILDREN_LIMIT);
        tops
    }
}

/// 扫描根直接子项的键与末段名：`C:\root` + `C:\root\a\b` → `("C:\root\a", "a")`。
/// 条目即扫描根本身（或前缀不符）时返回 None。
fn direct_child_key(root_key: &str, key: &str) -> Option<(String, String)> {
    if key.len() <= root_key.len() || !key.starts_with(root_key) {
        return None;
    }
    // root_key 之后必跟分隔符（两边都经过 normalize_key，尾分隔符已剥掉）。
    // 字节切片安全：starts_with 保证前缀逐字节相同，分隔符本身是 ASCII。
    let rest = &key[root_key.len() + 1..];
    let seg = match rest.find('\\') {
        Some(idx) => &rest[..idx],
        None => rest,
    };
    if seg.is_empty() {
        return None;
    }
    Some((format!("{root_key}\\{seg}"), seg.to_owned()))
}

/// Scan `root` recursively. `on_tick` receives [`ScanTick`] snapshots of
/// everything discovered so far whenever the walker crosses a tick boundary —
/// call it as often as you like (qianxun uses ~100ms).
///
/// Cancellation: set `cancelled` to `true` from any thread; the walk stops
/// at the next entry and the result is marked `cancelled`.
pub fn scan_directory(
    root: &Path,
    cancelled: &AtomicBool,
    on_tick: impl FnMut(ScanTick) + Send,
) -> crate::Result<DiskScanResult> {
    BACKGROUND_THREAD_POOL.install(|| {
        let tick = Mutex::new(on_tick);
        let collected = Mutex::new(Collected::default());
        let root_key = path_key(root);

        walk_parallel(root, &root_key, cancelled, &collected, &tick)?;

        let Collected {
            dirs,
            mut files,
            file_count,
            dir_count,
            skipped,
            ..
        } = collected.into_inner().expect("collected");

        let root_str = root.to_string_lossy().into_owned();
        let was_cancelled = cancelled.load(Ordering::Relaxed);

        // ---- recursive sizes: leaf-first ----
        // Sort by path depth descending so children accumulate into parents
        // before parents accumulate into grandparents.
        let mut dir_sizes: HashMap<String, u64> = HashMap::with_capacity(dirs.len() + 1);
        for (path, _) in &dirs {
            dir_sizes.entry(path_key(path)).or_insert(0);
        }
        let mut file_total: u64 = 0;
        for (path, _, size) in &files {
            file_total += size;
            let mut cursor = path_key(path);
            while let Some(idx) = cursor.rfind(['/', '\\']) {
                cursor.truncate(idx);
                *dir_sizes.entry(cursor.clone()).or_insert(0) += size;
            }
        }
        dir_sizes.insert(root_key.clone(), file_total);

        // ---- flat entry list (dirs first, then files) ----
        let mut entries: Vec<DiskEntry> = Vec::with_capacity(dirs.len() + files.len());
        for (path, name) in &dirs {
            let parent = parent_key_of(&path_key(path));
            entries.push(DiskEntry::new(
                parent,
                path.clone(),
                name.clone(),
                dir_sizes.get(&path_key(path)).copied().unwrap_or(0),
                true,
            ));
        }
        for (path, name, size) in &files {
            let parent = parent_key_of(&path_key(path));
            entries.push(DiskEntry::new(
                parent,
                path.clone(),
                name.clone(),
                *size,
                false,
            ));
        }

        // ---- largest files: select_nth O(n) 选出 TOP N 再排序 ----
        let mut largest_files: Vec<LargeFile> = Vec::new();
        if !files.is_empty() {
            let k = LARGEST_FILES_LIMIT.min(files.len());
            files.select_nth_unstable_by(k - 1, |a, b| b.2.cmp(&a.2));
            largest_files = files[..k]
                .iter()
                .map(|(path, name, size)| LargeFile {
                    name: name.clone(),
                    path: path.to_string_lossy().into_owned(),
                    size: *size,
                })
                .collect();
            largest_files.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        }

        // ---- assemble the tree bottom-up ----
        // 桶按「父路径键」聚合；entries 最深优先，保证处理到某个目录时
        // 它的直接孩子已经全部到齐（孙辈已先挂进孩子的 children）。
        let mut parent_map: HashMap<String, Vec<DiskEntry>> = HashMap::new();
        entries.sort_by(|a, b| {
            b.path
                .split(['/', '\\'])
                .count()
                .cmp(&a.path.split(['/', '\\']).count())
                .then_with(|| a.path.cmp(&b.path))
        });
        for entry in entries {
            if entry.is_dir {
                // 该目录的直接孩子此时已聚齐，整体挂上后把自己交给父桶。
                let own_key = path_key(Path::new(&entry.path));
                if let Some(mut children) = parent_map.remove(&own_key) {
                    sort_children(&mut children);
                    let mut entry = entry;
                    entry.children = children;
                    parent_map
                        .entry(entry.parent.clone())
                        .or_default()
                        .push(entry);
                    continue;
                }
            }
            parent_map
                .entry(entry.parent.clone())
                .or_default()
                .push(entry);
        }
        let mut root_children = parent_map.remove(&root_key).unwrap_or_default();
        sort_children(&mut root_children);
        for children in parent_map.values_mut() {
            sort_children(children);
        }

        let tree_root = DiskEntry {
            parent: String::new(),
            path: root_str,
            name: dir_name_of(root),
            size: file_total,
            is_dir: true,
            children: root_children,
        };

        Ok(DiskScanResult {
            root: root.to_string_lossy().into_owned(),
            cancelled: was_cancelled,
            file_count,
            dir_count,
            total_size: file_total,
            skipped_count: skipped,
            largest_files,
            entries: Vec::new(),
            tree_root,
        })
    })
}

/// `ignore`-based parallel walk: skips symlinks (no cycles), collects dirs and
/// files with sizes, and ticks the caller with aggregated snapshots.
fn walk_parallel(
    root: &Path,
    root_key: &str,
    cancelled: &AtomicBool,
    collected: &Mutex<Collected>,
    tick: &Mutex<impl FnMut(ScanTick) + Send>,
) -> crate::Result<()> {
    use ignore::WalkBuilder;

    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .threads(BACKGROUND_THREAD_POOL.current_num_threads());

    let walker = builder.build_parallel();

    walker.run(|| {
        let collected = &collected;
        let tick = &tick;
        let cancelled = cancelled;
        let root_key = root_key;

        Box::new(move |result| {
            if cancelled.load(Ordering::Relaxed) {
                return ignore::WalkState::Quit;
            }
            let Ok(entry) = result else {
                // 权限拒绝 / 遍历 IO 错误：计数但不计入任何大小，前端透明化。
                collected.lock().expect("collected").skipped += 1;
                return ignore::WalkState::Continue;
            };
            let Ok(metadata) = entry.metadata() else {
                collected.lock().expect("collected").skipped += 1;
                return ignore::WalkState::Continue;
            };
            let file_type = match entry.file_type() {
                Some(ft) => ft,
                None => {
                    collected.lock().expect("collected").skipped += 1;
                    return ignore::WalkState::Continue;
                }
            };
            let path = entry.path().to_path_buf();
            // 扫描根本身不算一个条目（dir_count/树都不含它，见结构体注释）。
            if entry.depth() == 0 {
                return ignore::WalkState::Continue;
            }
            let name = dir_name_of(&path);
            let key = path_key(&path);

            // 入账与 tick 判定同一次持锁完成；tick 回调放锁外（可能做 IPC）。
            let should_tick = {
                let mut guard = collected.lock().expect("collected");
                if file_type.is_dir() {
                    guard.dirs.push((path, name));
                    guard.dir_count += 1;
                    // 顶层目录桶即使还没有文件也要占位（空目录可见）。
                    guard.note_root_child(root_key, &key, 0, true);
                } else if file_type.is_file() {
                    let size = metadata.len();
                    guard.files.push((path, name, size));
                    guard.file_count += 1;
                    guard.bytes_so_far += size;
                    guard.note_root_child(root_key, &key, size, false);
                }
                guard.seen += 1;
                guard.seen % TICK_EVERY == 0
            };
            if should_tick {
                let guard = collected.lock().expect("collected");
                (tick.lock().expect("tick"))(ScanTick {
                    files: guard.file_count as u64,
                    dirs: guard.dir_count as u64,
                    bytes: guard.bytes_so_far,
                    skipped: guard.skipped as u64,
                    top_children: guard.top_children(),
                });
            }

            ignore::WalkState::Continue
        })
    });

    Ok(())
}

fn sort_children(children: &mut [DiskEntry]) {
    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
}

/// Canonical-ish key for parent/child matching: forward slashes, no trailing
/// separator (root keeps its own form, e.g. `C:`).
fn normalize_key(path: &str) -> String {
    let mut key = path.replace('/', "\\");
    while key.len() > 1 && key.ends_with('\\') {
        key.pop();
    }
    key
}

fn path_key(path: &Path) -> String {
    normalize_key(&path.to_string_lossy())
}

/// Parent key of a path key (`C:\a\b` → `C:\a`, `C:\a` → `C:`).
fn parent_key_of(key: &str) -> String {
    match key.rfind('\\') {
        Some(0) => key[..1].to_owned(),
        Some(idx) => key[..idx].to_owned(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet_tick(_: ScanTick) {}

    #[test]
    fn 扫描任意目录并组装树() {
        let dir = std::env::temp_dir().join(format!("fff-disk-{}", std::process::id()));
        // Windows 的 pid 会复用：先清掉上次崩溃残留，避免目录内容污染计数。
        let _ = std::fs::remove_dir_all(&dir);
        let nested = dir.join("nested/deeper");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.join("a.txt"), [0u8; 10]).unwrap();
        std::fs::write(nested.join("b.bin"), [0u8; 100]).unwrap();
        std::fs::write(dir.join("nested/c.log"), [0u8; 50]).unwrap();

        let cancelled = AtomicBool::new(false);
        let result = scan_directory(&dir, &cancelled, quiet_tick).unwrap();

        assert!(!result.cancelled);
        assert_eq!(result.file_count, 3);
        assert_eq!(result.total_size, 160);
        assert_eq!(result.skipped_count, 0);
        // root children sorted by size: nested(150) before a.txt(10)
        let children = &result.tree_root.children;
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "nested");
        assert_eq!(children[0].size, 150);
        assert_eq!(children[0].children.len(), 2);
        // deeper child has recursive parent size
        let deeper = children[0]
            .children
            .iter()
            .find(|c| c.name == "deeper")
            .unwrap();
        assert_eq!(deeper.size, 100);

        // 最大文件 TOP：b.bin(100) > c.log(50) > a.txt(10)。
        assert_eq!(result.largest_files.len(), 3);
        assert_eq!(result.largest_files[0].name, "b.bin");
        assert_eq!(result.largest_files[0].size, 100);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 取消后提前返回() {
        let dir = std::env::temp_dir().join(format!("fff-disk-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.txt"), [0u8; 1]).unwrap();

        let cancelled = AtomicBool::new(true);
        let result = scan_directory(&dir, &cancelled, quiet_tick).unwrap();
        assert!(result.cancelled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 直接子键切割() {
        let root = "C:\\root";
        assert_eq!(
            direct_child_key(root, "C:\\root\\a\\b\\f.txt"),
            Some(("C:\\root\\a".to_owned(), "a".to_owned()))
        );
        // 顶层文件：直接子项就是文件自己。
        assert_eq!(
            direct_child_key(root, "C:\\root\\f.txt"),
            Some(("C:\\root\\f.txt".to_owned(), "f.txt".to_owned()))
        );
        // 根自身与外部路径不产生子键。
        assert_eq!(direct_child_key(root, "C:\\root"), None);
        assert_eq!(direct_child_key(root, "D:\\elsewhere\\x"), None);
    }

    #[test]
    fn 顶层子项部分占用聚合() {
        let mut collected = Collected::default();
        // 真实 walker 会先记录目录本身（占位 is_dir），文件再累加大小。
        collected.note_root_child("C:\\r", "C:\\r\\aaa", 0, true);
        collected.note_root_child("C:\\r", "C:\\r\\bbb", 0, true);
        // aaa 下两个文件（10 + 40），bbb 一个（30），根下文件与空目录各一。
        collected.note_root_child("C:\\r", "C:\\r\\aaa\\x\\f.txt", 10, false);
        collected.note_root_child("C:\\r", "C:\\r\\aaa\\y\\g.txt", 40, false);
        collected.note_root_child("C:\\r", "C:\\r\\bbb\\h.txt", 30, false);
        collected.note_root_child("C:\\r", "C:\\r\\top.bin", 5, false);
        collected.note_root_child("C:\\r", "C:\\r\\empty", 0, true);

        let tops = collected.top_children();
        assert_eq!(tops.len(), 4);
        // 降序：aaa(50) > bbb(30) > top.bin(5) > empty(0)。
        assert_eq!(tops[0].name, "aaa");
        assert_eq!(tops[0].path, "C:\\r\\aaa");
        assert!(tops[0].is_dir);
        assert_eq!(tops[0].size, 50);
        assert_eq!(tops[1].name, "bbb");
        assert_eq!(tops[2].name, "top.bin");
        assert!(!tops[2].is_dir);
        assert_eq!(tops[3].name, "empty");
        assert!(tops[3].is_dir);
    }

    #[test]
    fn 进度帧在遍历中触发() {
        let dir = std::env::temp_dir().join(format!("fff-disk-tick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = dir.join("aaa");
        std::fs::create_dir_all(&a).unwrap();
        // 600 条 > TICK_EVERY(512)：无论线程怎么分片都必有全局帧。
        for index in 0..600 {
            std::fs::write(a.join(format!("a{index}.dat")), [0u8; 10]).unwrap();
        }

        let cancelled = AtomicBool::new(false);
        let mut ticks = Vec::new();
        let result = scan_directory(&dir, &cancelled, |tick| ticks.push(tick)).unwrap();
        assert!(!ticks.is_empty(), "600 个条目至少应触发一次全局 tick");
        // 帧计数单调且不超过最终值（部分快照语义）。
        for pair in ticks.windows(2) {
            assert!(pair[0].files <= pair[1].files);
            assert!(pair[0].bytes <= pair[1].bytes);
        }
        assert!(ticks.last().unwrap().files as usize <= result.file_count);
        assert_eq!(result.file_count, 600);

        std::fs::remove_dir_all(&dir).ok();
    }
}

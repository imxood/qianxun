//! Disk space scanner: parallel walk of an arbitrary directory with
//! streaming progress callbacks and cancellation support.
//!
//! Design goals (qianxun 磁盘扫描页):
//! - any directory can be scanned (drive root, project dir, whatever);
//! - the walk runs on the existing background rayon pool, workers push
//!   discovered entries into a shared buffer, the caller flushes at its own
//!   cadence (qianxun flushes every 100ms) — the scanner never blocks on UI;
//! - after the walk finishes, recursive directory sizes are computed
//!   bottom-up (leaf-first sort) and the full entry tree is assembled;
//! - `cancelled` is a shared `AtomicBool`: the walker checks it per entry and
//!   stops early, returning whatever was collected so far.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::parallelism::BACKGROUND_THREAD_POOL;

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
}

/// Scan `root` recursively. `on_tick` receives `(dirs, files, bytes_so_far)`
/// snapshots of everything discovered so far whenever the walker passes a
/// tick boundary — call it as often as you like (qianxun uses ~100ms).
///
/// Cancellation: set `cancelled` to `true` from any thread; the walk stops
/// at the next entry and the result is marked `cancelled`.
pub fn scan_directory(
    root: &Path,
    cancelled: &AtomicBool,
    on_tick: impl FnMut(u64, u64, u64) + Send,
) -> crate::Result<DiskScanResult> {
    BACKGROUND_THREAD_POOL.install(|| {
        let tick = Mutex::new(on_tick);
        let collected = Mutex::new(Collected::default());

        walk_parallel(root, cancelled, &collected, &tick)?;

        let Collected {
            dirs,
            files,
            file_count,
            dir_count,
        } = collected.into_inner().expect("collected");

        let root_str = root.to_string_lossy().into_owned();
        let root_key = normalize_key(&root_str);
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
            entries: Vec::new(),
            tree_root,
        })
    })
}

/// `ignore`-based parallel walk: skips symlinks (no cycles), collects dirs and
/// files with sizes, and ticks the caller with a monotonic entry counter.
fn walk_parallel(
    root: &Path,
    cancelled: &AtomicBool,
    collected: &Mutex<Collected>,
    tick: &Mutex<impl FnMut(u64, u64, u64) + Send>,
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
        let mut seen: u64 = 0;

        Box::new(move |result| {
            if cancelled.load(Ordering::Relaxed) {
                return ignore::WalkState::Quit;
            }
            let Ok(entry) = result else {
                return ignore::WalkState::Continue;
            };
            let Ok(metadata) = entry.metadata() else {
                return ignore::WalkState::Continue;
            };
            let file_type = match entry.file_type() {
                Some(ft) => ft,
                None => return ignore::WalkState::Continue,
            };
            let path = entry.path().to_path_buf();
            // 扫描根本身不算一个条目（dir_count/树都不含它，见结构体注释）。
            if entry.depth() == 0 {
                return ignore::WalkState::Continue;
            }
            let name = dir_name_of(&path);

            {
                let mut guard = collected.lock().expect("collected");
                if file_type.is_dir() {
                    guard.dirs.push((path, name));
                    guard.dir_count += 1;
                } else if file_type.is_file() {
                    let size = metadata.len();
                    guard.files.push((path, name, size));
                    guard.file_count += 1;
                }
            }

            seen += 1;
            if seen % 512 == 0 {
                let guard = collected.lock().expect("collected");
                (tick.lock().expect("tick"))(
                    guard.file_count as u64,
                    guard.dir_count as u64,
                    guard.files.iter().map(|f| f.2).sum::<u64>(),
                );
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
        let result = scan_directory(&dir, &cancelled, |_, _, _| {}).unwrap();

        assert!(!result.cancelled);
        assert_eq!(result.file_count, 3);
        assert_eq!(result.total_size, 160);
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

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 取消后提前返回() {
        let dir = std::env::temp_dir().join(format!("fff-disk-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.txt"), [0u8; 1]).unwrap();

        let cancelled = AtomicBool::new(true);
        let result = scan_directory(&dir, &cancelled, |_, _, _| {}).unwrap();
        assert!(result.cancelled);

        std::fs::remove_dir_all(&dir).ok();
    }
}

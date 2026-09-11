use crate::RescanReason;
use crate::constants::MAX_OVERFLOW_FILES;
use crate::error::Error;
use crate::shared::SharedFilePicker;
use crate::sort_buffer::sort_with_buffer;
use crate::watch::{RawWatchEvent, WatchEventKind};
use fff_query_parser::FFFMode;
use log::{debug, error, info, warn};
use notify::event::{AccessKind, AccessMode};
use notify::{Config, EventKind, EventKindMask, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, DebouncedEvent, NoCache, new_debouncer_opt};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

type Debouncer = notify_debouncer_full::Debouncer<notify::RecommendedWatcher, NoCache>;

/// Owns the file-system watcher and guarantees that all background threads
/// are fully joined before `stop()` / `Drop` returns.
pub struct BackgroundWatcher {
    debouncer: Arc<Mutex<Option<Debouncer>>>,
    watch_tx: Option<mpsc::Sender<WatchTask>>,
    owner_thread: Option<std::thread::JoinHandle<()>>,
}

enum WatchTask {
    /// Only subscribe to a specific path, this is happening when we did rescun and have to update
    /// the watcher only
    Subscribe(PathBuf),
    /// This is requires a separate walk of the new directory copies or created within a scan
    /// window because it might contain subdirectories we have to walk, prune, and add to index
    IndexNewDir(PathBuf),
}

const DEBOUNCE_TIMEOUT: Duration = Duration::from_millis(50);
/// Minimum seconds between frecency tracks of the same file in AI mode.
/// Prevents score inflation from rapid burst edits by AI agents.
#[allow(dead_code)]
const AI_MODE_COOLDOWN_SECS: u64 = 5 * 60;

impl BackgroundWatcher {
    pub(crate) fn new(
        base_path: PathBuf,
        shared_picker: SharedFilePicker,
        mode: FFFMode,
        enable_fs_root_scanning: bool,
        enable_home_dir_scanning: bool,
        _span: (),
    ) -> Result<Self, Error> {
        info!(
            "Initializing background watcher for path: {}, mode: {:?}",
            base_path.display(),
            mode,
        );

        // by default we do not want to allow users to search their FS root, this is very error prone
        // though some consumers would specifically allow that e.g. unikernels, windows disc
        // partition or sub file systems. By default - fail, unless user permits
        let is_fs_root = base_path.parent().is_none();
        // use rust's path api for maximum reliability of the comparison
        let is_home_dir = Some(&base_path) == dirs::home_dir().as_ref();

        if (is_fs_root && !enable_fs_root_scanning) || (is_home_dir && !enable_home_dir_scanning) {
            return Err(Error::FilesystemRoot(base_path));
        }

        // macOS: always use a single recursive FSEvent stream.
        // Per-dir NonRecursive watches create one FSEvent stream per dir.
        // The per-process FSEvent cap is lower than expected in practice
        // (4096 per process, but FFF usually is running within code editors),
        // and each failed `watch()` after the cap blocks ~40 ms on kernel retry.
        // Yes we pay for filtering events on handler phase but it is usable
        //
        // Windows doesn't seem to have a hard cap, but in practice non recursive watching
        // does a way worse job and often looses events which is not an option for us.
        //
        // Linux keeps the per-dir NonRecursive strategy: inotify has no
        // kernel-level watcher recursion, so we have to manually watch every single interested
        // directory for watch events which is in practice stable and fast if system has enough
        // spare watcher (configurable by the user, usually 100k - 1m)
        let use_recursive = cfg!(any(target_os = "macos", target_os = "windows"));

        let owner_weak_picker = shared_picker.weaken();

        let (watch_tx, watch_rx) = mpsc::channel::<WatchTask>();
        let watch_tx_for_debouncer = watch_tx.clone();

        let debouncer = Self::create_debouncer(
            base_path,
            shared_picker,
            mode,
            use_recursive,
            watch_tx_for_debouncer,
        )?;

        info!("Background file watcher initialized successfully");

        let debouncer = Arc::new(Mutex::new(Some(debouncer)));
        // Only the Linux per-dir-watch branch needs this clone; on other
        // platforms the owner thread never touches the debouncer.
        #[cfg(target_os = "linux")]
        let owner_debouncer = Arc::clone(&debouncer);

        let _owner_span = _span.clone();
        let owner_thread = std::thread::Builder::new()
            .name("fff-watcher-own".into())
            .spawn(move || {
                // No span to enter without otlp feature
                while let Ok(task) = watch_rx.recv() {
                    // if the picker is dropped we do need to exit the loop
                    let Some(strong_picker) = owner_weak_picker.upgrade() else {
                        break;
                    };

                    let (dir, is_new_dir) = match task {
                        WatchTask::Subscribe(dir) => (dir, false),
                        WatchTask::IndexNewDir(dir) => (dir, true),
                    };

                    // Register the watch BEFORE walking so files created mid-walk still handled
                    #[cfg(target_os = "linux")]
                    if !watch_dirs_nonrecursive(&owner_debouncer, std::iter::once(dir.as_path())) {
                        break;
                    }

                    if is_new_dir {
                        // need to call this on every platform to add subdirectories from the
                        // new folders to the picker, but on linux we have to handle the subdirs
                        let subdirs = index_new_directory(&dir, &strong_picker);

                        // on linux we manually resubscribe for new inodes
                        #[cfg(target_os = "linux")]
                        if !watch_dirs_nonrecursive(
                            &owner_debouncer,
                            subdirs.iter().map(|p| p.as_path()),
                        ) {
                            break;
                        }

                        drop(subdirs); // need it cause subdirs is unused on non-linux target'
                    }

                    drop(strong_picker);
                }

                log::info!("Background watcher is stopped");
            })
            .expect("failed to spawn fff-watcher-owner thread");

        Ok(Self {
            debouncer,
            watch_tx: Some(watch_tx),
            owner_thread: Some(owner_thread),
        })
    }

    fn create_debouncer(
        base_path: PathBuf,
        shared_picker: SharedFilePicker,
        mode: FFFMode,
        use_recursive: bool,
        watch_tx: mpsc::Sender<WatchTask>,
    ) -> Result<Debouncer, Error> {
        let config = Config::default()
            .with_follow_symlinks(false)
            // only the actual modification events, ignore the open syscals that we can generate by
            // our own grep calls and preview window rendering
            .with_event_kinds(EventKindMask::CORE);

        let base_path_for_handler = base_path.clone();
        let shared_picker_for_watching = shared_picker.clone();
        let file_picker = shared_picker.weaken();
        let mut debouncer = new_debouncer_opt(
            DEBOUNCE_TIMEOUT,
            Some(DEBOUNCE_TIMEOUT / 2), // tick rate for the event span
            {
                move |result: DebounceEventResult| match result {
                    Ok(events) => {
                        let Some(file_picker) = file_picker.upgrade() else {
                            return;
                        };

                        let new_dirs = handle_debounced_events(
                            mode,
                            events,
                            &base_path_for_handler,
                            &file_picker,
                        );

                        // every new directory created has to be reflected in the picker state
                        for dir in new_dirs {
                            if let Err(e) = watch_tx.send(WatchTask::IndexNewDir(dir)) {
                                error!("Failed to send directory update error: {}", e);
                            }
                        }
                    }
                    Err(errors) => {
                        error!("File watcher errors: {:?}", errors);
                    }
                }
            },
            // There is an issue with recommended cache implementation on macos
            // it keeps track of all the files added to the watcher which is not a problem
            // for us because any rename to the file will anyway require the removing from the
            // ordedred index and adding it back with the new name
            NoCache::new(),
            config,
        )?;

        if use_recursive {
            debouncer.watch(base_path.as_path(), RecursiveMode::Recursive)?;
            info!(
                "File watcher initialized with single recursive watch on {}",
                base_path.display(),
            );
        } else {
            debouncer.watch(base_path.as_path(), RecursiveMode::NonRecursive)?;

            const MAX_CONSECUTIVE_WATCH_FAILURES: usize = 16;

            let mut watched = 0usize;
            let mut consecutive_failures = 0usize;

            // `inotify` is fast-fail: on ENOSPC it returns
            // immediately, no kernel retry loop, so holding this lock is free
            if let Some(guard) = shared_picker_for_watching.read().ok()
                && let Some(picker) = guard.as_ref()
            {
                use std::ops::ControlFlow;
                picker.for_each_dir(|dir| {
                    match debouncer.watch(dir, RecursiveMode::NonRecursive) {
                        Ok(()) => {
                            watched += 1;
                            consecutive_failures = 0;
                            ControlFlow::Continue(())
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            if consecutive_failures <= 4 {
                                warn!("Failed to watch directory {}: {}", dir.display(), e);
                            }

                            if consecutive_failures >= MAX_CONSECUTIVE_WATCH_FAILURES {
                                warn!(
                                    "Giving up setting file watcher for all the directories. Check if your system has enough fs watchers limit. consecutive_failures={}, watched={}",
                                    consecutive_failures, watched
                                );

                                ControlFlow::Break(())
                            } else {
                                ControlFlow::Continue(())
                            }
                        }
                    }
                });
            }

            log::info!(
                "File watcher initialized: watched={}, path={}",
                watched,
                base_path.display()
            );
        }

        Ok(debouncer)
    }

    /// Signal the watcher to shut down without blocking on its worker
    /// threads. Safe to call from any context, including while holding
    /// the [`SharedFilePicker`] write lock.
    pub fn stop(&mut self) {
        self.watch_tx.take();
        if let Some(debouncer) = self.debouncer.lock().take() {
            debouncer.stop_nonblocking();
        }

        self.owner_thread.take();

        info!("Background file watcher stop signaled");
    }

    pub(crate) fn request_watch_dir(&self, dir: PathBuf) -> bool {
        match self.watch_tx.as_ref() {
            Some(tx) => tx.send(WatchTask::Subscribe(dir)).is_ok(),
            None => false,
        }
    }
}

impl Drop for BackgroundWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) fn handle_debounced_events(
    _mode: FFFMode,
    events: Vec<DebouncedEvent>,
    base_path: &Path,
    shared_picker: &SharedFilePicker,
) -> Vec<PathBuf> {
    // this will be called very often, we have to minimiy the lock time for file picker
    // Prefer the walker's own ignore rules (zlob); grab a cheap Arc clone once
    // per batch so we don't hold the picker lock during filtering.
    let walker_rules = shared_picker
        .read()
        .ok()
        .and_then(|g| g.as_ref().and_then(|p| p.ignore_rules()));
    let filter = IgnoreFilter::new(base_path, walker_rules);
    let _need_full_git_rescan = false;
    let mut batch_overflow_attempted = false;
    let mut paths_to_remove = Vec::new();
    let mut dirs_to_remove: Vec<PathBuf> = Vec::new();
    let mut paths_to_add_or_modify = Vec::new();
    let mut new_dirs_to_watch = Vec::new();
    let mut affected_paths_count = 0usize;

    let watch_registry = shared_picker.watch_registry();
    let need_events_propagation = watch_registry.is_active();

    let try_trigger_full_rescan = |reason: RescanReason| -> bool {
        if shared_picker.trigger_full_rescan_with_reason(reason) {
            warn!("Triggering full rescan: {:?}", reason);
            watch_registry.dispatch_rescan(base_path);
            true
        } else {
            false
        }
    };

    for debounced_event in &events {
        // It is very important to not react to the access errors because we inevitably
        // gonna trigger the sync by our own preview or other unnecessary noise
        if matches!(
            debounced_event.event.kind,
            EventKind::Access(
                AccessKind::Read
                    | AccessKind::Open(_)
                    | AccessKind::Close(AccessMode::Read | AccessMode::Execute)
            )
        ) {
            continue;
        }

        // When macOS FSEvents (or other backends) overflow their event buffer, the kernel
        // drops individual events and emits a rescan flag telling us to re-scan the subtree
        if debounced_event.event.need_rescan() {
            let small_and_known = debounced_event.event.paths.len() < 16 // this should be usually one event
                && debounced_event
                    .paths
                    .iter()
                    // but we are smart enough and not falling into the paths
                    .all(|p| !p.is_dir() && !filter.is_ignored(p));

            if !small_and_known && try_trigger_full_rescan(RescanReason::KernelEventLoss) {
                return Vec::new();
            }

            // Small batches and throttled rescans fall through: the listed
            // paths are still applied incrementally below.
        }

        debug!("Processing FS event: {:?}", debounced_event.event);
        for path in &debounced_event.event.paths {
            if matches!(
                path.file_name().and_then(|f| f.to_str()),
                Some(".ignore") | Some(".gitignore")
            ) {
                if path
                    .parent()
                    .is_some_and(|parent| filter.is_ignored(parent))
                {
                    continue;
                }

                info!(
                    "Detected change in ignore definition file: {}",
                    path.display()
                );

                if try_trigger_full_rescan(RescanReason::IgnoreFileChanged) {
                    return Vec::new();
                }

                // Throttled: fall through so the ignore file itself stays
                // indexed; the stale rules heal on the next admitted rescan.
            }

            if is_git_file(path) {
                continue;
            }

            // Use a combination of event kind and filesystem state to decide
            // whether a path is an addition/modification or a removal.
            //
            // We cannot rely on `path.exists()` alone because:
            //   - A freshly created file might not be visible yet (race).
            //   - macOS FSEvents uses Modify(Name(Any)) for both rename-in
            //     and rename-out, so we must stat the path to disambiguate.
            //
            // We cannot rely on event kind alone because:
            //   - Remove events are not always emitted (macOS often sends
            //     Modify(Name(Any)) instead of Remove).
            let is_removal = matches!(debounced_event.event.kind, EventKind::Remove(_));

            // Directory-level remove: both fsevents and inotify delivers a single
            // `Remove(Folder)` event for a whole directory tree (e.g.
            // after `git reset --hard` wipes a dir full of staged-but-
            // uncommitted files).
            let is_folder_removal = matches!(
                debounced_event.event.kind,
                EventKind::Remove(notify::event::RemoveKind::Folder)
            );

            let is_removed = is_folder_removal || is_removal || !path.exists();

            let (is_dir, is_ignored) = if is_removed {
                (false, true)
            } else {
                (path.is_dir(), filter.is_ignored(path))
            };

            if is_folder_removal {
                dirs_to_remove.push(path.to_path_buf());
            } else if is_removed {
                // best effort but doesn't require a stat and generally correct
                let maybe_directory = !matches!(
                    debounced_event.event.kind,
                    EventKind::Remove(notify::event::RemoveKind::File)
                );

                paths_to_remove.push((path.as_path(), maybe_directory));
            } else if is_dir {
                if !is_ignored {
                    new_dirs_to_watch.push(path.to_path_buf());
                }
            } else if !is_ignored {
                // For additions/modifications, still filter gitignored files.
                paths_to_add_or_modify.push(path.as_path());
            }
        }

        affected_paths_count += debounced_event.event.paths.len();
        if !batch_overflow_attempted && affected_paths_count > MAX_OVERFLOW_FILES * 4 {
            batch_overflow_attempted = true;
            warn!(
                "Too many affected paths in a single batch, triggering full rescan: affected={}, max={}",
                affected_paths_count,
                MAX_OVERFLOW_FILES * 4
            );

            if try_trigger_full_rescan(RescanReason::EventBatchOverflow) {
                return Vec::new();
            }
        }
    }

    // It's important to get the allocated sort
    sort_with_buffer(paths_to_add_or_modify.as_mut_slice(), |a, b| {
        a.as_os_str().cmp(b.as_os_str())
    });
    paths_to_add_or_modify.dedup_by(|a, b| a.as_os_str().eq(b.as_os_str()));

    info!(
        "Event processing summary: {} to remove, {} dirs to remove, {} to add/modify, {} new dirs",
        paths_to_remove.len(),
        dirs_to_remove.len(),
        paths_to_add_or_modify.len(),
        new_dirs_to_watch.len()
    );

    if paths_to_remove.is_empty() && dirs_to_remove.is_empty() && paths_to_add_or_modify.is_empty()
    {
        debug!("No file index changes to apply");
        return new_dirs_to_watch;
    }

    let mut index_update_rejected = false;
    let mut overflow_count = 0;
    let mut removed_from_dirs = Vec::new();
    let mut watch_events = ahash::AHashMap::new();

    if !paths_to_remove.is_empty()
        || !dirs_to_remove.is_empty()
        || !paths_to_add_or_modify.is_empty()
    {
        debug!(
            "Applying file index changes: {} to remove, {} dirs to remove, {} to add/modify",
            paths_to_remove.len(),
            dirs_to_remove.len(),
            paths_to_add_or_modify.len(),
        );

        let Ok(mut guard) = shared_picker.write() else {
            error!("Failed to acquire file picker write lock");
            return new_dirs_to_watch;
        };
        let Some(ref mut picker) = *guard else {
            error!("File picker not initialized");
            return new_dirs_to_watch;
        };

        for (path, may_be_dir) in &paths_to_remove {
            let removed = picker.remove_file_by_path(path);

            if removed {
                if need_events_propagation {
                    watch_events.insert(path.to_path_buf(), WatchEventKind::Removed);
                }
            } else if *may_be_dir {
                // Not an indexed file: likely a dir renamed out of the tree
                // (no Remove(Folder) is emitted), expand it per indexed file.
                dirs_to_remove.push(path.to_path_buf());
            }
        }

        // Single index scan for all dirs; misses (never-indexed paths) are free.
        dirs_to_remove.sort_unstable();
        dirs_to_remove.dedup();
        if !dirs_to_remove.is_empty() {
            let dirs = dirs_to_remove.iter().map(PathBuf::as_path);
            if need_events_propagation {
                picker.remove_all_files_in_dirs_with_callback(dirs, |path| {
                    removed_from_dirs.push(path.to_path_buf());
                })
            } else {
                picker.remove_all_files_in_dirs(dirs)
            };
        }

        if need_events_propagation {
            for path in removed_from_dirs.drain(..) {
                watch_events.insert(path, WatchEventKind::Removed);
            }
        }

        for path in &paths_to_add_or_modify {
            if picker.get_overflow_files().len() >= MAX_OVERFLOW_FILES
                && picker.get_file_by_path(path).is_none()
            {
                index_update_rejected = true;
                break;
            }

            let existed = need_events_propagation && picker.get_file_by_path(path).is_some();

            if picker.handle_create_or_modify(path).is_some() {
                if need_events_propagation {
                    let kind = if existed {
                        WatchEventKind::Modified
                    } else {
                        WatchEventKind::Created
                    };

                    watch_events.insert(path.to_path_buf(), kind);
                }
            } else {
                index_update_rejected = true;
            }
        }

        overflow_count = picker.get_overflow_files().len();
    }

    info!(
        "File index changes applied: overflow_count={}",
        overflow_count
    );

    let rescan_started = if index_update_rejected || overflow_count > MAX_OVERFLOW_FILES {
        let reason = if index_update_rejected {
            RescanReason::IndexUpdateRejected
        } else {
            RescanReason::OverflowCapacity
        };

        info!(
            "Watcher faced limit of index overflow. Triggering rescan: {:?}",
            reason
        );
        try_trigger_full_rescan(reason)
    } else {
        false
    };

    // When the rescan is throttled the incrementally applied changes are
    // still the freshest state we have — propagate them to subscribers.
    if !rescan_started && need_events_propagation {
        watch_registry.dispatch(
            base_path,
            watch_events
                .into_iter()
                .map(|(path, kind)| RawWatchEvent {
                    path,
                    kind,
                    is_ignored: false,
                })
                .collect(),
        );
    }

    new_dirs_to_watch
}

fn index_new_directory(dir: &Path, shared_picker: &SharedFilePicker) -> Vec<PathBuf> {
    // Prefer the walker's ignore rules; read base_path + rules from the picker.
    let (base_path, walker_rules, follow_symlinks) = match shared_picker.read().ok().and_then(|g| {
        g.as_ref().map(|p| {
            (
                p.base_path().to_path_buf(),
                p.ignore_rules(),
                p.follows_symlinks(),
            )
        })
    }) {
        Some(triple) => triple,
        None => return Vec::new(),
    };

    let walk = match crate::walk::walk_collect_files(
        dir,
        false,
        follow_symlinks,
        1,
        &Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    ) {
        Ok(walk) => walk,
        Err(e) => {
            warn!("Failed to walk new directory {:?}: {}", dir.display(), e);
            return Vec::new();
        }
    };

    // TODO: figure out a better optimized way for zlob to rerun the directory walk using existing
    // ignore rules, but currently we have to filter out ignored files on our own
    let filter = IgnoreFilter::new(&base_path, walker_rules);
    let join_unless_ignored = |relative_path: &str| -> Option<PathBuf> {
        let path = dir.join(relative_path);
        (!filter.is_ignored(&path)).then_some(path)
    };

    let files_to_add: Vec<PathBuf> = walk
        .pairs
        .iter()
        .filter_map(|(_, path)| join_unless_ignored(path))
        .collect();

    let subdirs: Vec<PathBuf> = walk
        .dirs
        .iter()
        .filter_map(|path| join_unless_ignored(path.trim_end_matches('/')))
        .collect();

    if files_to_add.is_empty() {
        return subdirs;
    }

    let mut indexed_files = Vec::with_capacity(files_to_add.len());
    {
        let Ok(mut guard) = shared_picker.write() else {
            return subdirs;
        };

        let Some(ref mut picker) = *guard else {
            return subdirs;
        };

        for path in files_to_add {
            if picker.handle_create_or_modify(&path).is_some() {
                indexed_files.push(path);
            }
        }
    }
    let added = indexed_files.len();

    let watch_registry = shared_picker.watch_registry();
    if watch_registry.is_active() {
        let events = indexed_files
            .iter()
            .map(|path| RawWatchEvent {
                path: path.clone(),
                kind: WatchEventKind::Created,
                is_ignored: false,
            })
            .collect();

        watch_registry.dispatch(&base_path, events);
    }

    debug!(
        "Indexed new {} files from new directory {}",
        added,
        dir.display(),
    );

    subdirs
}

#[cfg(target_os = "linux")]
fn watch_dirs_nonrecursive<'a>(
    debouncer: &Mutex<Option<Debouncer>>,
    dirs: impl Iterator<Item = &'a Path>,
) -> bool {
    let mut guard = debouncer.lock();
    let Some(debouncer) = guard.as_mut() else {
        return false;
    };

    for dir in dirs {
        if let Err(e) = debouncer.watch(dir, RecursiveMode::NonRecursive) {
            warn!(
                ?e,
                dir = %dir.display(),
                "Failed to init watcher for new directory"
            );
        }
    }

    true
}

struct IgnoreFilter<'a> {
    base_path: &'a Path,
    /// Reusable ignore rules from the last walk.
    rules: Option<Arc<crate::walk::WalkIgnoreRules>>,
}

impl<'a> IgnoreFilter<'a> {
    fn new(base_path: &'a Path, rules: Option<Arc<crate::walk::WalkIgnoreRules>>) -> Self {
        Self { base_path, rules }
    }

    /// Whether `path` (absolute) is ignored.
    fn is_ignored(&self, path: &Path) -> bool {
        if let Some(rules) = self.rules.as_ref() {
            let Ok(relative) = path.strip_prefix(self.base_path) else {
                return false;
            };
            // `IgnoreRules::is_ignored` enumerates every ancestor .gitignore
            // layer internally, so a leaf under an ignored directory (rule
            // `build/`, path `build/out.rs`) is caught in one call.
            return rules.is_ignored(relative);
        }
        // No rules: the non-code-dir heuristic, applied to the base-relative
        // path so ancestors of the base (e.g. a temp dir under AppData/Local
        // on Windows) never match.
        crate::ignore::is_non_code_directory(path.strip_prefix(self.base_path).unwrap_or(path))
    }
}

#[inline]
pub(crate) fn is_git_file(path: &Path) -> bool {
    // it could be in submodule
    path.components()
        .any(|component| component.as_os_str() == ".git")
}

#[allow(dead_code)]
fn is_dotgit_change_affecting_status(changed: &Path) -> bool {
    let git_dir = changed.parent().and_then(|p| p.parent()).and_then(|p| {
        let components: Vec<_> = p.components().collect();
        if components.len() >= 2
            && components
                .last()
                .map(|c| c.as_os_str() == ".git")
                .unwrap_or(false)
        {
            Some(p)
        } else {
            None
        }
    });

    if let Some(git_parent) = git_dir {
        if let Ok(path_in_git_dir) = changed.strip_prefix(git_parent) {
            // Only react to changes that rewrite the worktree state: commits,
            // staging, checkouts, merges, conflict resolution.
            if path_in_git_dir == Path::new("index") || path_in_git_dir == Path::new("index.lock") {
                return true;
            }

            if path_in_git_dir == Path::new("HEAD") {
                return true;
            }

            // some of the git ops are not involving neither index nor HEAD change, or sometimes
            // index updates can arrive too late after the change - that's why we track the log
            // the actual user action, once user
            if path_in_git_dir == Path::new("logs/HEAD") {
                return true;
            }

            if let Some(fname) = path_in_git_dir.file_name().and_then(|f| f.to_str())
                && matches!(fname, "MERGE_HEAD" | "CHERRY_PICK_HEAD" | "REVERT_HEAD")
            {
                return true;
            }
        }
    }

    false
}

#[allow(dead_code)]
fn watch_git_status_paths(_debouncer: &mut Debouncer) {
    // Git status watching removed - not needed for qianxun
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_picker::{FilePicker, FilePickerOptions};
    use crate::watch::{WatchEvent, WatchOptions};
    use notify::Event;
    use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn replacement_batch_emits_one_modified_event() {
        let tmp = tempfile::tempdir().unwrap();
        let base = crate::path_utils::canonicalize(tmp.path()).unwrap();
        let path = base.join("file.txt");
        std::fs::write(&path, "before").unwrap();

        let shared_picker = SharedFilePicker::default();
        let mut picker = FilePicker::new(FilePickerOptions {
            base_path: base.to_string_lossy().into_owned(),
            watch: false,
            ..Default::default()
        })
        .unwrap();
        picker.collect_files().unwrap();
        shared_picker.rebase_watches(&base);
        *shared_picker.write().unwrap() = Some(picker);

        let (sender, receiver) = mpsc::channel::<Vec<WatchEvent>>();
        shared_picker
            .watch_registry()
            .subscribe(
                &base,
                "**",
                WatchOptions::default(),
                Box::new(move |_, events| sender.send(events.to_vec()).unwrap()),
            )
            .unwrap();

        std::fs::write(&path, "after").unwrap();
        let now = Instant::now();
        let events = vec![
            DebouncedEvent::new(
                Event::new(EventKind::Remove(RemoveKind::File)).add_path(path.clone()),
                now,
            ),
            DebouncedEvent::new(
                Event::new(EventKind::Create(CreateKind::File)).add_path(path.clone()),
                now,
            ),
            DebouncedEvent::new(
                Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
                    .add_path(path.clone()),
                now,
            ),
        ];

        handle_debounced_events(FFFMode::Neovim, events, &base, &shared_picker);

        let received = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].path, path);
        assert_eq!(received[0].kind, WatchEventKind::Modified);
    }

    #[test]
    fn git_file_path_detection() {
        assert!(is_git_file(&PathBuf::from(".git/config")));
        assert!(is_git_file(&PathBuf::from(
            "submodule/.git/hooks/pre-commit"
        )));
        assert!(!is_git_file(&PathBuf::from("src/main.rs")));
        assert!(!is_git_file(&PathBuf::from(".github/workflows/ci.yml")));
    }
}

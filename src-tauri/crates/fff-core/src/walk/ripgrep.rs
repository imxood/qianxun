use crate::types::FileItem;
use crate::walk::WalkOutput;
use crate::watch::is_git_file;
use globset::GlobSet;
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use log::debug;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Directories that should be excluded from index in non-git repos.
const IGNORED_DIRS: &[&str] = &[
    ".cache",
    ".cargo",
    ".local",
    ".gradle",
    ".vscode",
    ".idea",
    ".rustup",
    "Library",
    "go/pkg",
    "node_modules",
    "Program Files",
    "Program Files (x86)",
    #[cfg(target_os = "windows")]
    "AppData/Local",
    #[cfg(target_os = "windows")]
    "AppData/Roaming",
];

fn build_non_git_overrides(base_path: &Path) -> Option<ignore::overrides::Override> {
    let mut builder = OverrideBuilder::new(base_path);
    for dir in IGNORED_DIRS {
        let pattern = format!("!**/{dir}/");
        if let Err(e) = builder.add(&pattern) {
            log::warn!("failed to add ignore pattern {pattern}: {e}");
        }
    }
    builder.build().ok()
}

pub(crate) struct WalkIgnoreRules {
    set: GlobSet,
    base_path: PathBuf,
}

impl WalkIgnoreRules {
    pub(crate) fn is_ignored(&self, relative_path: &Path) -> bool {
        if let Ok(stripped) = relative_path.strip_prefix(&self.base_path) {
            return self.set.is_match(stripped);
        }
        self.set.is_match(relative_path)
    }
}

pub(crate) fn walk_collect_files(
    base_path: &Path,
    is_git_repo: bool,
    follow_symlinks: bool,
    threads: usize,
    synced_files_count: &Arc<AtomicUsize>,
) -> crate::Result<WalkOutput> {
    debug!(
        "Walking filesystem: path={}, is_git_repo={}, follow_symlinks={}, threads={}",
        base_path.display(),
        is_git_repo,
        follow_symlinks,
        threads
    );

    let mut walk_builder = WalkBuilder::new(base_path);
    walk_builder
        // this is a very important guard for the user opening ~/ or other root non-git dir
        .hidden(!is_git_repo)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .ignore(true)
        .follow_links(follow_symlinks)
        .threads(threads);

    if !is_git_repo {
        if let Some(overrides) = build_non_git_overrides(base_path) {
            walk_builder.overrides(overrides);
        }
    }

    let walker = walk_builder.build_parallel();

    // Single lock for both collections: every entry is either a file or a
    // dir, so this keeps one mutex acquisition per entry.
    let collected =
        parking_lot::Mutex::new((Vec::<(FileItem, String)>::new(), Vec::<String>::new()));
    walker.run(|| {
        let collected = &collected;
        let counter = Arc::clone(synced_files_count);
        let base_path = base_path.to_path_buf();

        Box::new(move |result| {
            let Ok(entry) = result else {
                return ignore::WalkState::Continue;
            };

            if entry.file_type().is_some_and(|ft| ft.is_file()) {
                let path = entry.path();

                // Ignore walkers sometimes surface files inside `.git/`
                // when the base is itself a git repo — skip them.
                if is_git_file(path) {
                    return ignore::WalkState::Continue;
                }

                let metadata = entry.metadata().ok();
                let (file_item, rel_path) =
                    FileItem::new_from_walk(path, &base_path, metadata.as_ref());

                collected.lock().0.push((file_item, rel_path));
                counter.fetch_add(1, Ordering::Relaxed);
            } else if entry.depth() > 0 && entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let path = entry.path();
                if !is_git_file(path)
                    && let Ok(rel) = path.strip_prefix(&base_path)
                {
                    let mut rel = crate::path_utils::to_canonical_slashes(&rel.to_string_lossy())
                        .into_owned();
                    rel.push('/');
                    collected.lock().1.push(rel);
                }
            }
            ignore::WalkState::Continue
        })
    });

    let (pairs, dirs) = collected.into_inner();
    Ok(WalkOutput {
        pairs,
        dirs,
        ignore_rules: None,
    })
}

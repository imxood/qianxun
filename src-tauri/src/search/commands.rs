//! 搜索域 IPC 命令：开根、状态、文件名搜索、内容搜索、取消。

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use fff_search::{
    FFFMode, FilePicker, FilePickerOptions, FuzzySearchOptions, GrepMode, GrepSearchOptions,
    PaginationArgs, QueryParser, SharedFrecency,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::error::{Error, Result};

/// 内容搜索的默认每页上限与单文件上限（够用的经验值）。
const GREP_PAGE_LIMIT: usize = 200;
const GREP_MAX_PER_FILE: usize = 100;
const GREP_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
/// fff 的 grep 有内部时间预算；给到 3s，长目录靠分页游标继续。
const GREP_TIME_BUDGET_MS: u64 = 3000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOpen {
    pub root: String,
    pub generation: u64,
    /// 同根复用时为 false（不触发重建）。
    pub rebuilt: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchStatus {
    pub root: Option<String>,
    pub generation: u64,
    pub scanning: bool,
    pub watcher_ready: bool,
    pub files: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub path: String,
    pub score: i32,
    /// 文件名内的匹配区间（字节偏移，前端按 UTF-8 切片高亮）。
    pub offsets: Vec<(u32, u32)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesPage {
    pub items: Vec<FileHit>,
    pub total_matched: usize,
    pub total_files: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepHit {
    pub path: String,
    pub line_number: u64,
    pub col: usize,
    pub line_content: String,
    /// 行内匹配区间（字节偏移）。
    pub offsets: Vec<(u32, u32)>,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepPage {
    pub items: Vec<GrepHit>,
    pub files_searched: usize,
    pub files_with_matches: usize,
    pub next_file_offset: usize,
    pub aborted: bool,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct GrepOptions {
    pub regex: bool,
    pub smart_case: bool,
    pub before_context: usize,
    pub after_context: usize,
}

/// 打开（或切换）搜索根目录：重建索引并立即返回，进度走轮询。
#[tauri::command]
pub fn search_open(root: String, state: State<'_, crate::AppState>) -> Result<SearchOpen> {
    let canonical = normalize_root(Path::new(&root))?;
    let search = &state.search;
    if let Some(current) = search.root() {
        if current == canonical {
            return Ok(SearchOpen {
                root: current.to_string_lossy().into_owned(),
                generation: search.generation(),
                rebuilt: false,
            });
        }
    }

    // 换根：作废旧搜索，重建 picker（后台扫描 + watcher）。
    search.cancel_search();
    let generation = search.bump_generation();
    FilePicker::new_with_shared_state(
        search.picker().clone(),
        SharedFrecency::default(),
        FilePickerOptions {
            base_path: canonical.to_string_lossy().into_owned(),
            mode: FFFMode::Ai,
            watch: true,
            follow_symlinks: false,
            enable_fs_root_scanning: false,
            enable_home_dir_scanning: false,
            enable_mmap_cache: false,
            enable_content_indexing: false,
            cache_budget: None,
        },
    )
    .map_err(|cause| Error::Search(format!("无法索引 {}：{cause}", canonical.display())))?;
    search.set_root(Some(canonical.clone()));
    Ok(SearchOpen {
        root: canonical.to_string_lossy().into_owned(),
        generation,
        rebuilt: true,
    })
}

/// 索引状态（前端轮询：扫描中每 ~300ms）。
#[tauri::command]
pub fn search_status(state: State<'_, crate::AppState>) -> Result<SearchStatus> {
    let search = &state.search;
    let guard = search
        .picker()
        .read()
        .map_err(|cause| Error::Search(cause.to_string()))?;
    Ok(match guard.as_ref() {
        Some(picker) => {
            let progress = picker.get_scan_progress();
            SearchStatus {
                root: search.root().map(|p| p.to_string_lossy().into_owned()),
                generation: search.generation(),
                scanning: progress.is_scanning,
                watcher_ready: progress.is_watcher_ready,
                files: picker.live_file_count(),
            }
        }
        None => SearchStatus {
            root: None,
            generation: search.generation(),
            scanning: false,
            watcher_ready: false,
            files: 0,
        },
    })
}

/// 文件名 fuzzy 搜索（索引内存操作，毫秒级，直接同步返回）。
#[tauri::command]
pub fn search_files(
    query: String,
    limit: Option<usize>,
    offset: Option<usize>,
    state: State<'_, crate::AppState>,
) -> Result<FilesPage> {
    if query.trim().is_empty() {
        return Ok(FilesPage {
            items: Vec::new(),
            total_matched: 0,
            total_files: 0,
        });
    }
    let guard = state
        .search
        .picker()
        .read()
        .map_err(|cause| Error::Search(cause.to_string()))?;
    let picker = guard
        .as_ref()
        .ok_or_else(|| Error::Search("尚未选择搜索根目录".to_owned()))?;
    let parsed = QueryParser::default().parse(&query);
    let result = picker.fuzzy_search(
        &parsed,
        None,
        FuzzySearchOptions {
            max_threads: 0,
            current_file: None,
            project_path: None,
            combo_boost_score_multiplier: 0,
            min_combo_count: 0,
            pagination: PaginationArgs {
                offset: offset.unwrap_or(0),
                limit: limit.unwrap_or(100).clamp(1, 500),
            },
        },
    );
    let items = result
        .items
        .iter()
        .zip(result.scores.iter())
        .zip(result.match_byte_offsets.iter())
        .map(|((item, score), offsets)| FileHit {
            path: item.relative_path(picker),
            score: score.total,
            offsets: offsets.iter().map(|(a, b)| (*a, *b)).collect(),
        })
        .collect();
    Ok(FilesPage {
        items,
        total_matched: result.total_matched,
        total_files: result.total_files,
    })
}

/// 内容搜索。新调用自动取消上一次（取消令牌在 SearchState 接力）。
/// 大目录靠 nextFileOffset 分页继续，UI 传 fileOffset 翻页。
#[tauri::command]
pub fn search_content(
    query: String,
    opts: Option<GrepOptions>,
    file_offset: Option<usize>,
    state: State<'_, crate::AppState>,
) -> Result<GrepPage> {
    if query.trim().is_empty() {
        return Ok(GrepPage {
            items: Vec::new(),
            files_searched: 0,
            files_with_matches: 0,
            next_file_offset: 0,
            aborted: false,
        });
    }
    let opts = opts.unwrap_or_default();
    let token = state.search.rotate_abort();
    let guard = state
        .search
        .picker()
        .read()
        .map_err(|cause| Error::Search(cause.to_string()))?;
    let picker = guard
        .as_ref()
        .ok_or_else(|| Error::Search("尚未选择搜索根目录".to_owned()))?;

    let parsed = QueryParser::default().parse(&query);
    let result = picker.grep(
        &parsed,
        &GrepSearchOptions {
            max_file_size: GREP_MAX_FILE_SIZE,
            max_matches_per_file: GREP_MAX_PER_FILE,
            smart_case: opts.smart_case,
            file_offset: file_offset.unwrap_or(0),
            page_limit: GREP_PAGE_LIMIT,
            mode: if opts.regex {
                GrepMode::Regex
            } else {
                GrepMode::PlainText
            },
            time_budget_ms: GREP_TIME_BUDGET_MS,
            before_context: opts.before_context.min(10),
            after_context: opts.after_context.min(10),
            classify_definitions: false,
            trim_whitespace: false,
            abort_signal: Some(Arc::clone(&token)),
        },
    );

    let aborted = token.load(Ordering::Relaxed);
    let items = result
        .matches
        .iter()
        .map(|hit| {
            let file = result
                .files
                .get(hit.file_index)
                .map(|item| item.relative_path(picker))
                .unwrap_or_default();
            GrepHit {
                path: file,
                line_number: hit.line_number,
                col: hit.col,
                line_content: hit.line_content.clone(),
                offsets: hit
                    .match_byte_offsets
                    .iter()
                    .map(|(a, b)| (*a, *b))
                    .collect(),
                context_before: hit.context_before.clone(),
                context_after: hit.context_after.clone(),
            }
        })
        .collect();
    Ok(GrepPage {
        items,
        files_searched: result.total_files_searched,
        files_with_matches: result.files_with_matches,
        next_file_offset: result.next_file_offset,
        aborted,
    })
}

/// 取消进行中的内容搜索。
#[tauri::command]
pub fn search_cancel(state: State<'_, crate::AppState>) -> Result<()> {
    state.search.cancel_search();
    Ok(())
}

/// 等待扫描完成（供集成测试；UI 走轮询）。
#[tauri::command]
pub async fn search_wait_ready(app: AppHandle, timeout_ms: Option<u64>) -> Result<bool> {
    let shared = app.state::<crate::AppState>().search.picker().clone();
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(10_000));
    Ok(shared.wait_for_scan(timeout))
}

/// 规范化根目录：必须存在且是目录。
fn normalize_root(root: &Path) -> Result<PathBuf> {
    if !root.is_dir() {
        return Err(Error::Search(format!("目录不存在：{}", root.display())));
    }
    root.canonicalize()
        .map_err(|cause| Error::Search(format!("无法解析路径 {}: {cause}", root.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 根目录必须是存在的目录() {
        assert!(normalize_root(Path::new("Z:/definitely-not-exist")).is_err());
        let dir = std::env::temp_dir();
        assert!(normalize_root(&dir).is_ok());
    }

    /// fff 调用形态的真实验证：索引临时目录后 fuzzy 与 grep 都要命中，
    /// 且字段语义（相对路径 / 行号 / 上下文）符合我们的假设。
    #[test]
    fn 索引后文件名与内容搜索都能命中() {
        use fff_search::SharedFilePicker;

        let root = std::env::temp_dir().join(format!("qx-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("建目录");
        std::fs::write(
            root.join("src/hello_world.rs"),
            "fn main() {\n    let greeting = \"你好 search\";\n    println!(\"{greeting}\");\n}\n",
        )
        .expect("写文件");
        std::fs::write(root.join("src/other.txt"), "nothing here\n").expect("写文件2");

        let shared = SharedFilePicker::default();
        FilePicker::new_with_shared_state(
            shared.clone(),
            SharedFrecency::default(),
            FilePickerOptions {
                base_path: root.to_string_lossy().into_owned(),
                mode: FFFMode::Ai,
                watch: false,
                follow_symlinks: false,
                enable_fs_root_scanning: false,
                enable_home_dir_scanning: false,
                enable_mmap_cache: false,
                enable_content_indexing: false,
                cache_budget: None,
            },
        )
        .expect("建索引");
        assert!(
            shared.wait_for_scan(Duration::from_secs(20)),
            "扫描应在时限内完成"
        );

        // 文件名 fuzzy：hello_world.rs 必须以相对路径形态命中。
        {
            let guard = shared.read().expect("读");
            let picker = guard.as_ref().expect("picker");
            let query = QueryParser::default().parse("hello");
            let result = picker.fuzzy_search(
                &query,
                None,
                FuzzySearchOptions {
                    max_threads: 0,
                    current_file: None,
                    project_path: None,
                    combo_boost_score_multiplier: 0,
                    min_combo_count: 0,
                    pagination: PaginationArgs {
                        offset: 0,
                        limit: 20,
                    },
                },
            );
            let paths: Vec<String> = result
                .items
                .iter()
                .map(|item| item.relative_path(picker))
                .collect();
            assert!(
                paths
                    .iter()
                    .any(|path| path.replace('\\', "/") == "src/hello_world.rs"),
                "fuzzy 命中列表：{paths:?}"
            );
        }

        // 内容 grep：固定串命中行号与上下文。
        {
            let guard = shared.read().expect("读");
            let picker = guard.as_ref().expect("picker");
            let query = QueryParser::default().parse("greeting");
            let result = picker.grep(
                &query,
                &GrepSearchOptions {
                    max_file_size: 1024 * 1024,
                    max_matches_per_file: 10,
                    smart_case: true,
                    file_offset: 0,
                    page_limit: 50,
                    mode: GrepMode::PlainText,
                    time_budget_ms: 2000,
                    before_context: 1,
                    after_context: 1,
                    classify_definitions: false,
                    trim_whitespace: false,
                    abort_signal: Some(Arc::new(std::sync::atomic::AtomicBool::new(false))),
                },
            );
            assert!(!result.matches.is_empty(), "grep 应有命中");
            let hit = &result.matches[0];
            assert_eq!(hit.line_number, 2, "greeting 在第 2 行");
            assert_eq!(hit.context_before.len(), 1);
            assert!(hit.context_before[0].contains("fn main"));
            let path = result
                .files
                .get(hit.file_index)
                .map(|item| item.relative_path(picker))
                .unwrap_or_default();
            assert!(
                path.replace('\\', "/").ends_with("hello_world.rs"),
                "命中文件：{path}"
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    /// 性能靶场：自造 500×200=10 万文件的目录树（fff 内置忽略 node_modules，
    /// 不能拿真实依赖树当靶场），建索引并测 fuzzy 首屏耗时。
    /// 默认忽略，验收时 `cargo test -- --ignored`。
    #[test]
    #[ignore = "真实大目录（生成 10 万文件 + 索引），约 1-2 分钟"]
    fn 大目录索引与首屏性能() {
        use fff_search::SharedFilePicker;

        const GROUPS: usize = 500;
        const PER_GROUP: usize = 200;

        let root = std::env::temp_dir().join(format!("qx-bench-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let setup_started = std::time::Instant::now();
        for group in 0..GROUPS {
            let dir = root.join(format!("pkg-{group:03}/src"));
            std::fs::create_dir_all(&dir).expect("建目录");
            for file in 0..PER_GROUP {
                let name = if file % 7 == 0 {
                    format!("svelte_module_{group:03}_{file:03}.ts")
                } else {
                    format!("mod_{group:03}_{file:03}.ts")
                };
                std::fs::write(dir.join(name), "export const value = 1;\n").expect("写文件");
            }
        }
        println!(
            "生成 {} 个文件耗时 {:.2?}",
            GROUPS * PER_GROUP,
            setup_started.elapsed()
        );

        let started = std::time::Instant::now();
        let shared = SharedFilePicker::default();
        FilePicker::new_with_shared_state(
            shared.clone(),
            SharedFrecency::default(),
            FilePickerOptions {
                base_path: root.to_string_lossy().into_owned(),
                mode: FFFMode::Ai,
                watch: false,
                follow_symlinks: false,
                enable_fs_root_scanning: false,
                enable_home_dir_scanning: false,
                enable_mmap_cache: false,
                enable_content_indexing: false,
                cache_budget: None,
            },
        )
        .expect("建索引");
        assert!(
            shared.wait_for_scan(Duration::from_secs(180)),
            "扫描超时（180s）"
        );
        let files = {
            let guard = shared.read().expect("读");
            guard.as_ref().expect("picker").live_file_count()
        };
        println!("索引 {files} 个文件耗时 {:.2?}", started.elapsed());
        assert!(files >= GROUPS * PER_GROUP, "应至少索引到全部生成文件");

        // fuzzy 首屏：索引就绪后的内存操作。
        let query_started = std::time::Instant::now();
        {
            let guard = shared.read().expect("读");
            let picker = guard.as_ref().expect("picker");
            let query = QueryParser::default().parse("svelte");
            let result = picker.fuzzy_search(
                &query,
                None,
                FuzzySearchOptions {
                    max_threads: 0,
                    current_file: None,
                    project_path: None,
                    combo_boost_score_multiplier: 0,
                    min_combo_count: 0,
                    pagination: PaginationArgs {
                        offset: 0,
                        limit: 100,
                    },
                },
            );
            println!(
                "fuzzy 首屏命中 {} 条，耗时 {:.3?}",
                result.total_matched,
                query_started.elapsed()
            );
            assert!(result.total_matched > 0);
        }
        // 验收线：首屏 < 1s（架构 §4.9）。
        assert!(
            query_started.elapsed().as_millis() < 1000,
            "fuzzy 首屏超时：{query_started:?}"
        );

        let _ = std::fs::remove_dir_all(root);
    }
}

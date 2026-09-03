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

/// 内容搜索的每个分片：短时间预算既给引擎内的 abort 检查留粒度，
/// 又让「新搜索顶掉旧搜索」在分片边界即时生效；循环推进 file_offset
/// 直到搜完或被取消——对前端表现为流式（每分片经 Channel 推送）。
const GREP_CHUNK_LIMIT: usize = 100;
const GREP_CHUNK_BUDGET_MS: u64 = 800;
const GREP_MAX_PER_FILE: usize = 100;
const GREP_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
/// 单次搜索的命中总量上限：超大目录请缩小范围/glob 过滤（提示由前端给出）。
const GREP_MAX_TOTAL_ITEMS: usize = 2000;

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
    /// 大小（字节）与修改时间（毫秒）——列表排序列用；stat 失败记 0。
    pub size: u64,
    pub mtime: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesPage {
    pub items: Vec<FileHit>,
    pub total_matched: usize,
    pub total_files: usize,
}

#[derive(Serialize, Clone)]
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
    /// 0 = 已搜完；非 0 = 因取消中断（流式循环在分片边界推进，不再需要
    /// 前端手动翻页）。
    pub next_file_offset: usize,
    pub aborted: bool,
}

/// 流式分片（经 Tauri Channel 推送）：items 为本分片新命中。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GrepProgress {
    pub items: Vec<GrepHit>,
    pub files_searched: usize,
    pub files_with_matches: usize,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct GrepOptions {
    pub regex: bool,
    pub smart_case: bool,
    pub before_context: usize,
    pub after_context: usize,
    /// 文件名 glob 过滤（如 `*.rs`、`src/**`）：不含 `/` 时按文件名匹配，
    /// 含 `/` 时按相对路径匹配（封装层逐分片过滤，引擎无此旋钮）。
    pub glob: Option<String>,
}

/// 一个逻辑盘（搜索根选择器的盘符胶囊行）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    /// 形如 `C:\` 的根路径。
    pub path: String,
    /// fixed | removable | network | cdrom | ramdisk | unknown
    pub kind: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
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
        .map(|((item, score), offsets)| {
            // stat 补大小/修改时间（≤500 行/页，毫秒级）；失败记 0 不阻断。
            let absolute = state
                .search
                .root()
                .map(|root| root.join(item.relative_path(picker)));
            let (size, mtime) = absolute
                .as_deref()
                .and_then(|path| std::fs::metadata(path).ok())
                .map(|meta| {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| duration.as_millis() as i64)
                        .unwrap_or(0);
                    (meta.len(), mtime)
                })
                .unwrap_or((0, 0));
            FileHit {
                path: item.relative_path(picker),
                score: score.total,
                offsets: offsets.iter().map(|(a, b)| (*a, *b)).collect(),
                size,
                mtime,
            }
        })
        .collect();
    Ok(FilesPage {
        items,
        total_matched: result.total_matched,
        total_files: result.total_files,
    })
}

/// 内容搜索（流式）：分片推进 file_offset 直至搜完或取消，每分片经
/// Channel 推送增量命中。新调用自动取消上一次（取消令牌在 SearchState
/// 接力）；换根（generation 变化）也会终止进行中的搜索。
#[tauri::command]
pub async fn search_content(
    query: String,
    opts: Option<GrepOptions>,
    on_progress: tauri::ipc::Channel<GrepProgress>,
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
    let glob = opts
        .glob
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    if let Some(pattern) = &glob {
        glob_to_regex(pattern)?; // 先校验语法，坏 pattern 即时报错不静默。
    }
    let token = state.search.rotate_abort();
    let start_generation = state.search.generation();
    let search = state.search.clone();

    // 引擎 grep 是阻塞调用：整个分片循环放进 blocking 线程，主循环
    // （窗口/其他命令）全程不被冻结——这是「取消按钮立即生效」的前提。
    // parsed query 在闭包内构建：fff 的 ParsedQuery 借用查询串。
    tauri::async_runtime::spawn_blocking(move || {
        let mut all_items: Vec<GrepHit> = Vec::new();
        let mut files_searched = 0usize;
        let mut files_with_matches = 0usize;
        let mut offset = 0usize;
        let mut aborted = false;
        let parsed = QueryParser::default().parse(&query);

        loop {
            // 每分片独立取读锁：锁不跨分片持有，换根/状态轮询不再被长搜索拖住。
            // guard、grep 结果（借用 picker）、映射产物同块开始同块销毁。
            let (chunk_items, chunk_files, next_offset) = {
                let guard = search
                    .picker()
                    .read()
                    .map_err(|cause| Error::Search(cause.to_string()))?;
                let Some(picker) = guard.as_ref() else {
                    return Ok(GrepPage {
                        items: all_items,
                        files_searched,
                        files_with_matches,
                        next_file_offset: 0,
                        aborted: true,
                    });
                };

                let result = picker.grep(
                    &parsed,
                    &GrepSearchOptions {
                        max_file_size: GREP_MAX_FILE_SIZE,
                        max_matches_per_file: GREP_MAX_PER_FILE,
                        smart_case: opts.smart_case,
                        file_offset: offset,
                        page_limit: GREP_CHUNK_LIMIT,
                        mode: if opts.regex {
                            GrepMode::Regex
                        } else {
                            GrepMode::PlainText
                        },
                        time_budget_ms: GREP_CHUNK_BUDGET_MS,
                        before_context: opts.before_context.min(10),
                        after_context: opts.after_context.min(10),
                        classify_definitions: false,
                        trim_whitespace: false,
                        abort_signal: Some(Arc::clone(&token)),
                    },
                );
                files_searched = files_searched.max(result.total_files_searched);

                // glob 过滤在映射前：不命中的行不进推送，也不计入文件数。
                let chunk_items: Vec<GrepHit> = result
                    .matches
                    .iter()
                    .filter_map(|hit| {
                        let relative = result
                            .files
                            .get(hit.file_index)
                            .map(|item| item.relative_path(picker))
                            .unwrap_or_default();
                        if !glob
                            .as_deref()
                            .is_none_or(|pattern| hit_matches_glob(&relative, pattern))
                        {
                            return None;
                        }
                        Some(GrepHit {
                            path: relative,
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
                        })
                    })
                    .collect();
                let chunk_files = chunk_items
                    .iter()
                    .map(|hit| hit.path.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                (chunk_items, chunk_files, result.next_file_offset)
            };
            files_with_matches += chunk_files;

            all_items.extend(chunk_items.iter().cloned());
            let _ = on_progress.send(GrepProgress {
                items: chunk_items,
                files_searched,
                files_with_matches,
            });

            // 命中总量上限：更多结果请缩小范围或用 glob 过滤。
            if all_items.len() >= GREP_MAX_TOTAL_ITEMS {
                break;
            }

            if token.load(Ordering::Relaxed) {
                aborted = true;
                break;
            }
            // 换根（索引重建）后旧搜索结果失去意义：立即终止。
            if search.generation() != start_generation {
                aborted = true;
                break;
            }
            match next_offset {
                0 => break,                      // 搜完。
                next if next == offset => break, // 防御：游标不动则退出，避免死循环。
                next => offset = next,
            }
        }

        Ok(GrepPage {
            items: all_items,
            files_searched,
            files_with_matches,
            // 非 0 即「已中断」（流式循环下前端不再手动翻页）。
            next_file_offset: usize::from(aborted),
            aborted,
        })
    })
    .await
    .map_err(|cause| Error::Search(format!("内容搜索线程异常：{cause}")))?
}

/// 取消进行中的内容搜索。
#[tauri::command]
pub fn search_cancel(state: State<'_, crate::AppState>) -> Result<()> {
    state.search.cancel_search();
    Ok(())
}

// ---------------------------------------------------------------------------
// 盘符枚举（搜索根选择器）
// ---------------------------------------------------------------------------

/// 枚举逻辑盘（Windows：GetLogicalDrives 系；其余平台空表，Windows 优先）。
#[tauri::command]
pub fn search_list_drives() -> Vec<DriveInfo> {
    list_drives()
}

#[cfg(windows)]
fn list_drives() -> Vec<DriveInfo> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
    };
    // windows-sys 0.60 裁掉了 DRIVE_* 常量（Win32 文档值）。
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;

    let bitmask = unsafe { GetLogicalDrives() };
    if bitmask == 0 {
        return Vec::new();
    }
    let mut drives = Vec::new();
    for index in 0u32..26 {
        if bitmask & (1 << index) == 0 {
            continue;
        }
        let letter = (b'A' + index as u8) as char;
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        let kind_name = match kind {
            DRIVE_REMOVABLE => "removable",
            DRIVE_FIXED => "fixed",
            DRIVE_REMOTE => "network",
            DRIVE_CDROM => "cdrom",
            DRIVE_RAMDISK => "ramdisk",
            _ => continue, // UNKNOWN / NO_ROOT_DIR：不可用盘，不进选择器。
        };
        let mut free: u64 = 0;
        let mut total: u64 = 0;
        unsafe {
            GetDiskFreeSpaceExW(wide.as_ptr(), std::ptr::null_mut(), &mut total, &mut free);
        }
        drives.push(DriveInfo {
            path: root,
            kind: kind_name.to_owned(),
            total_bytes: total,
            free_bytes: free,
        });
    }
    drives.sort_by(|a, b| a.path.cmp(&b.path));
    drives
}

#[cfg(not(windows))]
fn list_drives() -> Vec<DriveInfo> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// glob 过滤（grep 文件名/路径筛选）
// ---------------------------------------------------------------------------

/// glob → 正则：`**` 跨段、`*`/`?` 不跨段、其余字面。错误即时上报。
fn glob_to_regex(pattern: &str) -> Result<regex::Regex> {
    let mut text = String::from("(?i)^");
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    text.push_str(".*");
                } else {
                    text.push_str("[^/\\\\]*");
                }
            }
            '?' => text.push_str("[^/\\\\]"),
            other => {
                for escaped in regex::escape(&other.to_string()).chars() {
                    text.push(escaped);
                }
            }
        }
    }
    text.push('$');
    regex::Regex::new(&text).map_err(|cause| Error::Search(format!("glob 语法错误：{cause}")))
}

/// 相对路径是否命中 glob：pattern 不含 `/` 时按文件名匹配，否则按全路径。
fn hit_matches_glob(relative: &str, pattern: &str) -> bool {
    let Ok(re) = glob_to_regex(pattern) else {
        return true; // 校验在入口做过；这里兜底放行，不静默吞结果。
    };
    if pattern.contains('/') {
        re.is_match(relative)
    } else {
        relative
            .rsplit(['/', '\\'])
            .next()
            .map(|name| re.is_match(name))
            .unwrap_or(false)
    }
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

    /// glob 语义：`*`/`?` 不跨段、`**` 跨段；不含 `/` 按文件名匹配，
    /// 含 `/` 按相对路径匹配；整体大小写不敏感（Windows 习惯）。
    #[test]
    fn glob匹配文件名与路径() {
        assert!(hit_matches_glob("src/main.rs", "*.rs"));
        assert!(!hit_matches_glob("src/main.rs", "*.ts"));
        assert!(hit_matches_glob("README.md", "*.md"));
        assert!(hit_matches_glob("src/lib/ipc/index.ts", "src/**"));
        assert!(!hit_matches_glob("lib/ipc/index.ts", "src/**"));
        assert!(hit_matches_glob("a.txt", "?.txt"));
        assert!(!hit_matches_glob("ab.txt", "?.txt"));
        assert!(hit_matches_glob("SRC/MAIN.rs", "*.RS"));
        assert!(hit_matches_glob("src/main.rs", "src/*"));
        assert!(!hit_matches_glob("deep/src/main.rs", "src/*"));
    }

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

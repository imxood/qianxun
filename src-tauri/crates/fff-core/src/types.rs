use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use crate::constants::PATH_BUF_SIZE;
use crate::index::constraints::Constrainable;
use crate::simd_path::ArenaPtr;
use fff_query_parser::{FFFQuery, FuzzyQuery, Location};

/// Different sources of the string storage used by FFF.
pub trait FFFStringStorage {
    fn arena_for(&self, file: &FileItem) -> ArenaPtr;
    fn base_arena(&self) -> ArenaPtr;
    fn overflow_arena(&self) -> ArenaPtr;
}

impl FFFStringStorage for ArenaPtr {
    #[inline]
    fn arena_for(&self, _file: &FileItem) -> ArenaPtr {
        *self
    }
    #[inline]
    fn base_arena(&self) -> ArenaPtr {
        *self
    }
    #[inline]
    fn overflow_arena(&self) -> ArenaPtr {
        *self
    }
}

pub trait FileSliceExt {
    fn live_count(&self) -> usize;
}

impl FileSliceExt for [FileItem] {
    #[inline]
    fn live_count(&self) -> usize {
        self.iter().filter(|f| !f.is_deleted()).count()
    }
}

pub struct FileItemFlags;

impl FileItemFlags {
    pub const BINARY: u8 = 1 << 0;
    pub const DELETED: u8 = 1 << 1;
    pub const OVERFLOW: u8 = 1 << 2;
}

pub struct DirFlags;

impl DirFlags {
    pub const OVERFLOW: u8 = 1 << 0;
    pub const DELETED: u8 = 1 << 1;
}

/// A directory in the file index. Shares chunk arena with file paths.
#[derive(Debug)]
pub struct DirItem {
    flags: u8,
    pub(crate) path: crate::simd_path::ChunkedString,
    last_segment_offset: u16,
}

impl Clone for DirItem {
    fn clone(&self) -> Self {
        Self {
            flags: self.flags,
            path: self.path.clone(),
            last_segment_offset: self.last_segment_offset,
        }
    }
}

impl DirItem {
    #[inline(always)]
    pub fn is_overflow(&self) -> bool {
        self.flags & DirFlags::OVERFLOW != 0
    }

    #[inline(always)]
    pub fn is_deleted(&self) -> bool {
        self.flags & DirFlags::DELETED != 0
    }

    pub(crate) fn set_deleted(&mut self, deleted: bool) -> bool {
        if self.is_deleted() == deleted {
            return false;
        }
        if deleted {
            self.flags |= DirFlags::DELETED;
        } else {
            self.flags &= !DirFlags::DELETED;
        }
        true
    }

    pub(crate) fn new(path: crate::simd_path::ChunkedString, last_segment_offset: u16) -> Self {
        Self {
            path,
            flags: 0,
            last_segment_offset,
        }
    }

    pub(crate) fn new_overflow(
        path: crate::simd_path::ChunkedString,
        last_segment_offset: u16,
    ) -> Self {
        Self {
            path,
            flags: DirFlags::OVERFLOW,
            last_segment_offset,
        }
    }

    #[inline]
    pub fn last_segment_offset(&self) -> u16 {
        self.last_segment_offset
    }

    pub(crate) fn read_relative_path<'a>(&self, arena: ArenaPtr, buf: &'a mut [u8]) -> &'a str {
        self.path.read_to_buf(arena, buf)
    }

    pub fn relative_path(&self, arena: impl FFFStringStorage) -> String {
        let mut out = String::new();
        let ptr = if self.is_overflow() {
            arena.overflow_arena()
        } else {
            arena.base_arena()
        };
        self.path.write_to_string(ptr, &mut out);
        out
    }

    pub fn write_dir_name(&self, arena: ArenaPtr, out: &mut String) {
        out.clear();
        let total = self.path.byte_len as usize;
        let offset = self.last_segment_offset as usize;
        if offset >= total {
            return;
        }
        let mut buf = [0u8; PATH_BUF_SIZE];
        let full = self.path.read_to_buf(arena, &mut buf);
        out.push_str(&full[offset..]);
    }

    pub fn dir_name(&self, arena: impl FFFStringStorage) -> String {
        let mut out = String::new();
        let ptr = if self.is_overflow() {
            arena.overflow_arena()
        } else {
            arena.base_arena()
        };
        self.write_dir_name(ptr, &mut out);
        out
    }

    pub fn absolute_path(&self, arena: impl FFFStringStorage, base_path: &Path) -> PathBuf {
        let rel = self.relative_path(arena);
        if rel.is_empty() {
            base_path.to_path_buf()
        } else {
            base_path.join(&rel)
        }
    }
}

impl Constrainable for DirItem {
    #[inline]
    fn write_file_name(&self, arena: ArenaPtr, out: &mut String) {
        self.write_dir_name(arena, out);
    }
    #[inline]
    fn write_relative_path(&self, arena: ArenaPtr, out: &mut String) {
        self.path.write_to_string(arena, out);
    }
    #[inline]
    fn is_overflow(&self) -> bool {
        DirItem::is_overflow(self)
    }
}

/// A single indexed file.
#[derive(Debug)]
pub struct FileItem {
    pub size: u64,
    pub modified: u64,
    pub(crate) path: crate::simd_path::ChunkedString,
    pub(crate) parent_dir_index: u32,
    flags: AtomicU8,
}

impl Clone for FileItem {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            parent_dir_index: self.parent_dir_index,
            size: self.size,
            modified: self.modified,
            flags: AtomicU8::new(self.flags.load(Ordering::Relaxed)),
        }
    }
}

/// Single-block read used by the binary classifier.
pub const BINARY_CLASSIFICATION_CHUNK_SIZE: usize = 16 * 1024;

/// A file is treated as binary if any NUL byte appears in the scanned prefix.
#[inline]
pub(crate) fn detect_binary_content(content: &[u8]) -> bool {
    memchr::memchr(0, content).is_some()
}

impl FileItem {
    pub fn new_raw(filename_start: u16, size: u64, modified: u64, is_binary: bool) -> Self {
        let mut flags = 0u8;
        if is_binary {
            flags |= FileItemFlags::BINARY;
        }
        let mut path = crate::simd_path::ChunkedString::empty();
        path.filename_offset = filename_start;
        Self {
            path,
            parent_dir_index: u32::MAX,
            size,
            modified,
            flags: AtomicU8::new(flags),
        }
    }

    pub fn absolute_path(&self, arena: impl FFFStringStorage, base_path: &Path) -> PathBuf {
        let mut buf = [0u8; PATH_BUF_SIZE];
        let rel = self.path.read_to_buf(arena.arena_for(self), &mut buf);
        base_path.join(rel)
    }

    pub(crate) fn set_path(&mut self, path: crate::simd_path::ChunkedString) {
        self.path = path;
    }

    pub fn dir_str(&self, arena: impl FFFStringStorage) -> String {
        let mut s = String::with_capacity(64);
        self.path.write_dir_to(arena.arena_for(self), &mut s);
        s
    }

    pub(crate) fn write_dir_str(&self, arena: ArenaPtr, out: &mut String) {
        self.path.write_dir_to(arena, out);
    }

    pub fn file_name(&self, arena: impl FFFStringStorage) -> String {
        let mut s = String::with_capacity(32);
        self.path.write_filename_to(arena.arena_for(self), &mut s);
        s
    }

    pub(crate) fn write_file_name_from_arena(&self, arena: ArenaPtr, out: &mut String) {
        self.path.write_filename_to(arena, out);
    }

    pub fn relative_path(&self, arena: impl FFFStringStorage) -> String {
        let mut s = String::with_capacity(64);
        self.path.write_to_string(arena.arena_for(self), &mut s);
        s
    }

    pub(crate) fn write_relative_path_from_arena(&self, arena: ArenaPtr, out: &mut String) {
        self.path.write_to_string(arena, out);
    }

    pub fn relative_path_len(&self) -> usize {
        self.path.byte_len as usize
    }

    pub fn filename_offset_in_relative_path(&self) -> usize {
        self.path.filename_offset as usize
    }

    pub(crate) fn relative_path_eq(&self, arena: ArenaPtr, other: &str) -> bool {
        if other.len() != self.path.byte_len as usize {
            return false;
        }
        let mut buf = [0u8; 512];
        let mine = self.path.read_to_buf(arena, &mut buf);
        mine == other
    }

    pub(crate) fn relative_path_starts_with(&self, arena: ArenaPtr, prefix: &str) -> bool {
        let mut buf = [0u8; PATH_BUF_SIZE];
        let path = self.path.read_to_buf(arena, &mut buf);
        path.starts_with(prefix)
    }

    pub(crate) fn write_absolute_path<'a>(
        &self,
        arena: ArenaPtr,
        base_path: &Path,
        buf: &'a mut [u8; PATH_BUF_SIZE],
    ) -> &'a Path {
        let base = base_path.as_os_str().as_encoded_bytes();
        let base_len = base.len();
        buf[..base_len].copy_from_slice(base);
        let sep_len = if base_len > 0 && base[base_len - 1] != std::path::MAIN_SEPARATOR as u8 {
            buf[base_len] = std::path::MAIN_SEPARATOR as u8;
            1
        } else {
            0
        };

        let base_end_idx = base_len + sep_len;
        let relative_portion_str = self.path.read_to_buf(arena, &mut buf[base_end_idx..]);
        let rel_len = relative_portion_str.len();
        let total = base_end_idx + rel_len;
        Path::new(unsafe { std::str::from_utf8_unchecked(&buf[..total]) })
    }

    #[cfg(unix)]
    pub(crate) fn write_relative_cstr<'a>(
        &self,
        arena: ArenaPtr,
        buf: &'a mut [u8; PATH_BUF_SIZE],
    ) -> &'a std::ffi::CStr {
        let rel = self.path.read_to_buf(arena, &mut buf[..PATH_BUF_SIZE - 1]);
        let n = rel.len();
        buf[n] = 0;
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(&buf[..=n]) }
    }

    /// Reads a fixed bytes count from the file.
    #[inline]
    pub(crate) fn read_trimmed_into_buf(
        &self,
        base_fd: i32,
        base_path: &Path,
        arena: ArenaPtr,
        path_buf: &mut [u8; PATH_BUF_SIZE],
        buf: &mut [u8],
    ) -> usize {
        #[cfg(unix)]
        {
            self.read_into_buf_unix(base_fd, base_path, arena, path_buf, buf)
        }
        #[cfg(not(unix))]
        {
            let _ = base_fd;
            self.read_into_buf_std(base_path, arena, path_buf, buf)
        }
    }

    #[cfg(unix)]
    fn read_into_buf_unix(
        &self,
        _base_fd: i32,
        base_path: &Path,
        arena: ArenaPtr,
        path_buf: &mut [u8; PATH_BUF_SIZE],
        buf: &mut [u8],
    ) -> usize {
        let abs = self.write_absolute_path(arena, base_path, path_buf);
        let Ok(mut f) = std::fs::File::open(abs) else {
            return 0;
        };
        match f.read(buf) {
            Ok(n) => n,
            Err(_) => 0,
        }
    }

    #[cfg(not(unix))]
    fn read_into_buf_std(
        &self,
        base_path: &Path,
        arena: ArenaPtr,
        path_buf: &mut [u8; PATH_BUF_SIZE],
        buf: &mut [u8],
    ) -> usize {
        let abs = self.write_absolute_path(arena, base_path, path_buf);
        let Ok(mut f) = std::fs::File::open(abs) else {
            return 0;
        };
        let mut filled = 0usize;
        while filled < buf.len() {
            match f.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(_) => return 0,
            }
        }
        filled
    }

    #[inline]
    pub fn is_binary(&self) -> bool {
        self.flags.load(Ordering::Relaxed) & FileItemFlags::BINARY != 0
    }

    #[inline]
    pub fn set_binary(&self, val: bool) {
        if val {
            self.flags
                .fetch_or(FileItemFlags::BINARY, Ordering::Relaxed);
        } else {
            self.flags
                .fetch_and(!FileItemFlags::BINARY, Ordering::Relaxed);
        }
    }

    pub(crate) fn detect_binary_per_byte(&self, path: &Path, chunk: &mut [u8]) {
        if self.size == 0 {
            return;
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .write(false)
            .read(true)
            .open(path)
        else {
            return;
        };
        loop {
            match file.read(chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if detect_binary_content(&chunk[..n]) {
                        self.set_binary(true);
                    }
                }
                Err(_) => break,
            }
        }
    }

    #[inline]
    pub fn is_deleted(&self) -> bool {
        self.flags.load(Ordering::Relaxed) & FileItemFlags::DELETED != 0
    }

    #[inline]
    pub fn set_deleted(&self, val: bool) {
        if val {
            self.flags
                .fetch_or(FileItemFlags::DELETED, Ordering::Relaxed);
        } else {
            self.flags
                .fetch_and(!FileItemFlags::DELETED, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn is_overflow(&self) -> bool {
        self.flags.load(Ordering::Relaxed) & FileItemFlags::OVERFLOW != 0
    }

    #[inline]
    pub fn set_overflow(&self, val: bool) {
        if val {
            self.flags
                .fetch_or(FileItemFlags::OVERFLOW, Ordering::Relaxed);
        } else {
            self.flags
                .fetch_and(!FileItemFlags::OVERFLOW, Ordering::Relaxed);
        }
    }

    /// Read file content for grep search (reads via std::fs::read into mmap_slot buffer).
    pub(crate) fn get_content_for_search(
        &self,
        _mmap_slot: &mut MmapSlot,
        arena: ArenaPtr,
        base_path: &Path,
        budget: &ContentCacheBudget,
    ) -> Option<&'static [u8]> {
        // Check file size against budget
        if self.size > budget.max_file_size {
            return None;
        }

        let abs_path = self.absolute_path(arena, base_path);

        // Read file content and leak it to return a 'static reference
        let Ok(data) = std::fs::read(&abs_path) else {
            return None;
        };
        let slice = data.leak();
        Some(unsafe { &*(slice as &[u8] as *const [u8] as *const [u8]) })
    }
}

impl Constrainable for FileItem {
    #[inline]
    fn write_file_name(&self, arena: ArenaPtr, out: &mut String) {
        self.path.write_filename_to(arena, out);
    }
    #[inline]
    fn write_relative_path(&self, arena: ArenaPtr, out: &mut String) {
        self.path.write_to_string(arena, out);
    }
    #[inline]
    fn is_overflow(&self) -> bool {
        FileItem::is_overflow(self)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Score {
    pub total: i32,
    pub base_score: i32,
    pub filename_bonus: i32,
    pub special_filename_bonus: i32,
    pub distance_penalty: i32,
    pub current_file_penalty: i32,
    pub path_alignment_bonus: i32,
    pub exact_match: bool,
    pub match_type: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct PaginationArgs {
    pub offset: usize,
    pub limit: usize,
}

impl Default for PaginationArgs {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 100,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScoringContext<'a> {
    pub query: &'a FFFQuery<'a>,
    pub project_path: Option<&'a Path>,
    pub current_file: Option<&'a str>,
    pub max_typos: u16,
    pub max_threads: usize,
    pub pagination: PaginationArgs,
}

impl ScoringContext<'_> {
    pub fn effective_query(&self) -> &str {
        match &self.query.fuzzy_query {
            FuzzyQuery::Text(t) => t,
            FuzzyQuery::Parts(parts) if !parts.is_empty() => parts[0],
            _ => self.query.raw_query.trim(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchResult<'a> {
    pub items: Vec<&'a FileItem>,
    pub scores: Vec<Score>,
    pub match_byte_offsets: Vec<smallvec::SmallVec<[(u32, u32); 4]>>,
    pub total_matched: usize,
    pub total_files: usize,
    pub location: Option<Location>,
}

#[derive(Debug, Clone, Default)]
pub struct DirSearchResult<'a> {
    pub items: Vec<&'a DirItem>,
    pub scores: Vec<Score>,
    pub total_matched: usize,
    pub total_dirs: usize,
}

#[derive(Debug, Clone)]
pub enum MixedItemRef<'a> {
    File(&'a FileItem),
    Dir(&'a DirItem),
}

#[derive(Debug, Clone, Default)]
pub struct MixedSearchResult<'a> {
    pub items: Vec<MixedItemRef<'a>>,
    pub scores: Vec<Score>,
    pub total_matched: usize,
    pub total_files: usize,
    pub total_dirs: usize,
    pub location: Option<Location>,
}

impl Default for MixedItemRef<'_> {
    fn default() -> Self {
        unreachable!("MixedItemRef::default should not be called")
    }
}

/// A minimal content cache budget for grep search.
/// On Windows we don't mmap, so this is effectively unused except for file-size cap.
#[derive(Debug)]
pub struct ContentCacheBudget {
    pub max_file_size: u64,
}

impl ContentCacheBudget {
    pub fn unlimited() -> Self {
        Self {
            max_file_size: crate::constants::MAX_FFFILE_SIZE,
        }
    }
    pub fn zero() -> Self {
        Self { max_file_size: 0 }
    }
    pub fn new_for_repo(_file_count: usize) -> Self {
        Self::unlimited()
    }
    pub fn from_overrides(_max_files: usize, _max_bytes: u64, max_file_size: u64) -> Option<Self> {
        if max_file_size == 0 {
            return None;
        }
        Some(Self { max_file_size })
    }
}

/// Simple buffer for reading file content (replaces MmapSlot for cross-platform compatibility).
#[derive(Default)]
pub struct MmapSlot {
    buffer: Vec<u8>,
}

impl MmapSlot {
    pub fn as_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }
    pub fn as_ref(&self) -> &[u8] {
        &self.buffer
    }
    pub fn set_len(&mut self, len: usize) {
        unsafe { self.buffer.set_len(len) };
    }
    pub fn reserve(&mut self, cap: usize) {
        self.buffer.reserve(cap);
    }
}

impl Default for ContentCacheBudget {
    fn default() -> Self {
        Self::unlimited()
    }
}

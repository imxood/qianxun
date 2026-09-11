//! # FFF Search — High-performance file finder core
//!
//! This crate provides the core search engine for [FFF (Fast File Finder)].
//! It includes filesystem indexing with real-time watching, fuzzy matching,
//! and multi-mode grep search.
//!
//! ## Architecture
//!
//! - [`file_picker::FilePicker`] — Main entry point. Indexes a directory tree
//!   in a background thread, maintains a sorted file list, watches the
//!   filesystem for changes, and performs fuzzy search with scoring.
//! - [`grep`] — Live grep search supporting regex, plain-text, and fuzzy modes.
//! - [`watch`] — Client-facing filesystem watch subscriptions.

#![allow(unexpected_cfgs)]

/// Primary entry points with thread-safe [`SharedFilePicker`](shared::FilePicker) instance
pub mod shared;
pub use shared::*;

/// Core file picker: filesystem indexing, background watching, and fuzzy search.
pub mod file_picker;
pub use file_picker::*;

/// Live grep search with regex, plain-text, and fuzzy matching modes.
pub mod grep;
pub use grep::*;

/// Disk space scanner: parallel walk with streaming ticks and cancellation.
pub mod disk;
pub use disk::{DiskEntry as DiskSpaceEntry, DiskScanResult, scan_directory};

/// Core data types shared across the crate.
pub mod types;
pub use types::*;

pub mod constants;

mod error;
mod ignore;
mod scan;
mod score;
mod sort_buffer;

pub(crate) mod index;
pub(crate) mod parallelism;
pub(crate) mod path_utils;
pub(crate) mod rescan_stats;
pub(crate) mod rescan_throttle;
pub(crate) mod simd_path;
pub(crate) mod simd_string_utils;
pub(crate) mod stable_vec;
pub(crate) mod walk;

/// Filesystem watch subscriptions with glob filtering and batched delivery.
#[path = "watcher/mod.rs"]
pub mod watch;
pub use watch::{WatchEvent, WatchEventKind, WatchId, WatchOptions};

// fff error
pub use error::{Error, Result};

pub use fff_query_parser::*;

//! 情报域落盘（docs/17 §8）：state.json / days/{date}/{topic}.json /
//! favorites.json / read-state.json。全部走 `atomic::write`；读取端对
//! 缺文件 / 半截历史文件宽容（返回空，不致命）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate};
use serde::de::DeserializeOwned;

use crate::atomic;
use crate::error::{Error, Result};
use crate::paths;

use super::types::{FavoriteEntry, FeedDayFile, IntelStateFile};

/// 保留天数（docs/17 §8.2）。
pub const RETENTION_DAYS: i64 = 90;

pub fn root(app: &tauri::AppHandle) -> Result<PathBuf> {
    paths::intel_dir(app)
}

/// 本地日期（yyyy-MM-dd）。「当天一次」与文件名统一用它。
pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn day_dir(root_dir: &Path, date: &str) -> PathBuf {
    root_dir.join("days").join(date)
}

fn feed_day_path(root_dir: &Path, date: &str, topic: &str) -> PathBuf {
    day_dir(root_dir, date).join(format!("{topic}.json"))
}

/// days/ 下某主题有数据文件的日期列表（降序）。目录名不合法即忽略。
pub fn list_days(root_dir: &Path, topic: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root_dir.join("days")) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if NaiveDate::parse_from_str(&name, "%Y-%m-%d").is_err() {
            continue;
        }
        if feed_day_path(root_dir, &name, topic).is_file() {
            out.push(name);
        }
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

/// 启动清理：删除超过保留期的日期目录。
pub fn prune(root_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(root_dir.join("days")) else {
        return;
    };
    let cutoff = Local::now().date_naive() - chrono::Duration::days(RETENTION_DAYS);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(date) = NaiveDate::parse_from_str(&name, "%Y-%m-%d") else {
            continue;
        };
        if date < cutoff {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes =
        serde_json::to_vec(value).map_err(|cause| Error::Intel(format!("序列化失败：{cause}")))?;
    atomic::write(path, &bytes).map_err(|cause| Error::Intel(format!("写 {path:?} 失败：{cause}")))
}

// ---- state.json ----

pub fn load_state(root_dir: &Path) -> IntelStateFile {
    read_json(&root_dir.join("state.json")).unwrap_or_default()
}

pub fn save_state(root_dir: &Path, state: &IntelStateFile) -> Result<()> {
    write_json(&root_dir.join("state.json"), state)
}

// ---- 天文件 ----

pub fn read_feed_day(root_dir: &Path, date: &str, topic: &str) -> Option<FeedDayFile> {
    read_json(&feed_day_path(root_dir, date, topic))
}

pub fn write_feed_day(root_dir: &Path, date: &str, topic: &str, day: &FeedDayFile) -> Result<()> {
    write_json(&feed_day_path(root_dir, date, topic), day)
}

// ---- 出刊文章（docs/17 §7.4）----

fn articles_day_path(root_dir: &Path, date: &str, topic: &str) -> PathBuf {
    day_dir(root_dir, date).join(format!("{topic}.articles.json"))
}

/// 某主题某天的出刊文章（无文件 = 尚未出刊，返回 None）。
pub fn read_articles_day(
    root_dir: &Path,
    date: &str,
    topic: &str,
) -> Option<super::types::ArticlesDayFile> {
    read_json(&articles_day_path(root_dir, date, topic))
}

/// 某主题有出刊文章的日期列表（降序）——冷启动回退用（docs/17 §7.5.2-4）。
pub fn list_article_days(root_dir: &Path, topic: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root_dir.join("days")) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if NaiveDate::parse_from_str(&name, "%Y-%m-%d").is_err() {
            continue;
        }
        if articles_day_path(root_dir, &name, topic).is_file() {
            out.push(name);
        }
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

pub fn write_articles_day(
    root_dir: &Path,
    date: &str,
    topic: &str,
    day: &super::types::ArticlesDayFile,
) -> Result<()> {
    write_json(&articles_day_path(root_dir, date, topic), day)
}

// ---- favorites.json / read-state.json ----

pub fn load_favorites(root_dir: &Path) -> Vec<FavoriteEntry> {
    read_json(&root_dir.join("favorites.json")).unwrap_or_default()
}

pub fn save_favorites(root_dir: &Path, favorites: &[FavoriteEntry]) -> Result<()> {
    write_json(&root_dir.join("favorites.json"), &favorites.to_vec())
}

pub fn load_read_state(root_dir: &Path) -> BTreeMap<String, i64> {
    read_json(&root_dir.join("read-state.json")).unwrap_or_default()
}

pub fn save_read_state(root_dir: &Path, read: &BTreeMap<String, i64>) -> Result<()> {
    write_json(&root_dir.join("read-state.json"), read)
}

#[cfg(test)]
mod tests {
    use super::super::types::SourceState;
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qx-intel-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn state与天文件roundtrip() {
        let root_dir = temp_root("state");
        let state = IntelStateFile {
            sources: BTreeMap::from([(
                "arxiv".into(),
                SourceState {
                    last_fetch_date: today(),
                    last_ok_at: Some(1234),
                    last_error: None,
                    consecutive_failures: 0,
                },
            )]),
        };
        save_state(&root_dir, &state).unwrap();
        let loaded = load_state(&root_dir);
        assert_eq!(loaded.sources["arxiv"].last_ok_at, Some(1234));

        let day = FeedDayFile {
            fetched_at: 42,
            items: vec![],
        };
        write_feed_day(&root_dir, "2026-10-02", "ai", &day).unwrap();
        assert_eq!(
            read_feed_day(&root_dir, "2026-10-02", "ai")
                .unwrap()
                .fetched_at,
            42
        );
        assert!(read_feed_day(&root_dir, "2026-10-01", "ai").is_none());
        std::fs::remove_dir_all(&root_dir).ok();
    }

    #[test]
    fn list_days只认合法日期且降序() {
        let root_dir = temp_root("days");
        for date in ["2026-10-02", "2026-10-01", "not-a-date"] {
            write_feed_day(
                &root_dir,
                date,
                "ai",
                &FeedDayFile {
                    fetched_at: 1,
                    items: vec![],
                },
            )
            .unwrap();
        }
        assert_eq!(list_days(&root_dir, "ai"), vec!["2026-10-02", "2026-10-01"]);
        std::fs::remove_dir_all(&root_dir).ok();
    }

    #[test]
    fn prune删除过期日期目录() {
        let root_dir = temp_root("prune");
        let old = (Local::now().date_naive() - chrono::Duration::days(RETENTION_DAYS + 1))
            .format("%Y-%m-%d")
            .to_string();
        for date in [old.as_str(), today().as_str()] {
            write_feed_day(
                &root_dir,
                date,
                "ai",
                &FeedDayFile {
                    fetched_at: 1,
                    items: vec![],
                },
            )
            .unwrap();
        }
        prune(&root_dir);
        assert!(read_feed_day(&root_dir, &old, "ai").is_none());
        assert!(read_feed_day(&root_dir, &today(), "ai").is_some());
        std::fs::remove_dir_all(&root_dir).ok();
    }
}

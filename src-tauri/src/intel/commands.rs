//! 情报域 IPC 命令（docs/17 §9）：主题状态 / 天数据 / 历史 / 刷新 /
//! 收藏 / 已读。全部同步（读文件为毫秒级小文件），刷新走后台 spawn。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};

use super::scheduler::IntelRuntime;
use super::store;
use super::topics;
use super::types::{FavoriteEntry, FeedItem};

/// 主题状态（intel_topics 返回项）：卡片直接渲染的三行信息。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelTopicStatus {
    pub id: String,
    pub label: String,
    /// 当天条目数（条）。
    pub today_count: u32,
    /// 最近一次成功抓取（毫秒；任一源）。
    pub last_fetch_at: Option<i64>,
    /// 全部源都成功过且无遗留错误。
    pub ok: bool,
    /// 抓取进行中（按钮转圈 / 卡片脉冲）。
    pub fetching: bool,
    /// 汇总错误摘要（取任一源的最后错误）。
    pub last_error: Option<String>,
    /// 抓取中当前并发请求（状态页「正在请求 xxx」）。
    #[serde(default)]
    pub active: Vec<super::types::SourceActivity>,
}

#[tauri::command]
pub fn intel_topics(app: AppHandle) -> Result<Vec<IntelTopicStatus>> {
    let root_dir = store::root(&app)?;
    let state = store::load_state(&root_dir);
    let today = store::today();
    let runtime = app.state::<IntelRuntime>();
    let mut out = Vec::new();
    for topic in topics::TOPICS {
        let today_count = store::read_feed_day(&root_dir, &today, topic.id)
            .map(|day| day.items.len())
            .unwrap_or(0);
        let mut last_fetch_at = None;
        let mut ok = !topic.sources.is_empty();
        let mut last_error = None;
        for source in topic.sources {
            let source_state = state.sources.get(source.id);
            last_fetch_at = last_fetch_at.max(source_state.and_then(|s| s.last_ok_at));
            match source_state {
                Some(s) if s.last_error.is_some() => {
                    ok = false;
                    last_error = last_error.or_else(|| s.last_error.clone());
                }
                Some(s) if s.last_ok_at.is_none() => ok = false,
                None => ok = false,
                _ => {}
            }
        }
        out.push(IntelTopicStatus {
            id: topic.id.to_owned(),
            label: topic.label.to_owned(),
            today_count: today_count as u32,
            last_fetch_at,
            ok,
            fetching: runtime.is_fetching(topic.id),
            last_error,
            active: runtime.active_of(topic.id),
        });
    }
    Ok(out)
}

/// 主题的一天（intel_topic_day 返回）。当天无数据 = items None（前端出「今日未抓」）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicDay {
    pub topic_id: String,
    pub date: String,
    pub items: Option<Vec<FeedItem>>,
    pub fetched_at: Option<i64>,
    /// 已读表（只含本日条目的 id，标题置灰用）。
    pub read: BTreeMap<String, i64>,
}

#[tauri::command]
pub fn intel_topic_day(app: AppHandle, topic_id: String, date: Option<String>) -> Result<TopicDay> {
    let topic =
        topics::topic(&topic_id).ok_or_else(|| Error::Intel(format!("未知主题：{topic_id}")))?;
    let root_dir = store::root(&app)?;
    let date = date.filter(|d| !d.is_empty()).unwrap_or_else(store::today);
    let read_all = store::load_read_state(&root_dir);

    let (items, fetched_at) = match store::read_feed_day(&root_dir, &date, topic.id) {
        Some(day) => (Some(day.items), Some(day.fetched_at)),
        None => (None, None),
    };
    // 已读表：只保留出现在本日条目里的 id。
    let read = read_all
        .iter()
        .filter(|(id, _)| {
            items
                .as_ref()
                .map(|list| list.iter().any(|item| &item.id == *id))
                .unwrap_or(false)
        })
        .map(|(id, at)| (id.clone(), *at))
        .collect();
    Ok(TopicDay {
        topic_id: topic.id.to_owned(),
        date,
        items,
        fetched_at,
        read,
    })
}

/// 主题某天的出刊文章（intel_articles）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticlesDay {
    pub topic_id: String,
    pub date: String,
    /// 出刊模型（provider/model）；空串 = 未出刊。
    pub model: String,
    pub articles: Vec<super::types::Article>,
}

#[tauri::command]
pub fn intel_articles(
    app: AppHandle,
    topic_id: String,
    date: Option<String>,
) -> Result<ArticlesDay> {
    let topic =
        topics::topic(&topic_id).ok_or_else(|| Error::Intel(format!("未知主题：{topic_id}")))?;
    let root_dir = store::root(&app)?;
    let date = date.filter(|d| !d.is_empty()).unwrap_or_else(store::today);
    if let Some(day) = store::read_articles_day(&root_dir, &date, topic.id) {
        return Ok(ArticlesDay {
            topic_id: topic.id.to_owned(),
            date,
            model: day.model,
            articles: day.articles,
        });
    }
    // 冷启动回退（docs/17 §7.5.2-4）：当天未出刊 → 最近一期（date 字段即实际期日，
    // 前端据此显示「{date} 刊」徽章）。
    let fallback = store::list_article_days(&root_dir, topic.id)
        .into_iter()
        .find(|day| day.as_str() != date);
    match fallback
        .and_then(|day| store::read_articles_day(&root_dir, &day, topic.id).map(|file| (day, file)))
    {
        Some((day, file)) => Ok(ArticlesDay {
            topic_id: topic.id.to_owned(),
            date: day,
            model: file.model,
            articles: file.articles,
        }),
        None => Ok(ArticlesDay {
            topic_id: topic.id.to_owned(),
            date,
            model: String::new(),
            articles: Vec::new(),
        }),
    }
}

/// 当前出刊进度（intel_digest_status）：状态弹框首次打开时拉全量。
#[tauri::command]
pub fn intel_digest_status(
    app: AppHandle,
    topic_id: String,
) -> Result<Option<super::types::DigestProgress>> {
    let runtime = app.state::<super::digest::DigestRuntime>();
    Ok(runtime.get(&topic_id))
}

/// 手动重出刊（intel_digest_start）：覆盖当天文章文件。
#[tauri::command]
pub fn intel_digest_start(app: AppHandle, topic_id: String) -> Result<()> {
    let topic =
        topics::topic(&topic_id).ok_or_else(|| Error::Intel(format!("未知主题：{topic_id}")))?;
    let today = store::today();
    super::digest::spawn_digest(app, topic, today, true);
    Ok(())
}

#[tauri::command]
pub fn intel_days(app: AppHandle, topic_id: String) -> Result<Vec<String>> {
    let topic =
        topics::topic(&topic_id).ok_or_else(|| Error::Intel(format!("未知主题：{topic_id}")))?;
    let root_dir = store::root(&app)?;
    Ok(store::list_days(&root_dir, topic.id))
}

/// 手动刷新：spawn 后台抓取（force 绕过当天闸门），立即返回。
#[tauri::command]
pub fn intel_refresh(app: AppHandle, topic_id: String) -> Result<()> {
    super::scheduler::spawn_refresh(app, topic_id)
}

/// intel_save 入参：收藏 = 深拷贝快照（含 zh 字段未来的演进也一并拷贝）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelSaveArgs {
    pub topic_id: String,
    pub feed: Option<FeedItem>,
    pub article: Option<super::types::Article>,
}

#[tauri::command]
pub fn intel_save(app: AppHandle, item: IntelSaveArgs) -> Result<Vec<FavoriteEntry>> {
    let topic = topics::topic(&item.topic_id)
        .ok_or_else(|| Error::Intel(format!("未知主题：{}", item.topic_id)))?;
    let id = if let Some(feed) = &item.feed {
        feed.id.clone()
    } else if let Some(article) = &item.article {
        article.id.clone()
    } else {
        return Err(Error::Intel("收藏条目为空".to_owned()));
    };
    let root_dir = store::root(&app)?;
    let mut favorites = store::load_favorites(&root_dir);
    favorites.retain(|entry| entry.id != id);
    favorites.insert(
        0,
        FavoriteEntry {
            id,
            topic_id: topic.id.to_owned(),
            saved_at: chrono::Utc::now().timestamp_millis(),
            feed: item.feed,
            article: item.article,
        },
    );
    store::save_favorites(&root_dir, &favorites)?;
    Ok(favorites)
}

#[tauri::command]
pub fn intel_unsave(app: AppHandle, id: String) -> Result<Vec<FavoriteEntry>> {
    let root_dir = store::root(&app)?;
    let mut favorites = store::load_favorites(&root_dir);
    favorites.retain(|entry| entry.id != id);
    store::save_favorites(&root_dir, &favorites)?;
    Ok(favorites)
}

#[tauri::command]
pub fn intel_favorites(app: AppHandle) -> Result<Vec<FavoriteEntry>> {
    Ok(store::load_favorites(&store::root(&app)?))
}

#[tauri::command]
pub fn intel_mark_read(app: AppHandle, ids: Vec<String>) -> Result<()> {
    let root_dir = store::root(&app)?;
    let mut read = store::load_read_state(&root_dir);
    let now = chrono::Utc::now().timestamp_millis();
    for id in ids {
        read.insert(id, now);
    }
    store::save_read_state(&root_dir, &read)
}

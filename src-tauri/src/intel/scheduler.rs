//! 抓取调度（docs/17 §3/§6）：启动补抓（当天一次）+ 按主题手动刷新。
//! 无后台定时器——「每天自动」由启动触发，跨天不关机由前端横幅一键补。
//!
//! 失败语义：单源失败只记 state.json 并降级出内容；仅当全部源失败且
//! 当天尚无可用数据时才向上报错（手动刷新的 toast / 启动日志）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};

use crate::error::{Error, Result};

use super::store;
use super::topics::{self, SourceDef, TopicDef};
use super::types::{
    FeedDayFile, FeedItem, IntelFetchingEvent, IntelStateFile, IntelUpdatedEvent, SourceActivity,
};

/// 运行态：正在抓取的主题（防重复刷新）+ 各主题并发中的请求（状态页展示）。
/// 独立 manage（同 LayaProcState 模式）。
#[derive(Default)]
pub struct IntelRuntime {
    fetching: Mutex<BTreeSet<String>>,
    /// topicId → 并发中的请求（begin/end 维护）。
    active: Mutex<BTreeMap<String, Vec<SourceActivity>>>,
}

impl IntelRuntime {
    /// 主题是否正在抓取（intel_topics 卡片状态）。
    pub(crate) fn is_fetching(&self, topic_id: &str) -> bool {
        self.fetching
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(topic_id)
    }

    /// 主题当前并发中的请求（intel_topics 返回 + intel://fetching 增量）。
    pub(crate) fn active_of(&self, topic_id: &str) -> Vec<SourceActivity> {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(topic_id)
            .cloned()
            .unwrap_or_default()
    }

    fn begin_source(&self, topic_id: &str, activity: SourceActivity) {
        let mut guard = self.active.lock().unwrap_or_else(|p| p.into_inner());
        let list = guard.entry(topic_id.to_owned()).or_default();
        list.retain(|a| a.source_id != activity.source_id);
        list.push(activity);
    }

    fn end_source(&self, topic_id: &str, source_id: &str) {
        let mut guard = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(list) = guard.get_mut(topic_id) {
            list.retain(|a| a.source_id != source_id);
        }
    }
}

/// 登记一个活动请求并广播（状态页「正在请求 xxx」）。
fn report_active(app: &AppHandle, runtime: &IntelRuntime, topic_id: &str, source: &SourceDef) {
    runtime.begin_source(
        topic_id,
        SourceActivity {
            source_id: source.id.to_owned(),
            url: source.endpoint.to_owned(),
        },
    );
    broadcast_active(app, runtime, topic_id);
}

fn clear_active(app: &AppHandle, runtime: &IntelRuntime, topic_id: &str, source_id: &str) {
    runtime.end_source(topic_id, source_id);
    broadcast_active(app, runtime, topic_id);
}

fn broadcast_active(app: &AppHandle, runtime: &IntelRuntime, topic_id: &str) {
    let active = runtime
        .active
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(topic_id)
        .cloned()
        .unwrap_or_default();
    let _ = app.emit(
        "intel://fetching",
        &IntelFetchingEvent {
            topic_id: topic_id.to_owned(),
            active,
        },
    );
}

/// 启动入口：清理过期天文件 → 逐主题补抓（错峰）→ 同日出刊补偿 → 日期哨兵。
pub async fn startup(app: AppHandle) {
    if let Ok(root_dir) = store::root(&app) {
        store::prune(&root_dir);
    }
    for topic in topics::TOPICS {
        if let Err(cause) = fetch_topic(app.clone(), topic, false).await {
            crate::logging::log(
                "warn",
                &format!("情报「{}」启动补抓失败：{cause}", topic.label),
            );
        }
        // 主题间错峰：对上游克制（docs/17 §6）。
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    compensate_digest(&app);
    spawn_day_sentinel(app);
}

/// 日期哨兵（docs/17 §7.5.2-2）：跨天自动补抓 + 出刊，「每日杂志」的节律保证。
/// 每小时比对本地日期（不是高频轮询；跨天后的首轮 startup 逻辑完成全部工作）。
fn spawn_day_sentinel(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut today = store::today();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            let now = store::today();
            if now == today {
                continue;
            }
            today = now.clone();
            crate::logging::log("info", &format!("情报跨天（{now}）：补抓并出刊"));
            for topic in topics::TOPICS {
                if let Err(cause) = fetch_topic(app.clone(), topic, false).await {
                    crate::logging::log(
                        "warn",
                        &format!("情报「{}」跨天补抓失败：{cause}", topic.label),
                    );
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    });
}

/// 同日出刊补偿（docs/17 §7.5.2-2）：当天 feed 已抓但 articles 缺失
/// （上次出刊被中断）→ 补一次出刊。幂等：spawn_digest(force=false)。
fn compensate_digest(app: &AppHandle) {
    let Ok(root_dir) = store::root(app) else {
        return;
    };
    let today = store::today();
    for topic in topics::TOPICS {
        let feed_ok = store::read_feed_day(&root_dir, &today, topic.id).is_some();
        let articles_ok = store::read_articles_day(&root_dir, &today, topic.id).is_some();
        if feed_ok && !articles_ok {
            crate::logging::log(
                "info",
                &format!("情报「{}」当日未出刊，补偿出刊", topic.label),
            );
            super::digest::spawn_digest(app.clone(), topic, today.clone(), false);
        }
    }
}

/// 抓一个主题：合并去重落盘，逐源更新 state。
/// `force=false` 时按源的 `lastFetchDate` 闸门跳过当天已成功的源。
pub async fn fetch_topic(app: AppHandle, topic: &'static TopicDef, force: bool) -> Result<()> {
    {
        let runtime = app.state::<IntelRuntime>();
        let mut fetching = runtime.fetching.lock().unwrap_or_else(|p| p.into_inner());
        if fetching.contains(topic.id) {
            return Ok(());
        }
        fetching.insert(topic.id.to_owned());
    }

    // 并发拉源（≤4）：Semaphore 限流 + JoinSet 收集。刻意不走
    // futures_util 组合子栈（stream::iter + buffer_unordered）——
    // 它触发 rustc #100013 的 Future Send+'static 推断缺陷
    // （"lifetime bound not satisfied"，已实测二分定位）。
    let result = {
        let root_dir = store::root(&app)?;
        let today = store::today();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut state = store::load_state(&root_dir);
        let client = super::fetch::client().map_err(Error::Intel)?;
        let runtime_state = app.state::<IntelRuntime>();

        let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
        let mut tasks = tokio::task::JoinSet::new();
        for source in topic.sources {
            if !force && source_fresh(&state, source.id, &today) {
                continue;
            }
            let client = client.to_owned();
            let Ok(_permit) = semaphore.clone().acquire_owned().await else {
                continue;
            };
            report_active(&app, &runtime_state, topic.id, source);
            tasks.spawn(async move {
                let _permit = _permit;
                // 失败重试 1 次（间隔 5s）：偶发网络抖动不记 ⚠。
                let mut outcome = super::fetch::fetch_feed_source(client.to_owned(), source).await;
                if outcome.is_err() {
                    crate::logging::log(
                        "info",
                        &format!("情报源 {} 首抓失败，5s 后重试", source.id),
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    outcome = super::fetch::fetch_feed_source(client, source).await;
                }
                match outcome {
                    Ok(items) => (source.id.to_owned(), None, Some(items)),
                    Err(cause) => (source.id.to_owned(), Some(cause), None),
                }
            });
        }

        let mut merged: BTreeMap<String, FeedItem> = BTreeMap::new();
        // 已有当天文件：同 id 保留 firstSeen（刷新只更新热度）。
        if let Some(existing) = store::read_feed_day(&root_dir, &today, topic.id) {
            for item in existing.items {
                merged.insert(item.id.clone(), item);
            }
        }
        let mut ok_sources = 0usize;
        let mut skipped = 0usize;
        let mut errors: Vec<String> = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            let Ok((source_id, error, items)) = joined else {
                continue; // 任务 panic（不应发生）：当跳过处理。
            };
            clear_active(&app, &runtime_state, topic.id, &source_id);
            match (error, items) {
                (None, Some(items)) => {
                    ok_sources += 1;
                    record(&mut state, &source_id, None, now_ms, &today);
                    for mut item in items {
                        if let Some(kept) = merged.get(&item.id) {
                            item.first_seen_at = kept.first_seen_at;
                        }
                        merged.insert(item.id.clone(), item);
                    }
                }
                (None, None) => {
                    skipped += 1;
                }
                (Some(cause), _) => {
                    errors.push(cause.clone());
                    record(&mut state, &source_id, Some(&cause), now_ms, &today);
                }
            }
        }

        let mut items: Vec<FeedItem> = merged.into_values().collect();
        sort_feed(&mut items);
        items.truncate(100);
        store::write_feed_day(
            &root_dir,
            &today,
            topic.id,
            &FeedDayFile {
                fetched_at: now_ms,
                items,
            },
        )?;
        store::save_state(&root_dir, &state)?;

        // 抓到内容 → 自动出刊（Agent 编辑部，docs/17 §7.4）。
        // force=false：同日已出刊则跳过（手动重出刊走 intel_digest_start）。
        if ok_sources > 0 {
            super::digest::spawn_digest(app.clone(), topic, today.clone(), false);
        }

        // 全部源都尝试过且无一成功、当天也没有既有数据 → 报错。
        if ok_sources == 0 && skipped == 0 && !topic.sources.is_empty() {
            let has_existing = store::read_feed_day(&root_dir, &today, topic.id)
                .map(|day| !day.items.is_empty())
                .unwrap_or(false);
            if !has_existing {
                return Err(Error::Intel(format!(
                    "全部 {} 源失败：{}",
                    topic.sources.len(),
                    errors.join("；")
                )));
            }
        }
        Ok(())
    };

    {
        let runtime = app.state::<IntelRuntime>();
        let mut fetching = runtime.fetching.lock().unwrap_or_else(|p| p.into_inner());
        fetching.remove(topic.id);
    }
    // 无论成败都发事件：前端刷新状态行（失败也体现在源状态里）。
    let _ = app.emit(
        "intel://updated",
        &IntelUpdatedEvent {
            topic_id: topic.id.to_owned(),
        },
    );
    result
}

/// 每源抓取后的记分（state.json 更新）：成功清错误、失败累加退避计数。
fn record(
    state: &mut IntelStateFile,
    source_id: &str,
    error: Option<&str>,
    now_ms: i64,
    today: &str,
) {
    let entry = state.sources.entry(source_id.to_owned()).or_default();
    match error {
        None => {
            entry.last_fetch_date = today.to_owned();
            entry.last_ok_at = Some(now_ms);
            entry.last_error = None;
            entry.consecutive_failures = 0;
        }
        Some(cause) => {
            entry.last_error = Some(cause.to_owned());
            entry.consecutive_failures += 1;
        }
    }
}

fn source_fresh(state: &IntelStateFile, source_id: &str, today: &str) -> bool {
    state
        .sources
        .get(source_id)
        .map(|s| s.last_fetch_date == today)
        .unwrap_or(false)
}

/// Feed 排序：有热度的按热度降序，其余按发布时间降序。
fn sort_feed(items: &mut [FeedItem]) {
    items.sort_by_key(|item| {
        std::cmp::Reverse(
            (item.heat.unwrap_or(0) as i64) * 1_000_000_000
                + (item.published_at.unwrap_or(0) / 10_000),
        )
    });
}

/// 供 commands.rs 触发手动刷新（spawn 到后台，立即返回）。
pub fn spawn_refresh(app: AppHandle, topic_id: String) -> Result<()> {
    let topic =
        topics::topic(&topic_id).ok_or_else(|| Error::Intel(format!("未知主题：{topic_id}")))?;
    tauri::async_runtime::spawn(async move {
        if let Err(cause) = fetch_topic(app, topic, true).await {
            crate::logging::log("warn", &format!("情报「{}」刷新失败：{cause}", topic.label));
        }
    });
    Ok(())
}

/// 启动任务入口（lib.rs setup spawn）。
pub fn spawn_startup(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        startup(app).await;
    });
}

#[cfg(test)]
mod tests {
    use super::super::types::SourceState;
    use super::*;

    fn item(id: &str, heat: Option<u64>, published: i64) -> FeedItem {
        FeedItem {
            id: id.to_owned(),
            source_id: "s".into(),
            title: id.to_owned(),
            url: format!("https://example.com/{id}"),
            summary: String::new(),
            heat,
            published_at: Some(published),
            first_seen_at: 0,
        }
    }

    #[test]
    fn feed排序热度优先发布次之() {
        let mut items = vec![
            item("b", Some(10), 200),
            item("a", Some(500), 100),
            item("c", None, 300),
        ];
        sort_feed(&mut items);
        assert_eq!(items[0].id, "a");
        assert_eq!(items[1].id, "b");
        assert_eq!(items[2].id, "c");
    }

    #[test]
    fn 当天闸门按源判定() {
        let mut state = IntelStateFile::default();
        assert!(!source_fresh(&state, "arxiv", "2026-10-02"));
        state.sources.insert(
            "arxiv".into(),
            SourceState {
                last_fetch_date: "2026-10-02".into(),
                ..Default::default()
            },
        );
        assert!(source_fresh(&state, "arxiv", "2026-10-02"));
        assert!(!source_fresh(&state, "arxiv", "2026-10-03"));
    }

    #[test]
    fn record成功清错误失败累加() {
        let mut state = IntelStateFile::default();
        record(&mut state, "arxiv", Some("请求失败"), 1, "2026-10-02");
        record(&mut state, "arxiv", Some("请求失败"), 2, "2026-10-02");
        assert_eq!(state.sources["arxiv"].consecutive_failures, 2);
        assert!(state.sources["arxiv"].last_error.is_some());
        record(&mut state, "arxiv", None, 3, "2026-10-02");
        assert_eq!(state.sources["arxiv"].consecutive_failures, 0);
        assert!(state.sources["arxiv"].last_error.is_none());
        assert_eq!(state.sources["arxiv"].last_fetch_date, "2026-10-02");
    }
}

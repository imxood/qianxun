//! Agent 编辑部出刊（docs/17 §7.4）：筛选 → 取正文 → 深加工 → 归并。
//! 每一步经 `intel://digest` 事件上报进度（状态弹框数据源）；
//! LLM 只走 qx-llm 桥，DSH 未运行/未配置 → `llm-unavailable` 明确提示。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use super::super::error::Error;
use super::super::harness;
use super::prompts;
use super::store;
use super::types::{
    Article, ArticleSource, ArticlesDayFile, DigestProgress, DigestStage, FeedItem,
    IntelUpdatedEvent,
};

/// 单次出刊的护栏：rewrite 阶段最多篇数（杂志配额；2026-10-03 质量修订
/// 15→12：以质量优先，把预算让给篇幅与原料量）。
const MAX_REWRITE: usize = 12;
/// bodies/rewrite 并发。
const DIGEST_CONCURRENCY: usize = 3;
/// 跨主题全局 LLM 闸门（实测 2026-10-03：三主题同时出刊 3×3 并发打爆
/// zai 限流，429 丢篇）。所有 LLM 调用（triage/rewrite/synthesis）共用。
static LLM_GATE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
/// triage 每批条数（100 条一批输出必截断，docs/17 §7.5.1-2）。
const TRIAGE_BATCH: usize = 50;
/// 单次 LLM 调用超时（秒）。
const LLM_TIMEOUT_SECS: u64 = 120;

/// 出刊进度状态（全局）：topicId → 最近一次进度 + 运行中标记。
#[derive(Default)]
pub struct DigestRuntime {
    progress: std::sync::Mutex<BTreeMap<String, DigestProgress>>,
    /// 运行中集合：spawn 的同步临界区内「检查 + 置位」，杜绝双跑竞态
    /// （旧实现按 progress.stage 判断，与异步首次写入之间有调度窗口）。
    running: std::sync::Mutex<BTreeSet<String>>,
}

impl DigestRuntime {
    pub fn get(&self, topic_id: &str) -> Option<DigestProgress> {
        self.progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(topic_id)
            .cloned()
    }

    fn set(&self, progress: DigestProgress) {
        self.progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(progress.topic_id.clone(), progress);
    }

    /// 同步抢占：true = 成功登记（可以跑），false = 已在跑。
    fn try_begin(&self, topic_id: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(topic_id.to_owned())
    }

    fn end(&self, topic_id: &str) {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(topic_id);
    }
}

/// 更新进度并广播（状态弹框实时刷新）。started_at 保留首次值（出刊起点）。
fn report(
    app: &AppHandle,
    topic_id: &str,
    stage: DigestStage,
    done: u32,
    total: u32,
    message: impl Into<String>,
) {
    let message: String = message.into();
    let now = chrono::Utc::now().timestamp_millis();
    let started_at = app
        .try_state::<DigestRuntime>()
        .and_then(|runtime| {
            let started = runtime
                .get(topic_id)
                .and_then(|p| p.started_at)
                .or(Some(now));
            runtime.set(DigestProgress {
                topic_id: topic_id.to_owned(),
                stage,
                done,
                total,
                message: message.clone(),
                started_at: started,
            });
            started
        })
        .or(Some(now));
    // 广播必须无条件（状态页/横幅的实时数据源）。
    let _ = app.emit(
        "intel://digest",
        &DigestProgress {
            topic_id: topic_id.to_owned(),
            stage,
            done,
            total,
            message,
            started_at,
        },
    );
}

/// 启动出刊。force=false（自动路径）：同日已出刊则跳过（修「刷新=重出刊」）；
/// force=true（用户显式「重新出刊」）：覆盖当天文件。
pub fn spawn_digest(
    app: AppHandle,
    topic: &'static super::topics::TopicDef,
    day: String,
    force: bool,
) {
    let Some(runtime_state) = app.try_state::<DigestRuntime>() else {
        return;
    };
    // 同步临界区防重入：双击/自动+手动并发只会有一个跑。
    if !runtime_state.try_begin(topic.id) {
        return;
    }
    if !force {
        // 自动路径幂等：当天已出刊不再烧 token。
        let already = store::root(&app)
            .ok()
            .map(|root| store::read_articles_day(&root, &day, topic.id).is_some())
            .unwrap_or(false);
        if already {
            runtime_state.end(topic.id);
            return;
        }
    }
    tauri::async_runtime::spawn(async move {
        let result = run_digest(app.clone(), topic, &day).await;
        if let Some(runtime_state) = app.try_state::<DigestRuntime>() {
            runtime_state.end(topic.id);
        }
        if let Err(cause) = result {
            crate::logging::log(
                "warn",
                &format!("情报「{label}」出刊失败：{cause}", label = topic.label),
            );
            report(&app, topic.id, DigestStage::Failed, 0, 0, cause.to_string());
        }
    });
}

/// 四段流水线主体。
async fn run_digest(
    app: AppHandle,
    topic: &'static super::topics::TopicDef,
    day: &str,
) -> std::result::Result<(), String> {
    let root_dir = store::root(&app).map_err(|e| e.to_string())?;
    let Some(day_file) = store::read_feed_day(&root_dir, day, topic.id) else {
        return Ok(()); // 当天没有条目（还没抓到）：无事可做。
    };
    let items = day_file.items;
    if items.is_empty() {
        return Ok(());
    }

    // 0. LLM 通道（用户定稿：仅 qx-llm 桥）。
    // 不检查 supervisor：外部自启的 DSH（系统级常驻）不被千寻的 Supervisor
    // 接管，但出刊只依赖配置+key 直连（docs/17 §7.5.3 阶段 2 之 H1 修复）。
    // 配置发现失败（无 provider/无 key）→ LlmUnavailable 明确提示。
    report(&app, topic.id, DigestStage::Triage, 0, 1, "检查 LLM 通道…");
    let settings = crate::settings::load(&app).map_err(|e| e.to_string())?;
    let dsh_home = harness::dsh_home(&app, &settings);
    let channel = super::llm::discover_channel(&dsh_home).map_err(|cause| {
        let message = format!("LLM 通道不可用：{cause}");
        crate::logging::log("warn", &format!("情报出刊：{message}"));
        report(
            &app,
            topic.id,
            DigestStage::LlmUnavailable,
            0,
            0,
            message.clone(),
        );
        message
    })?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(LLM_TIMEOUT_SECS))
        .user_agent("qianxun-intel/0.7 (desktop; personal)")
        .build()
        .map_err(|cause| format!("HTTP 客户端构建失败：{cause}"))?;
    let channel_label = format!("{}/{}", channel.provider, channel.writing_model);

    // 1. 筛选（≤50 条/批；短序号映射；覆盖率护栏——docs/17 §7.5.1-2）。
    report(
        &app,
        topic.id,
        DigestStage::Triage,
        0,
        1,
        format!("筛选 {} 条…", items.len()),
    );
    let mut scored: Vec<(FeedItem, u8)> = Vec::new();
    for batch in items.chunks(TRIAGE_BATCH) {
        let lines: Vec<String> = batch
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                format!(
                    "{idx} | {} | {} | {}",
                    item.source_id,
                    item.title,
                    truncate_chars(&item.summary, 160)
                )
            })
            .collect();
        let llm_permit = LLM_GATE.clone().acquire_owned().await;
        let raw = super::llm::complete(
            client.clone(),
            &channel,
            super::llm::CompleteArgs {
                model: channel.model.clone(),
                system: prompts::editor_prompt(topic.id).to_owned(),
                user: prompts::triage_instruction(topic.id, &lines.join("\n"), batch.len()),
                max_tokens: 8192,
            },
        )
        .await;
        drop(llm_permit);
        let raw = raw.map_err(|cause| format!("筛选调用失败：{cause}"))?;
        scored.extend(triage_scores(batch, &raw));
    }
    // 配额：按全局评分降序取前 MAX_REWRITE（importance 相对性来自 triage）。
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
    scored.truncate(MAX_REWRITE);
    let kept = scored;
    if kept.is_empty() {
        write_articles(&app, &root_dir, day, topic.id, &channel_label, Vec::new())?;
        report(
            &app,
            topic.id,
            DigestStage::Done,
            1,
            1,
            "今日无值得报道的内容",
        );
        return Ok(());
    }
    report(
        &app,
        topic.id,
        DigestStage::Triage,
        1,
        1,
        format!("筛出 {} 条", kept.len()),
    );

    // 2. 取正文（并发 3；失败退 RSS 摘要）。
    let kept_len = kept.len() as u32;
    report(
        &app,
        topic.id,
        DigestStage::Bodies,
        0,
        kept_len,
        "抓取原文正文…",
    );
    let semaphore = Arc::new(tokio::sync::Semaphore::new(DIGEST_CONCURRENCY));
    let mut body_tasks = tokio::task::JoinSet::new();
    for (item, _score) in kept.iter() {
        let client = client.clone();
        let item = item.clone();
        let permit = semaphore.clone().acquire_owned().await;
        body_tasks.spawn(async move {
            let _permit = permit;
            let raw = fetch_raw(&client, &item.url).await;
            let extracted = raw.and_then(|(source_id, text)| {
                super::extract::extract_body(&source_id, &item.url, &text)
            });
            let body_text = match extracted {
                Some(extracted) => extracted.text,
                None => item.summary.clone(),
            };
            (item.id, body_text)
        });
    }
    let mut bodies: BTreeMap<String, String> = BTreeMap::new();
    let mut bodies_done = 0u32;
    while let Some(joined) = body_tasks.join_next().await {
        if let Ok((id, text)) = joined {
            bodies.insert(id, text);
        }
        bodies_done += 1;
        report(
            &app,
            topic.id,
            DigestStage::Bodies,
            bodies_done,
            kept_len,
            "抓取原文正文…",
        );
    }

    // 3. 深加工（并发 3；写作模型分层；单篇 JSON 失败带提示重试 1 次）。
    report(
        &app,
        topic.id,
        DigestStage::Rewriting,
        0,
        kept_len,
        format!("深加工（{channel_label}）…"),
    );
    let mut rewrite_tasks = tokio::task::JoinSet::new();
    for (item, score) in kept.iter() {
        let client = client.clone();
        let channel = channel.clone();
        let channel_label = channel_label.clone();
        let item = item.clone();
        let score = *score;
        let body_text = bodies.get(&item.id).cloned().unwrap_or_default();
        let permit = semaphore.clone().acquire_owned().await;
        let llm_permit = LLM_GATE.clone().acquire_owned().await;
        rewrite_tasks.spawn(async move {
            let _permit = permit;
            let _llm_permit = llm_permit;
            let sources = format!("{}（{}）", item.source_id, item.url);
            let mut instruction = prompts::rewrite_instruction(
                topic.id,
                &item.title,
                &truncate_chars(&item.summary, 300),
                &body_text,
                &sources,
            );
            // 4096：1200-2000 字中文 + JSON 包裹的输出预算（2026-10-03 质量修订）。
            let result = super::llm::complete(
                client.clone(),
                &channel,
                super::llm::CompleteArgs {
                    model: channel.writing_model.clone(),
                    system: prompts::editor_prompt(topic.id).to_owned(),
                    user: instruction.clone(),
                    max_tokens: 4096,
                },
            )
            .await;
            // JSON 解析失败：附提示重试 1 次（其余错误不重试——complete 已内置）。
            let value = match result
                .as_ref()
                .ok()
                .and_then(|raw| super::llm::extract_json(raw))
            {
                Some(value) => value,
                None => {
                    instruction.push_str("\n\n（上一次输出不是合法 JSON，请只输出 JSON 本体。）");
                    let retry = super::llm::complete(
                        client,
                        &channel,
                        super::llm::CompleteArgs {
                            model: channel.writing_model.clone(),
                            system: prompts::editor_prompt(topic.id).to_owned(),
                            user: instruction,
                            max_tokens: 4096,
                        },
                    )
                    .await?;
                    super::llm::extract_json(&retry)
                        .ok_or_else(|| Error::Intel("深加工输出不是 JSON".to_owned()))?
                }
            };
            article_from_json(&item, topic.id, &channel_label, &value, score)
        });
    }
    let mut drafts: Vec<Article> = Vec::new();
    let mut rewrite_done = 0u32;
    let mut rewrite_failed = 0u32;
    while let Some(joined) = rewrite_tasks.join_next().await {
        rewrite_done += 1;
        match joined {
            Ok(Ok(article)) => drafts.push(article),
            Ok(Err(cause)) => {
                rewrite_failed += 1;
                crate::logging::log("warn", &format!("情报深加工单篇失败：{cause}"));
            }
            Err(cause) => {
                rewrite_failed += 1;
                crate::logging::log("warn", &format!("情报深加工任务异常：{cause}"));
            }
        }
        report(
            &app,
            topic.id,
            DigestStage::Rewriting,
            rewrite_done,
            kept_len,
            format!("深加工（{channel_label}）…"),
        );
    }
    if drafts.is_empty() {
        return Err(format!("深加工全部失败（{rewrite_failed} 篇）"));
    }

    // 4. 归并（分组决策 1 次调用；失败退草稿原样）。
    report(
        &app,
        topic.id,
        DigestStage::Synthesis,
        0,
        1,
        "归并同事件报道…",
    );
    let drafts_block = drafts
        .iter()
        .map(|a| format!("{} | {} | {}", a.id, a.title, a.lead))
        .collect::<Vec<_>>()
        .join("\n");
    let llm_permit = LLM_GATE.clone().acquire_owned().await;
    let synthesis_result = super::llm::complete(
        client.clone(),
        &channel,
        super::llm::CompleteArgs {
            model: channel.model.clone(),
            system: prompts::editor_prompt(topic.id).to_owned(),
            user: prompts::synthesis_instruction(topic.id, &drafts_block),
            max_tokens: 2048,
        },
    )
    .await;
    drop(llm_permit);
    let final_articles = match synthesis_result {
        Ok(raw) => apply_synthesis(&drafts, &raw),
        Err(cause) => {
            crate::logging::log("warn", &format!("情报归并失败，按未合并出刊：{cause}"));
            drafts
        }
    };
    report(&app, topic.id, DigestStage::Synthesis, 1, 1, "归并完成");

    // 落盘 + 收尾。
    let count = final_articles.len();
    write_articles(
        &app,
        &root_dir,
        day,
        topic.id,
        &channel_label,
        final_articles,
    )?;
    report(
        &app,
        topic.id,
        DigestStage::Done,
        1,
        1,
        format!("出刊完成：{count} 篇（{channel_label}）"),
    );
    let _ = app.emit(
        "intel://updated",
        &IntelUpdatedEvent {
            topic_id: topic.id.to_owned(),
        },
    );
    Ok(())
}

/// 筛选结果解析（docs/17 §7.5.1-2/3）：输出格式 `[{"i":序号,"score":n,"reason":"…"}]`。
/// 返回 (条目, score)。解析失败 = 全保留（score 3 兜底，保旧语义）；缺失序号 =
/// 显式 drop（记日志，不再静默消失）。
fn triage_scores(batch: &[FeedItem], raw: &str) -> Vec<(FeedItem, u8)> {
    let Some(value) = super::llm::extract_json(raw) else {
        crate::logging::log("warn", "情报筛选输出解析失败，本批全部保留");
        return batch.iter().map(|item| (item.clone(), 3)).collect();
    };
    let Some(entries) = value.as_array() else {
        return batch.iter().map(|item| (item.clone(), 3)).collect();
    };
    let mut scores: Vec<Option<u8>> = vec![None; batch.len()];
    for entry in entries {
        let Some(idx) = entry.get("i").and_then(|v| v.as_u64()) else {
            continue;
        };
        let idx = idx as usize;
        if idx >= batch.len() {
            continue; // 幻觉序号：忽略。
        }
        let score = entry
            .get("score")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, 5) as u8)
            .unwrap_or(3);
        scores[idx] = Some(score);
    }
    let missing = scores.iter().filter(|s| s.is_none()).count();
    if missing > 0 {
        crate::logging::log(
            "warn",
            &format!("情报筛选覆盖率不足：{missing} 条无 verdict，按丢弃处理"),
        );
    }
    batch
        .iter()
        .zip(scores)
        .filter_map(|(item, score)| score.map(|s| (item.clone(), s)))
        .filter(|(_, score)| *score >= 3)
        .collect()
}

/// 深加工 JSON → Article（字段缺失兜底，避免单篇解析炸掉整轮）。
/// importance 由 triage 的全局 score 注入（rewrite 不再自行打分）；
/// topic_id 参数化（当前硬编码 "ai"，多主题接入即错——docs/17 §7.5.1-8）。
fn article_from_json(
    item: &FeedItem,
    topic_id: &str,
    model: &str,
    value: &Value,
    score: u8,
) -> std::result::Result<Article, Error> {
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| Error::Intel("缺少 title".to_owned()))?
        .to_owned();
    let lead = value
        .get("lead")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_owned();
    let body: Vec<String> = value
        .get("body")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str())
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    // 正文空则回退导语；两者皆空 = 空壳文章，算失败。
    let body = if body.is_empty() && !lead.is_empty() {
        vec![lead.clone()]
    } else {
        body
    };
    if body.is_empty() {
        return Err(Error::Intel("正文与导语皆空".to_owned()));
    }
    let tags = value
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(Article {
        id: format!("a-{}", item.id),
        topic_id: topic_id.to_owned(),
        title,
        lead,
        body,
        tags,
        importance: score.clamp(1, 5),
        sources: vec![ArticleSource {
            item_id: item.id.clone(),
            source_id: item.source_id.clone(),
            url: item.url.clone(),
            title: item.title.clone(),
        }],
        model: model.to_owned(),
        created_at: chrono::Utc::now().timestamp_millis(),
    })
}

/// 归并结果应用（docs/17 §7.5.1-4）：模型只输出合并组 `[{members, headline}]`。
/// 合并篇 = 组内 importance 最高者为骨架，headline 覆盖标题，组员 sources 并集，
/// id = 组内 itemId 排序拼接的确定性后缀（重出刊 id 稳定，收藏不失联）。
/// 解析失败/空组 → 草稿原样。
fn apply_synthesis(drafts: &[Article], raw: &str) -> Vec<Article> {
    let groups: Vec<(Vec<String>, String)> = match super::llm::extract_json(raw)
        .as_ref()
        .and_then(Value::as_array)
    {
        Some(entries) => entries
            .iter()
            .filter_map(|entry| {
                let members: Vec<String> = entry
                    .get("members")
                    .and_then(Value::as_array)?
                    .iter()
                    .filter_map(|m| m.as_str().map(str::to_owned))
                    .collect();
                if members.len() < 2 {
                    return None; // 单成员组无意义。
                }
                let headline = entry
                    .get("headline")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                Some((members, headline))
            })
            .collect(),
        None => {
            crate::logging::log("warn", "情报归并输出解析失败，按未合并出刊");
            Vec::new()
        }
    };
    if groups.is_empty() {
        return drafts.to_vec();
    }
    let mut merged_ids: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<Article> = Vec::new();
    for (members, headline) in &groups {
        let group: Vec<&Article> = members
            .iter()
            .filter_map(|id| drafts.iter().find(|d| &d.id == id))
            .collect();
        if group.len() < 2 {
            continue; // 组员不足 2：放弃该组（成员照常独立出刊）。
        }
        // 骨架 = importance 最高（同分取 id 序，保证确定性）。
        let skeleton = group
            .iter()
            .max_by_key(|a| (a.importance, std::cmp::Reverse(a.id.clone())))
            .expect("group 非空");
        let mut sources = skeleton.sources.clone();
        for article in &group {
            for source in &article.sources {
                if !sources.iter().any(|s| s.item_id == source.item_id) {
                    sources.push(source.clone());
                }
            }
        }
        // 确定性 id：组内 item_id 排序拼接取尾部 16 位（跨重跑稳定）。
        let mut item_ids: Vec<&str> = group
            .iter()
            .map(|a| a.sources[0].item_id.as_str())
            .collect();
        item_ids.sort_unstable();
        let joined = item_ids.concat();
        let suffix: String = joined
            .chars()
            .rev()
            .take(16)
            .collect::<Vec<_>>()
            .iter()
            .rev()
            .collect();
        out.push(Article {
            id: format!("m-{suffix}"),
            title: if headline.is_empty() {
                skeleton.title.clone()
            } else {
                headline.clone()
            },
            sources,
            ..(*skeleton).clone()
        });
        for article in &group {
            merged_ids.insert(article.id.clone());
        }
    }
    // 未被合并的草稿保持独立。
    for draft in drafts {
        if !merged_ids.contains(&draft.id) {
            out.push(draft.clone());
        }
    }
    // 出刊顺序：重要性降序。
    out.sort_by(|a, b| {
        b.importance
            .cmp(&a.importance)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// 抓取任意 URL 原文（供正文抽取）。返回 (sourceKind, text)：
/// sourceKind 决定 extract 的解析通道（reddit JSON / arxiv abstract / 通用 HTML）。
async fn fetch_raw(client: &reqwest::Client, url: &str) -> Option<(String, String)> {
    let (fetch_url, kind) = if url.contains("reddit.com") {
        (format!("{url}.json"), "reddit-ml".to_owned())
    } else if url.contains("arxiv.org/abs") || url.contains("abs.arxiv.org") {
        (url.to_owned(), "arxiv".to_owned())
    } else {
        (url.to_owned(), String::new())
    };
    let resp = client
        .get(&fetch_url)
        .header("User-Agent", "qianxun-intel/0.7 (desktop; personal)")
        .send()
        .await
        .ok()?;
    let text = resp.text().await.ok()?;
    Some((kind, text))
}

/// 落盘文章集（原子写）。
fn write_articles(
    _app: &AppHandle,
    root_dir: &std::path::Path,
    day: &str,
    topic_id: &str,
    model: &str,
    articles: Vec<Article>,
) -> std::result::Result<(), String> {
    store::write_articles_day(
        root_dir,
        day,
        topic_id,
        &ArticlesDayFile {
            fetched_at: chrono::Utc::now().timestamp_millis(),
            model: model.to_owned(),
            articles,
        },
    )
    .map_err(|cause| format!("文章落盘失败：{cause}"))
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> FeedItem {
        FeedItem {
            id: id.to_owned(),
            source_id: "arxiv".to_owned(),
            title: format!("标题 {id}"),
            summary: "摘要".to_owned(),
            url: format!("https://arxiv.org/abs/{id}"),
            heat: None,
            published_at: None,
            first_seen_at: 0,
        }
    }

    fn draft(id: &str, importance: u8) -> Article {
        Article {
            id: format!("a-{id}"),
            topic_id: "ai".to_owned(),
            title: format!("文章 {id}"),
            lead: "导语".to_owned(),
            body: vec!["段落".to_owned()],
            tags: vec![],
            importance,
            sources: vec![ArticleSource {
                item_id: id.to_owned(),
                source_id: "arxiv".to_owned(),
                url: format!("https://arxiv.org/abs/{id}"),
                title: format!("原文 {id}"),
            }],
            model: "test".to_owned(),
            created_at: 0,
        }
    }

    #[test]
    fn triage解析短序号且覆盖率不足显式丢弃() {
        let batch: Vec<FeedItem> = (0..3).map(|i| item(&format!("id{i}"))).collect();
        let raw = r#"废话 [{"i":1,"score":5,"reason":"旗舰"},{"i":0,"score":2,"reason":"小版本"}]"#;
        let scored = triage_scores(&batch, raw);
        // id0 score2 被过滤；id1 score5 保留；id2 无 verdict 显式丢弃。
        assert_eq!(scored.len(), 1);
        assert_eq!(scored[0].0.id, "id1");
        assert_eq!(scored[0].1, 5);
    }

    #[test]
    fn triage解析失败兜底全保留() {
        let batch: Vec<FeedItem> = (0..2).map(|i| item(&format!("id{i}"))).collect();
        let scored = triage_scores(&batch, "完全不是 JSON");
        assert_eq!(scored.len(), 2);
        assert!(scored.iter().all(|(_, s)| *s == 3));
    }

    #[test]
    fn 归并分组决策聚合来源且id确定() {
        let drafts = vec![draft("x1", 5), draft("x2", 3), draft("x3", 4)];
        let raw = r#"[{"members":["a-x1","a-x2"],"headline":"合并标题"}]"#;
        let out = apply_synthesis(&drafts, raw);
        // x1+x2 合并（骨架 = importance 5 的 x1），x3 独立。
        assert_eq!(out.len(), 2);
        let merged = out
            .iter()
            .find(|a| a.id.starts_with("m-"))
            .expect("应有合并篇");
        assert_eq!(merged.title, "合并标题");
        assert_eq!(merged.importance, 5);
        assert_eq!(merged.sources.len(), 2);
        // 确定性 id：同输入重跑 id 不变。
        let again = apply_synthesis(&drafts, raw);
        assert_eq!(
            merged.id,
            again.iter().find(|a| a.id.starts_with("m-")).unwrap().id
        );
    }

    #[test]
    fn 深加工body空回退导语且score注入() {
        let value: Value =
            serde_json::from_str(r#"{"title":"标题","lead":"导语一句","body":[],"tags":["ai"]}"#)
                .unwrap();
        let article = article_from_json(&item("x9"), "ai", "m", &value, 4).expect("应成功");
        assert_eq!(article.body, vec!["导语一句"]);
        assert_eq!(article.importance, 4);
        assert_eq!(article.topic_id, "ai");
        // 两者皆空 = 失败。
        let empty: Value = serde_json::from_str(r#"{"title":"标题","lead":"","body":[]}"#).unwrap();
        assert!(article_from_json(&item("x9"), "ai", "m", &empty, 4).is_err());
    }
}

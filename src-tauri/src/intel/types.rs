//! 情报域数据类型（docs/17 §6）。serde camelCase 与前端 contract.ts
//! 字段级锁步（contract.test.ts 双向集合比对，RESPONSE_TYPES 抽查三类）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Feed 条目：标题/摘要/链接 + 热度元数据。
/// 原始字段永不改写（M1 无 LLM 增列；M2 的 zhTitle/zhSummary 届时以
/// 新字段追加，同样不动这里）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    /// sha2(sourceId + 归一化 url)：同源幂等去重主键。
    pub id: String,
    /// 源标识（徽章显示 + 抓取状态归因）。
    pub source_id: String,
    /// 原文标题，永不改写。
    pub title: String,
    /// 来源 URL，常驻显示，点击跳原站。
    pub url: String,
    /// 源自带的原始摘要（RSS description 等；无则空串）。
    #[serde(default)]
    pub summary: String,
    /// 热度/排名（得分或榜单名次；无热度语义的源为 null）。
    #[serde(default)]
    pub heat: Option<u64>,
    /// 发布时间（毫秒；源未给 = 抓取时刻）。
    #[serde(default)]
    pub published_at: Option<i64>,
    /// 首次入库时间（毫秒）：同 id 刷新只更新热度，此值不变。
    pub first_seen_at: i64,
}

// Design Arena 单榜位、指标出处、模型家族条目等榜单类型已在 M3 移除
// （docs/17 §7.6：榜单是「列表」不是「新闻」，模型主题改为评测新闻流）。

/// Feed 主题的一天（days/{date}/{topic}.json 信封）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedDayFile {
    pub fetched_at: i64,
    pub items: Vec<FeedItem>,
}

/// 每源抓取状态（state.json）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceState {
    /// 最近一次成功抓取的本地日期（yyyy-MM-dd）。「当天一次」闸门。
    #[serde(default)]
    pub last_fetch_date: String,
    #[serde(default)]
    pub last_ok_at: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
}

/// state.json 顶层。
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelStateFile {
    #[serde(default)]
    pub sources: BTreeMap<String, SourceState>,
}

/// 收藏条目：深拷贝快照（不引用天文件），独立于历史清理。
/// 旧版本可能含 model 字段（榜单时代）：serde 默认忽略未知字段，读取不炸。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteEntry {
    /// feed.id / article.id。
    pub id: String,
    pub topic_id: String,
    pub saved_at: i64,
    pub feed: Option<FeedItem>,
    /// 出刊文章快照（docs/17 §7.4）。
    #[serde(default)]
    pub article: Option<Article>,
}

/// intel://updated 事件负载。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelUpdatedEvent {
    pub topic_id: String,
}

// ---------------------------------------------------------------------------
// Agent 编辑部出刊（docs/17 §7.4）
// ---------------------------------------------------------------------------

/// 文章的原始来源引用（多源归并时多条；原文 URL 永不丢）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleSource {
    /// 原 FeedItem.id（可回溯原始条目）。
    pub item_id: String,
    pub source_id: String,
    pub url: String,
    /// 原文标题（永不改写）。
    pub title: String,
}

/// 加工成品文章（days/{date}/{topic}.articles.json）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    /// sha2(日期 + topic + 归并条目 id 集)：稳定幂等。
    pub id: String,
    pub topic_id: String,
    /// Agent 撰写的中文标题。
    pub title: String,
    /// 导语（一两句，列表页直接显示）。
    pub lead: String,
    /// 正文段落（阅读视图顺序呈现）。
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Agent 评的重要性 1-5（5 = 头条）。
    pub importance: u8,
    /// 来源引用（单源 1 条，多源归并 >1 条）。
    pub sources: Vec<ArticleSource>,
    /// 加工模型（provider/model，溯源）。
    pub model: String,
    pub created_at: i64,
}

/// ArticlesDayFile 信封。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticlesDayFile {
    pub fetched_at: i64,
    /// 出刊使用的模型（provider/model）；llm-unavailable 时为空串。
    pub model: String,
    pub articles: Vec<Article>,
}

/// 出刊阶段（DigestProgress.stage）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DigestStage {
    /// 尚未开始（今天还没抓取，或非 Feed 主题）。
    Idle,
    Fetching,
    Triage,
    Bodies,
    Rewriting,
    Synthesis,
    Done,
    Failed,
    /// LLM 通道不可用（DSH 未运行 / 未配置 provider / 无 key）。
    LlmUnavailable,
}

/// intel://digest 事件负载 + intel_digest_status 返回值：
/// 状态弹框的每一步进度都从这里来（docs/17 §7.4「进度可见」）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestProgress {
    pub topic_id: String,
    pub stage: DigestStage,
    /// 当前阶段已完成步数（bodies/rewriting 的 n/total）。
    pub done: u32,
    /// 当前阶段总步数。
    pub total: u32,
    /// 人类可读的当前动作或失败原因（一行）。
    pub message: String,
    /// 出刊启动时间（毫秒；idle 时 null）。
    pub started_at: Option<i64>,
}

/// 一个正在进行的 HTTP 请求（intel://fetching 事件项）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceActivity {
    /// 源标识（徽章 / 状态页短名）。
    pub source_id: String,
    /// 正在请求的 URL（HF models 为代表页；出刊取正文同理）。
    pub url: String,
}

/// intel://fetching 事件负载：某主题当前并发中的请求列表。
/// 开始/结束各推一次；active 为空 = 该主题抓取结束。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelFetchingEvent {
    pub topic_id: String,
    pub active: Vec<SourceActivity>,
}

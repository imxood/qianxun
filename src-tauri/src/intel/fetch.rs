//! 源适配器（docs/17 §5）：每源一个纯函数，输入 HTTP 客户端 + 源定义，
//! 输出 `Vec<FeedItem>`。单源失败返回 String 错误（进 state.json 展示），
//! 绝不拖垮同主题其它源。
//!
//! XML 解析是有意的轻量做法：目标源（arXiv Atom / WordPress RSS）结构
//! 稳定，`regex`（已有依赖）抽块 + 实体解码足够，不引入 feed 解析 crate。

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::topics::{SourceDef, SourceKind};
use super::types::FeedItem;

/// 单源抓取超时。
const REQUEST_TIMEOUT_SECS: u64 = 20;
/// Feed UA：Reddit 等对无 UA 客户端硬拒。
const USER_AGENT: &str = "qianxun-intel/0.7 (desktop; personal)";
/// RSS 条目新鲜度窗口（docs/17 §7.6-6）：HF blog 等全量 feed 含长历史，
/// 旧文混进当日原料会让「今日要览」名不副实。
const RSS_FRESH_DAYS: i64 = 14;

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|cause| format!("HTTP 客户端构建失败：{cause}"))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// 条目主键：sha2(sourceId + 归一化 url)，同源重复抓到只更新热度。
pub(crate) fn item_id(source_id: &str, url: &str) -> String {
    let normalized = url.trim().trim_end_matches('/');
    let mut hasher = Sha256::new();
    hasher.update(source_id.as_bytes());
    hasher.update(b"|");
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 折叠空白 + 截断（char 安全）。
pub(crate) fn clean_summary(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(max_chars).collect();
    format!("{cut}…")
}

fn parse_ms(text: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(text) {
        return Some(dt.timestamp_millis());
    }
    None
}

// ---- 轻量 XML 助手（目标源结构稳定，不做通用解析器） ----

/// 取 `<tag ...>body</tag>` 的 body（不处理嵌套同名标签——目标源没有）。
pub(crate) fn tag_body<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let start = text.find(&open)?;
    let after_open = &text[start..];
    let gt = after_open.find('>')?;
    let rest = &after_open[gt + 1..];
    let close = format!("</{tag}>");
    let end = rest.find(&close)?;
    Some(&rest[..end])
}

/// 取标签首个属性值（如 `<link href="..">` 的 href）。
fn tag_attr(text: &str, tag: &str, attr: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = text.find(&open)?;
    let after_open = &text[start..];
    let gt = after_open.find('>')?;
    let open_tag = &after_open[..=gt];
    let needle = format!("{attr}=\"");
    let attr_start = open_tag.find(&needle)? + needle.len();
    let rest = &open_tag[attr_start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// 解 CDATA + 基础实体 + 折叠空白。
pub(crate) fn xml_text(text: &str) -> String {
    let inner = {
        let trimmed = text.trim();
        if trimmed.starts_with("<![CDATA[") && trimmed.ends_with("]]>") {
            &trimmed[9..trimmed.len() - 3]
        } else {
            trimmed
        }
    };
    let decoded = inner
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    collapsed(&decoded)
}

pub(crate) fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- Feed 源 ----

/// 分发 Feed 源。client 按值传递：嵌套 async fn 全所有权参数，
/// 绕开 spawn 的 `Send + 'static` 推断限制（rustc #100013）。
pub async fn fetch_feed_source(
    client: reqwest::Client,
    source: &'static SourceDef,
) -> Result<Vec<FeedItem>, String> {
    match source.kind {
        SourceKind::Arxiv => fetch_atom(client, source).await,
        SourceKind::Rss => fetch_rss(client, source).await,
        SourceKind::Reddit => fetch_reddit(client, source).await,
        SourceKind::HfPapers => fetch_hf_papers(client, source).await,
    }
}

async fn get_text(client: reqwest::Client, url: &'static str) -> Result<String, String> {
    client
        .get(url)
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|cause| format!("请求失败：{cause}"))?
        .text()
        .await
        .map_err(|cause| format!("读响应失败：{cause}"))
}

/// arXiv Atom API：search_query 已在 endpoint 里（cs.AI/CL/LG，按提交时间）。
async fn fetch_atom(
    client: reqwest::Client,
    source: &'static SourceDef,
) -> Result<Vec<FeedItem>, String> {
    let text = get_text(client, source.endpoint).await?;
    let mut items = Vec::new();
    for block in text.split("<entry>").skip(1) {
        let Some(end) = block.find("</entry>") else {
            continue;
        };
        let entry = &block[..end];
        let Some(title) = tag_body(entry, "title") else {
            continue;
        };
        let title = xml_text(title);
        if title.is_empty() {
            continue;
        }
        let link = tag_attr(entry, "link", "href")
            .or_else(|| tag_body(entry, "id").map(str::to_owned))
            .unwrap_or_default();
        if link.is_empty() {
            continue;
        }
        let summary = tag_body(entry, "summary").map(|body| clean_summary(&xml_text(body), 200));
        let published = tag_body(entry, "published").and_then(|raw| parse_ms(&collapsed(raw)));
        items.push(FeedItem {
            id: item_id(source.id, &link),
            source_id: source.id.to_owned(),
            title,
            url: link,
            summary: summary.unwrap_or_default(),
            heat: None,
            published_at: Some(published.unwrap_or_else(now_ms)),
            first_seen_at: now_ms(),
        });
    }
    if items.is_empty() {
        return Err("响应里没有条目（接口可能已变化）".to_owned());
    }
    Ok(items)
}

/// RSS 2.0（量子位 / HF blog / the-decoder / Solidot / hnrss，均为标准形态）。
async fn fetch_rss(
    client: reqwest::Client,
    source: &'static SourceDef,
) -> Result<Vec<FeedItem>, String> {
    let text = get_text(client, source.endpoint).await?;
    parse_rss_items(source, &text)
}

/// RSS 解析内核（HTTP 层剥离，单测直测这里）。
/// 丢弃发布超过 RSS_FRESH_DAYS 天的条目（docs/17 §7.6-6）。
fn parse_rss_items(source: &SourceDef, text: &str) -> Result<Vec<FeedItem>, String> {
    let fresh_floor = now_ms() - RSS_FRESH_DAYS * 86_400_000;
    let mut items = Vec::new();
    for block in text.split("<item>").skip(1) {
        let Some(end) = block.find("</item>") else {
            continue;
        };
        let item = &block[..end];
        let Some(title) = tag_body(item, "title") else {
            continue;
        };
        let title = xml_text(title);
        if title.is_empty() {
            continue;
        }
        let Some(link) = tag_body(item, "link") else {
            continue;
        };
        let link = collapsed(link);
        if link.is_empty() {
            continue;
        }
        let published = tag_body(item, "pubDate")
            .or_else(|| tag_body(item, "dc:date"))
            .and_then(|raw| parse_ms(&collapsed(raw)));
        // 新鲜度：源给了发布时间且明显过期的条目直接跳过（无发布时间视为新鲜）。
        if let Some(at) = published {
            if at < fresh_floor {
                continue;
            }
        }
        let summary = tag_body(item, "description").map(|body| clean_summary(&xml_text(body), 200));
        items.push(FeedItem {
            id: item_id(source.id, &link),
            source_id: source.id.to_owned(),
            title,
            url: link,
            summary: summary.unwrap_or_default(),
            heat: None,
            published_at: Some(published.unwrap_or_else(now_ms)),
            first_seen_at: now_ms(),
        });
    }
    if items.is_empty() {
        return Err("响应里没有条目（接口可能已变化）".to_owned());
    }
    Ok(items)
}

#[derive(Default, Deserialize)]
struct RedditListing {
    #[serde(default)]
    data: RedditListingData,
}

#[derive(Default, Deserialize)]
struct RedditListingData {
    #[serde(default)]
    children: Vec<RedditChild>,
}

#[derive(Default, Deserialize)]
struct RedditChild {
    #[serde(default)]
    data: RedditPost,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
struct RedditPost {
    #[serde(default)]
    title: String,
    #[serde(default)]
    permalink: String,
    #[serde(default)]
    score: i64,
    #[serde(default)]
    created_utc: Option<f64>,
    #[serde(default)]
    selftext: String,
}

async fn fetch_reddit(
    client: reqwest::Client,
    source: &'static SourceDef,
) -> Result<Vec<FeedItem>, String> {
    let listing: RedditListing = client
        .get(source.endpoint)
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|cause| format!("请求失败：{cause}"))?
        .json()
        .await
        .map_err(|cause| format!("解析 JSON 失败：{cause}"))?;
    let mut items = Vec::new();
    for child in &listing.data.children {
        let post = &child.data;
        if post.title.is_empty() || post.permalink.is_empty() {
            continue;
        }
        let url = format!("https://www.reddit.com{}", post.permalink);
        items.push(FeedItem {
            id: item_id(source.id, &url),
            source_id: source.id.to_owned(),
            title: clean_summary(&post.title, 200),
            url,
            summary: clean_summary(&post.selftext, 200),
            heat: Some(post.score.max(0) as u64),
            published_at: post.created_utc.map(|secs| (secs as i64) * 1000),
            first_seen_at: now_ms(),
        });
    }
    if items.is_empty() {
        return Err("响应里没有条目（接口可能已变化）".to_owned());
    }
    Ok(items)
}

#[derive(Deserialize)]
struct HfPaperEntry {
    #[serde(default)]
    paper: Option<HfPaper>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    upvotes: Option<i64>,
}

#[derive(Deserialize)]
struct HfPaper {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

/// HF Daily Papers（已实测：顶层 title/summary 与 paper.title 冗余，取顶层）。
async fn fetch_hf_papers(
    client: reqwest::Client,
    source: &'static SourceDef,
) -> Result<Vec<FeedItem>, String> {
    let entries: Vec<HfPaperEntry> = client
        .get(source.endpoint)
        .send()
        .await
        .and_then(|resp| resp.error_for_status())
        .map_err(|cause| format!("请求失败：{cause}"))?
        .json()
        .await
        .map_err(|cause| format!("解析 JSON 失败：{cause}"))?;
    let mut items = Vec::new();
    for entry in entries {
        let title = entry
            .title
            .or(entry.paper.as_ref().and_then(|p| p.title.clone()));
        let summary = entry
            .summary
            .or(entry.paper.as_ref().and_then(|p| p.summary.clone()));
        let paper_id = entry.paper.as_ref().and_then(|p| p.id.clone());
        let (Some(title), Some(paper_id)) = (title, paper_id) else {
            continue;
        };
        if title.is_empty() {
            continue;
        }
        let url = format!("https://huggingface.co/papers/{paper_id}");
        items.push(FeedItem {
            id: item_id(source.id, &url),
            source_id: source.id.to_owned(),
            title: clean_summary(&title, 200),
            url,
            summary: summary
                .as_deref()
                .map(|s| clean_summary(s, 200))
                .unwrap_or_default(),
            heat: Some(entry.upvotes.unwrap_or(0).max(0) as u64),
            published_at: None,
            first_seen_at: now_ms(),
        });
    }
    if items.is_empty() {
        return Err("响应里没有条目（接口可能已变化）".to_owned());
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 条目id同源同url稳定() {
        let a = item_id("arxiv", "http://arxiv.org/abs/2609.40222");
        let b = item_id("arxiv", "http://arxiv.org/abs/2609.40222/");
        assert_eq!(a, b, "尾斜杠归一化后应同 id");
        let c = item_id("reddit-ml", "http://arxiv.org/abs/2609.40222");
        assert_ne!(a, c, "不同源不同 id");
    }

    #[test]
    fn xml助手抽块与实体解码() {
        let rss = r#"<item><title><![CDATA[量子位：GLM-5.3 很强 &amp; 稳]]></title><link>
            https://www.qbitai.com/2026/10/500098.html
        </link></item>"#;
        assert_eq!(
            xml_text(tag_body(rss, "title").unwrap()),
            "量子位：GLM-5.3 很强 & 稳"
        );
        assert_eq!(
            tag_body(rss, "link").map(collapsed).as_deref(),
            Some("https://www.qbitai.com/2026/10/500098.html")
        );
        let atom = r#"<entry><link rel="alternate" type="text/html" href="http://arxiv.org/abs/1v1" /></entry>"#;
        assert_eq!(
            tag_attr(atom, "link", "href").as_deref(),
            Some("http://arxiv.org/abs/1v1")
        );
    }

    #[test]
    fn clean_summary折叠并截断() {
        let long = "词 ".repeat(300);
        let out = clean_summary(long.trim(), 10);
        assert!(out.chars().count() == 11, "10 字 + 省略号");
        assert!(out.ends_with('…'));
    }

    #[test]
    fn rss解析完整条目并过滤过期() {
        let src = SourceDef {
            id: "qbitai",
            endpoint: "https://www.qbitai.com/feed",
            kind: SourceKind::Rss,
        };
        let body = r#"<?xml version="1.0"?><rss><channel>
            <item>
                <title>标题一</title>
                <link>https://www.qbitai.com/a.html</link>
                <pubDate>Fri, 02 Oct 2026 07:34:15 +0000</pubDate>
                <description><![CDATA[摘要一]]></description>
            </item>
            <item>
                <title>标题二</title>
                <link>https://www.qbitai.com/b.html</link>
            </item>
            <item>
                <title>旧文</title>
                <link>https://www.qbitai.com/old.html</link>
                <pubDate>Mon, 01 Jan 2024 00:00:00 +0000</pubDate>
            </item>
        </channel></rss>"#;
        let items = parse_rss_items(&src, body).unwrap();
        assert_eq!(items.len(), 2, "2024 年旧文应被新鲜度过滤丢弃");
        assert_eq!(items[0].title, "标题一");
        assert_eq!(items[0].url, "https://www.qbitai.com/a.html");
        assert_eq!(items[0].summary, "摘要一");
        assert!(items[0].published_at.is_some());
        assert_eq!(items[1].summary, "");
        assert!(items[1].published_at.is_some(), "缺省发布时间回退抓取时刻");
    }
}

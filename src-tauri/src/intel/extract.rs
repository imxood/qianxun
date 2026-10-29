//! 原文正文抽取（docs/17 §7.4 第 2 段）：按源适配，失败退 RSS 摘要。
//! M2a 从简：通用 HTML 主文本块抽取 + 三个源专属通道（arXiv/Reddit/通用页）。

use super::fetch::{collapsed, tag_body, xml_text};

/// 抽取出的正文文本（超限已截断）。
pub struct Extracted {
    pub text: String,
}

/// 正文原料上限（字符）：超过截断——加工不需要全文长尾。
/// 正文原料上限（字符）。2026-10-03 质量修订 8000→12000：原料越充分，
/// 长文的事实密度越高（12000 字符 ≈ 6-8k tokens 输入，成本可忽略）。
const BODY_LIMIT: usize = 12_000;

/// Reddit JSON（.json API）→ selftext + 前置热评。
fn reddit_body(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let post = value
        .get(0)?
        .get("data")?
        .get("children")?
        .get(0)?
        .get("data")?;
    let mut parts: Vec<String> = Vec::new();
    if let Some(selftext) = post.get("selftext").and_then(|v| v.as_str()) {
        let cleaned = selftext.trim();
        if !cleaned.is_empty() && cleaned != "[removed]" && cleaned != "[deleted]" {
            parts.push(cleaned.to_owned());
        }
    }
    // 热评前 3 条（点赞序已由 reddit 排序）。
    if let Some(comments) = value
        .get(1)
        .and_then(|c| c.get("data"))
        .and_then(|d| d.get("children"))
        .and_then(|c| c.as_array())
    {
        let mut taken = 0;
        for child in comments {
            if taken >= 3 {
                break;
            }
            let Some(body) = child
                .get("data")
                .and_then(|d| d.get("body"))
                .and_then(|b| b.as_str())
            else {
                continue;
            };
            let cleaned = body.trim();
            if cleaned.is_empty() || cleaned == "[removed]" || cleaned == "[deleted]" {
                continue;
            }
            parts.push(format!("热评：{cleaned}"));
            taken += 1;
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// HTML → 主文本块（从简：去 head/script/style/nav，剥全部标签，压空白）。
/// 不做可读性算法——源站结构相对稳定，全文文本足够 Agent 提炼。
fn html_to_text(html: &str) -> String {
    let lower = html.to_lowercase();
    let mut text = String::with_capacity(html.len() / 2);
    // 逐段跳过 <script>/<style>/<head>/<nav>/<header>/<footer>/<aside>。
    let skip_tags = [
        "script", "style", "head", "nav", "header", "footer", "aside", "noscript",
    ];
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // 是否进入跳过段。
            let rest = &lower[i..];
            let Some(tag_end_rel) = rest.find('>') else {
                break;
            };
            let tag_end = i + tag_end_rel;
            let open_name = rest[1..tag_end_rel]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('/')
                .to_owned();
            if skip_tags.contains(&open_name.as_str()) && !rest[tag_end_rel..].starts_with("</") {
                // 跳到对应闭标签之后。
                let close = format!("</{open_name}");
                match lower[tag_end..].find(&close) {
                    Some(rel) => {
                        let close_abs = tag_end + rel;
                        i = lower[close_abs..]
                            .find('>')
                            .map(|p| close_abs + p + 1)
                            .unwrap_or(html.len());
                        continue;
                    }
                    None => break, // 未闭合：余下全弃。
                }
            }
            text.push(' '); // 标签边界当空白。
            i = tag_end + 1;
            continue;
        }
        // 普通字符：整段复制到下一个 '<'。
        let segment_end = lower[i..].find('<').map(|p| i + p).unwrap_or(html.len());
        text.push_str(&html[i..segment_end]);
        i = segment_end;
    }
    // HTML 实体（常见几个，够用）+ 空白压缩。
    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    collapsed(decoded.trim())
}

/// 按源抽取正文：sourceKind 决定通道（reddit JSON / arxiv abstract / 通用 HTML），
/// raw 是已拉取的响应文本。
pub fn extract_body(source_id: &str, _url: &str, raw: &str) -> Option<Extracted> {
    let text = match source_id {
        "reddit-ml" | "reddit-localllama" => reddit_body(raw),
        "arxiv" => tag_body(raw, "blockquote")
            .map(xml_text)
            .filter(|t| !t.trim().is_empty()),
        _ => {
            // 通用 HTML（量子位等 RSS 源的文章页）。
            let plain = html_to_text(raw);
            if plain.len() < 200 {
                None // 空壳页（反爬/重定向），退摘要。
            } else {
                Some(plain)
            }
        }
    }?;
    let cut: String = text.chars().take(BODY_LIMIT).collect();
    Some(Extracted { text: cut })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reddit自文本与热评() {
        let json = r#"[
          {"data":{"children":[{"data":{"selftext":"正文内容","title":"t"}}]}},
          {"data":{"children":[
            {"data":{"body":"第一条热评"}},
            {"data":{"body":"[removed]"}},
            {"data":{"body":"第二条热评"}}
          ]}}
        ]"#;
        let body = reddit_body(json).unwrap();
        assert!(body.starts_with("正文内容"));
        assert!(body.contains("热评：第一条热评"));
        assert!(!body.contains("[removed]"));
    }

    #[test]
    fn html剥标签与script() {
        let html = r#"<html><head><title>x</title><style>.a{}</style></head>
            <body><nav>菜单</nav><article><h1>标题</h1>
            <p>第一段 &amp; 符号。</p><script>evil()</script>
            <p>第二段</p></article><footer>页脚</footer></body></html>"#;
        let text = html_to_text(html);
        assert!(text.contains("第一段 & 符号。"));
        assert!(text.contains("第二段"));
        assert!(!text.contains("evil"));
        assert!(!text.contains("页脚"));
        assert!(!text.contains("菜单"));
    }

    #[test]
    fn 截断到上限() {
        let long = "字".repeat(30_000);
        let extracted = extract_body("qbitai", "https://x", &format!("<p>{long}</p>")).unwrap();
        assert_eq!(extracted.text.chars().count(), BODY_LIMIT);
    }

    #[test]
    fn 空壳页返回空() {
        assert!(extract_body("qbitai", "https://x", "<html><body>ok</body></html>").is_none());
    }
}

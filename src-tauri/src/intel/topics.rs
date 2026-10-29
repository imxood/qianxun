//! 主题与源注册表（docs/17 §4/§7.6）：加主题 = 这里加一个 TopicDef，
//! 源走 fetch.rs 的通用适配器（Rss/Arxiv/Reddit/HfPapers），前端卡片动态渲染。
//! 所有主题都是杂志流（M3 起无榜单形态）；源准入制：先实测可达再收录。

pub const TOPIC_AI: &str = "ai";
pub const TOPIC_MODEL_NEWS: &str = "model-news";
pub const TOPIC_TECH: &str = "tech";

/// 源适配器类型（fetch.rs 按 kind 分发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// arXiv Atom API。
    Arxiv,
    /// RSS 2.0（量子位 / HF blog / the-decoder / Solidot / hnrss）。
    Rss,
    /// Reddit 某版的 hot.json。
    Reddit,
    /// Hugging Face Daily Papers。
    HfPapers,
}

/// 一个数据源。endpoint 为最终 URL（模板占位由适配器内部展开）。
pub struct SourceDef {
    pub id: &'static str,
    pub endpoint: &'static str,
    pub kind: SourceKind,
}

/// 主题定义（全部为 Feed 杂志流；M3 移除榜单形态，docs/17 §7.6）。
pub struct TopicDef {
    pub id: &'static str,
    pub label: &'static str,
    pub sources: &'static [SourceDef],
}

const ARXIV_ENDPOINT: &str = "http://export.arxiv.org/api/query?search_query=cat:cs.AI+OR+cat:cs.CL+OR+cat:cs.LG&sortBy=submittedDate&sortOrder=descending&max_results=40";

const AI_SOURCES: &[SourceDef] = &[
    SourceDef {
        id: "arxiv",
        endpoint: ARXIV_ENDPOINT,
        kind: SourceKind::Arxiv,
    },
    SourceDef {
        id: "hf-papers",
        endpoint: "https://huggingface.co/api/daily_papers",
        kind: SourceKind::HfPapers,
    },
    SourceDef {
        id: "reddit-ml",
        endpoint: "https://www.reddit.com/r/MachineLearning/hot.json?limit=30",
        kind: SourceKind::Reddit,
    },
    SourceDef {
        id: "reddit-localllama",
        endpoint: "https://www.reddit.com/r/LocalLLaMA/hot.json?limit=30",
        kind: SourceKind::Reddit,
    },
    SourceDef {
        id: "qbitai",
        endpoint: "https://www.qbitai.com/feed",
        kind: SourceKind::Rss,
    },
];

/// 模型评测主题（docs/17 §7.6：榜单 → 评测新闻/报告；均实测可达）。
const MODEL_NEWS_SOURCES: &[SourceDef] = &[
    SourceDef {
        id: "hf-blog",
        endpoint: "https://huggingface.co/blog/feed.xml",
        kind: SourceKind::Rss,
    },
    SourceDef {
        id: "the-decoder",
        endpoint: "https://the-decoder.com/feed/",
        kind: SourceKind::Rss,
    },
];

/// 科技主题（Solidot 中文科技 + HN 高分热帖；BBC/联合早报/36kr 实测不可达）。
const TECH_SOURCES: &[SourceDef] = &[
    SourceDef {
        id: "solidot",
        endpoint: "https://www.solidot.org/index.rss",
        kind: SourceKind::Rss,
    },
    SourceDef {
        id: "hn",
        endpoint: "https://hnrss.org/newest?points=200",
        kind: SourceKind::Rss,
    },
];

/// 主题注册表（M3）：AI · 模型评测 · 科技，全部走编辑部流水线。
pub static TOPICS: &[TopicDef] = &[
    TopicDef {
        id: TOPIC_AI,
        label: "AI",
        sources: AI_SOURCES,
    },
    TopicDef {
        id: TOPIC_MODEL_NEWS,
        label: "模型评测",
        sources: MODEL_NEWS_SOURCES,
    },
    TopicDef {
        id: TOPIC_TECH,
        label: "科技",
        sources: TECH_SOURCES,
    },
];

pub fn topic(id: &str) -> Option<&'static TopicDef> {
    TOPICS.iter().find(|def| def.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 注册表主题与源齐备() {
        let ai = topic(TOPIC_AI).expect("ai 主题应在注册表");
        assert_eq!(ai.sources.len(), 5);
        let evals = topic(TOPIC_MODEL_NEWS).expect("model-news 主题应在注册表");
        assert_eq!(evals.sources.len(), 2);
        let tech = topic(TOPIC_TECH).expect("tech 主题应在注册表");
        assert_eq!(tech.sources.len(), 2);
        assert!(topic("models").is_none(), "旧榜单主题已移除（M3）");
        assert!(topic("hot").is_none());
        // 源 id 全局唯一（state.json 按源 id 记状态）。
        let mut ids: Vec<&str> = TOPICS
            .iter()
            .flat_map(|t| t.sources.iter().map(|s| s.id))
            .collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "源 id 不得重复");
    }
}

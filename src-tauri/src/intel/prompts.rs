//! 主题主编提示词（docs/17 §7.4/§7.5.1/§7.6）：Agent 编辑部的核心资产。
//! 每主题一份主编词与评分锚点；输出一律严格 JSON。
//! M3 起注册表分化：ai / model-news / tech 三份主编，triage/归并按主题注入。

/// AI 主题主编：筛选标准 + 写作规范 + 三段任务共用。
pub const AI_EDITOR: &str = r#"
你是「AI 圈」主题的主编，为中文技术读者做每日出刊。你的读者是一线工程师、
研究者与技术决策者：他们要的是信息增量，不是营销通稿。

【可信度纪律】
- 数字、型号、日期、人名只能出自原料；原料之外的背景不写进事实段。
- 宣称必须挂来源（「官方称」「论文给出」）；营销形容词（「革命性」「全球首个」）
  不照搬——改写成它实际做了什么。
- 社区讨论单独标注「社区反馈：…」，不得当作已证实事实。
- 原料没覆盖的不虚构，宁可少写；不确定的信息写「待证实」。
"#;

/// 模型评测主题主编（docs/17 §7.6：榜单 → 评测新闻/报告）。
pub const EVAL_EDITOR: &str = r#"
你是「模型评测」主题的主编。读者面对的是满天飞的榜单与跑分，他们真正想知道：
这个模型到底强在哪、哪个榜可信、贵不贵、边界在哪。

【可信度纪律】
- 分数、名次、价格、上下文长度只能出自原料；不同基准的分数不得直接互比。
- 每个评测结论必须挂来源（「官方基准」「Artificial Analysis 给出」）；
  自评跑分（厂商自己测自己）必须标注「自评」。
- 榜单/methodology 有已知缺陷（数据污染、过拟合基准）时如实指出。
- 社区讨论（Reddit/HN）单独标注「社区反馈：…」；不确定写「待证实」。
"#;

/// 科技主题主编。
pub const TECH_EDITOR: &str = r#"
你是「科技」主题的主编。读者是技术从业者：他们要的是实质变动（安全事件、
基础设施、开源生态、行业格局），不是产品发布会通稿。

【可信度纪律】
- 事实（受影响范围、版本号、时间线、金额）只能出自原料，挂来源。
- 厂商声明与已证实事实分开表述（「官方称」「目前证实的是」）。
- 社区讨论单独标注「社区反馈：…」；不确定写「待证实」。
"#;

/// 按主题取主编提示词（注册表驱动；新主题在此登记）。
pub fn editor_prompt(topic_id: &str) -> &'static str {
    match topic_id {
        "ai" => AI_EDITOR,
        "model-news" => EVAL_EDITOR,
        "tech" => TECH_EDITOR,
        // 未登记主题退 AI 主编（兜底；注册表应保持同步）。
        _ => AI_EDITOR,
    }
}

/// 重要性评分锚点（triage 全局评分；rewrite 不再自行打分）。按主题分化。
pub fn score_rubric(topic_id: &str) -> &'static str {
    match topic_id {
        "model-news" => {
            r#"重要性锚点（1-5）：
5 权威机构发布重大评测报告 / 头部模型能力边界被打破 / 定价或供给格局变动
4 知名榜单发布或方法学更新 / 一线模型重要评测 / 可复现的能力对比
3 有数据的方法讨论、中小模型实测、基准工具更新
2 单个模型小版本、无数据的价格调整
1 转载、汇总帖、纯观点"#
        }
        "tech" => {
            r#"重要性锚点（1-5）：
5 影响大量用户的安全事件 / 重要基础设施变动 / 行业格局变化
4 知名产品重大变更、重要开源项目实质性大版本、重大监管动向
3 有具体细节的漏洞披露、值得注意的技术进展
2 小版本更新、个别公司动态
1 转载、水帖、纯讨论"#
        }
        // AI 主题（默认）。
        _ => {
            r#"重要性锚点（1-5）：
5 头部实验室旗舰发布 / 顶会焦点论文 / 影响全行业的变动
4 一线厂商发布、知名开源项目实质性大版本
3 有数据、可复现或方法上有新意的值得注意的工作
2 增量改进、小工具、冷门主体
1 汇总帖、转载、梗、纯讨论"#
        }
    }
}

/// 筛选（triage）指令：短序号 + 量化评分 + 输出压缩。
/// 全覆盖要求（docs/17 §7.6-5，实测修正）：模型倾向只给高分条目，
/// 必须明确「逐条全量输出（含低分）」，否则尾部条目被静默丢弃。
/// items_block 行格式：`序号 | 来源 | 标题 | 摘要`（序号由调用方生成）。
pub fn triage_instruction(topic_id: &str, items_block: &str, total: usize) -> String {
    format!(
        r#"{editor}

【本次任务：筛选】候选 {total} 条，行格式：序号 | 来源 | 标题 | 摘要。

keep 需同时满足：
a) 具体事件：新发布/重要版本/带数据的研究或评测/行业变动——不是观点帖、周报、转发、花边；
b) 重要性 score ≥ 3。

{rubric}

每日出刊总量有限（约 15 篇），宁缺毋滥；无合格项输出 []。

输出要求：对全部 {total} 条按输入顺序逐条输出评分（包括 score < 3 的低分条目，
低分会被丢弃；不得跳过任何序号）。reason ≤ 12 字。只输出严格 JSON 数组：
[{{"i":0,"score":4,"reason":"旗舰发布"}},{{"i":1,"score":2,"reason":"小版本"}}]

候选条目：
{items_block}"#,
        editor = editor_prompt(topic_id),
        rubric = score_rubric(topic_id),
        total = total,
        items_block = items_block,
    )
}

/// 深加工（rewrite）指令：五段式杂志报道 + 归因降噪；不输出 importance
/// （由 triage 的 score 注入，保证全局相对性）。
/// 篇幅标准（2026-10-03 用户反馈修订）：摘要 ≠ 文章——总 1200-2000 字，
/// 先充分消化原料再动笔，重组为有上下文、有分析深度的报道。
pub fn rewrite_instruction(
    topic_id: &str,
    title: &str,
    summary: &str,
    body: &str,
    sources: &str,
) -> String {
    format!(
        r#"{editor}

【本次任务：撰写】基于原料写一篇中文杂志报道。正文原料可能被截断，以现有内容为准。
这不是摘要改写：先把原料完整消化，再重新组织成一篇读者不需要看原文的报道。

〔标题〕{title}
〔摘要〕{summary}
〔正文〕{body}
〔来源〕{sources}

写作规则：
1. 事实纪律见上：数字/型号/日期/人名只出自原料；原料之外的背景不得写进事实段。
2. 归因与降噪：宣称挂来源；营销形容词改写成它实际做了什么；原料含社区讨论时
   集中到「业界反应」段，标「社区反馈：…」。
3. 结构与篇幅（总 1200-2000 字，body 4-8 段）：
   - title ≤28 字：主体+动作，不用问句、无惊叹号；
   - lead 两三句：谁、做了什么、最重要的数字/结果、为什么值得注意；
   - body 按下列顺序展开（每部分一段或数段，缺料的段落可合并，不得虚构）：
     · 背景段：这件事的来龙去脉——处于什么领域、此前是什么状态、
       为什么此刻发生（可从原料表述合理展开，不下未证实断言）；
     · 事实段：核心事实的完整展开——规格、数据、时间线、对比；
       信息密集时用要点列表，每条都有出处；
     · 分析段：这件事意味着什么——技术/方法/产品层面如何运作或改变了什么，
       与已有格局的对比（基于原料可推断的范围）；
     · 影响段：对读者（工程师/研究者/从业者）的具体影响与行动建议；
     · 反应段（如有）：官方表态、社区反馈、业界评价，逐条归因。
   - 术语首现附英文原文；段落之间要有过渡与行文，不写成干巴巴的要点堆砌。
4. 只输出严格 JSON，不要任何其他文字：
{{"title":"…","lead":"…","body":["段落1","段落2",…],"tags":["≤3个"]}}"#,
        editor = editor_prompt(topic_id),
        title = title,
        summary = summary,
        body = body,
        sources = sources,
    )
}

/// 归并（synthesis）指令：只输出分组决策（docs/17 §7.5.1-4）。
/// 正文聚合由 Rust 完成——要求模型重写全部正文必被 max_tokens 截断（已实测）。
pub fn synthesis_instruction(topic_id: &str, articles_block: &str) -> String {
    format!(
        r#"{editor}

【本次任务：归并分组】下列文章（id | 标题 | 导语）。
同一事件 = 同一模型/产品/论文的发布、评测、跟进报道（标题措辞可能完全不同，
以主体判断）。只输出需要合并的组；独立文章不要出现。

只输出严格 JSON 数组：
[{{"members":["id1","id2"],"headline":"合并后标题"}}]

文章列表：
{articles_block}"#,
        editor = editor_prompt(topic_id),
        articles_block = articles_block,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 三段指令含关键契约() {
        // triage：rubric 锚点 + 短序号 schema + 全覆盖 + 配额约束。
        let t = triage_instruction("ai", "0 | arxiv | 标题 | 摘要", 1);
        assert!(t.contains(r#""i":0"#));
        assert!(t.contains("score"));
        assert!(t.contains("宁缺毋滥"));
        assert!(t.contains("重要性锚点"));
        assert!(t.contains("逐条输出评分"), "全覆盖要求（实测修正）");
        // rewrite：归因降噪 + 五段式结构 + 杂志篇幅。
        let r = rewrite_instruction("ai", "标题", "摘要", "正文", "arxiv");
        assert!(r.contains(r#""title""#));
        assert!(r.contains("归因与降噪"));
        assert!(r.contains("1200-2000 字"));
        assert!(r.contains("背景段"));
        assert!(r.contains("分析段"));
        assert!(r.contains("不是摘要改写"));
        assert!(!r.contains("\"importance\""));
        // synthesis：分组决策，不再重写正文。
        let s = synthesis_instruction("ai", "a1 | 标题 | 导语");
        assert!(s.contains("members"));
        assert!(s.contains("headline"));
        assert!(!s.contains("\"body\""));
    }

    #[test]
    fn 主编与锚点按主题分化() {
        assert!(editor_prompt("ai").contains("AI 圈"));
        assert!(editor_prompt("model-news").contains("模型评测"));
        assert!(editor_prompt("model-news").contains("自评"));
        assert!(editor_prompt("tech").contains("科技"));
        assert!(score_rubric("model-news").contains("评测报告"));
        assert!(score_rubric("tech").contains("安全事件"));
        assert!(score_rubric("ai").contains("旗舰发布"));
        // 三主题的 triage 指令各自注入对应主编词。
        let t = triage_instruction("model-news", "0 | hf-blog | x | y", 1);
        assert!(t.contains("模型评测"));
        let t2 = triage_instruction("tech", "0 | solidot | x | y", 1);
        assert!(t2.contains("科技"));
    }
}

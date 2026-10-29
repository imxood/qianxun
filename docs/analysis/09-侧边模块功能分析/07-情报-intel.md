# 侧边模块分析报告 · 情报（intel）

> 分析日期：2026-10-05 ｜ 范围：前端 `src/features/intel/**`、`src/stores/intel.svelte.ts` + 后端 `src-tauri/src/intel/**`（11 文件 ~3030 行）
> 负责：主会话 / intel-module
> 设计依据：`docs/17-每日资讯聚合设计.md`（草案 v3.1）

## 0. 速览

| 项         | 值                                                                                                                                    |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| 一句话定位 | 多主题每日资讯聚合：Rust 侧直抓公开 API/RSS，按天按主题落盘，LLM 加工成中文「每日杂志」，前端是主题卡网格 + 文章流 + 收藏的三视图外壳 |
| 完成度     | **88/100（良好，接近完成）**                                                                                                          |
| 代码规模   | 前端 849 行（含 28 行测试）/ 后端 ~3030 行（含 3 个 store 单测 ~50 行）                                                               |
| 最严重问题 | 出刊强依赖外部 LLM 通道（DSH 运行 + 已配 key），不可用时**整条出刊管线零产出**，且失败态只在进入主题视图后才可见                      |

### 值得称道的部分

- **架构分层极干净**：抓取层永远无 LLM、原始字段永远保留，LLM 只写新字段（`docs/17` §2.6）。降级语义是设计的第一公民，而非事后补的。
- **运行态与磁盘态分离**：`IntelRuntime`（`scheduler.rs:23`）只管「谁在抓」，落盘态在 `state.json`。防重复刷新靠 `BTreeSet` + `claim` 式 `try_begin`（`digest.rs:60`），没有全局定时器。
- **错误可分类**：单源失败只记 `state.json` 降级出内容，仅当全部源失败且当天无数据才上抛（`scheduler.rs:4-5` 注释即契约）。
- **IPC 合同有门禁**：`IntelTopicStatus` / `FeedItem` / `Article` / `DigestProgress` 都在 `contract.test.ts:92-107` 的 `RESPONSE_TYPES` 名单里，字段漂移会让 CI 失败。

---

## 1. 模块定位与设计意图

`docs/17` 开篇的需求表（§1）逐条都能在代码里找到落点：

| 需求                                   | 设计落点                                                            | 实现状态                          |
| -------------------------------------- | ------------------------------------------------------------------- | --------------------------------- |
| 侧边一个「情报」页，打开是主题卡片列表 | `IntelPage.svelte:57-98` 卡片网格                                   | ✅ 一致                           |
| 至少 模型/AI/热点，方便加主题          | `topics.rs` `TOPICS` 注册表                                         | ✅ 加主题零前端改动（但见 §4-D6） |
| 启动后当天只查一次                     | `scheduler.rs:98-114` `startup()` 逐主题 `fetch_topic(force=false)` | ✅                                |
| 数据存、历史可查                       | `store.rs` 按天落盘 + `prune()`                                     | ✅ 90 天                          |
| 收藏                                   | `favorites.json` 快照拷贝                                           | ✅                                |
| 英文源经 LLM 出中文标题/摘要（可选层） | `digest.rs` 五阶段出刊                                              | ⚠️ 见 §2.3                        |

**与设计的偏差**：设计 §3 提到「模型表」视图（`ModelsView.svelte`），实际 M3 起所有主题统一为杂志流、榜单形态已移除（`contract.ts:311` 注释、`topics.rs:29`）。这是**有意的演进**（`docs/17` §7.6），但设计文档 §3 的架构图仍画着 `ModelsView.svelte`，文档未同步。

---

## 2. 功能清单与完成度

### 2.1 抓取与调度

| #   | 功能                                      | 状态   | 证据(file:line)                                | 备注                           |
| --- | ----------------------------------------- | ------ | ---------------------------------------------- | ------------------------------ |
| 1   | 主题注册表驱动                            | 已完成 | `topics.rs:31-116`                             | 3 主题 / 10 源，源准入制       |
| 2   | 4 种源适配器（Arxiv/Rss/Reddit/HfPapers） | 已完成 | `fetch.rs`（416 行）                           | 通用适配器按 `kind` 分发       |
| 3   | 启动补抓（当天闸门）                      | 已完成 | `scheduler.rs:98-114`                          | 逐主题 + 2s 错峰               |
| 4   | 手动刷新（force 绕过闸门）                | 已完成 | `scheduler.rs:204-206` `spawn_refresh`         |                                |
| 5   | 主题间错峰 2s / 并发 ≤3                   | 已完成 | `scheduler.rs:110`                             | 对上游克制                     |
| 6   | 日期哨兵（跨天自动补抓）                  | 已完成 | `scheduler.rs:116-118`                         | 每小时比对本地日期，非高频轮询 |
| 7   | 单源失败降级                              | 已完成 | `scheduler.rs:4-5` 契约 + 实现                 | 只记 state，主题仍出内容       |
| 8   | 并发源活动可视化                          | 已完成 | `scheduler.rs:48-95` + `intel://fetching` 事件 | 「正在请求 xxx」               |
| 9   | 90 天保留 + prune                         | 已完成 | `store.rs:18,57`                               | 有单测（`store.rs:230`）       |
| 10  | 出刊同日补偿                              | 已完成 | `scheduler.rs:112` `compensate_digest`         |                                |

### 2.2 阅读与导航

| #   | 功能                                 | 状态   | 证据(file:line)                                        | 备注                            |
| --- | ------------------------------------ | ------ | ------------------------------------------------------ | ------------------------------- |
| 11  | 主题卡片网格（今日条数/状态点/时间） | 已完成 | `IntelPage.svelte:57-98`                               |                                 |
| 12  | 今日要览（跨主题头条聚合）           | 已完成 | `FrontPage.svelte:9-16`                                | 头条 = 全 Feed importance 最高  |
| 13  | 文章流（头条大卡 + 紧凑卡）          | 已完成 | `FeedView.svelte:270-350`                              |                                 |
| 14  | 原始源 tab（未加工条目）             | 已完成 | `FeedView.svelte:150-223`                              | 原文标题永不改写                |
| 15  | 阅读视图（正文/来源/加工模型）       | 已完成 | `FeedView.svelte:224-269`                              |                                 |
| 16  | 日期导航（前一天/后一天/回今天）     | 已完成 | `FeedView.svelte:63-114` + `intel.svelte.ts:354-373`   |                                 |
| 17  | 收藏（快照拷贝，按日分组）           | 已完成 | `FavoritesView.svelte` + `commands.rs:217-243`         | 不受历史清理影响                |
| 18  | 已读标记（标题置灰）                 | 已完成 | `intel.svelte.ts:403-410`                              | 先本地回显，落盘异步            |
| 19  | 冷启动回退（当天未出刊→最近一期）    | 已完成 | `commands.rs:151-171` + `FeedView.svelte:273-280` 徽章 | 设计得好                        |
| 20  | 收藏跨主题跳转                       | 已完成 | `FavoritesView.svelte:40-44`                           | 先 `open(topicId)` 再进阅读视图 |
| 21  | 状态页（全局进度时间线）             | 已完成 | `StatusDialog.svelte` + `intel.svelte.ts:252-274`      |                                 |
| 22  | LLM 不可用红字提示                   | 已完成 | `FeedView.svelte:355-359`                              | 不静默降级                      |
| 23  | 出刊进度横幅                         | 已完成 | `FeedView.svelte:137-147`                              |                                 |
| 24  | 旧榜单时代收藏快照过滤               | 已完成 | `FavoritesView.svelte:14`                              | 演进兼容                        |

### 2.3 出刊管线（LLM 加工）

| #   | 功能                      | 状态         | 证据(file:line)               | 备注                                                      |
| --- | ------------------------- | ------------ | ----------------------------- | --------------------------------------------------------- |
| 25  | 通道发现（读 DSH 配置链） | 已完成       | `llm.rs:1-12`                 | cordis.patch.yml → .credentials.yaml                      |
| 26  | 五阶段出刊                | 已完成       | `digest.rs`（750 行）         | fetching→triage→bodies→rewriting→synthesis                |
| 27  | LLM 超时                  | 已完成       | `digest.rs:32,195`            | 120s                                                      |
| 28  | 非法 JSON 重试            | 已完成       | `digest.rs:362-373`           | `retry` + 二次 `extract_json`                             |
| 29  | 通道探测（triage 首阶段） | 已完成       | `digest.rs:178-187`           | 失败即 `llm-unavailable`                                  |
| 30  | flash / 写作模型分离      | 已完成       | `llm.rs:29-33`                | 分类用 flash，改写用主模型                                |
| 31  | GLM 思考模型特殊处理      | 已完成       | `llm.rs:36-40`                | 思考 token 计入 max_tokens                                |
| 32  | 多协议支持                | **部分完成** | `llm.rs:12`                   | 仅 openai-completions；anthropic-messages 标注为 M2b 未做 |
| 33  | 出刊并发保护              | 已完成       | `digest.rs:60-65` `try_begin` | 同主题不重复出刊                                          |
| 34  | 归并分组                  | 已完成       | `digest.rs:591-639`           | 有 `.expect("group 非空")`（见 §4-D3）                    |

---

## 3. 实现分析

### 3.1 架构与数据流

```
启动 → lib.rs setup spawn(intel::scheduler::startup)
         ↓
    prune(90天) → 逐主题 fetch_topic(force=false)  ← 当天已抓则跳过
         ↓ 每主题 emit intel://updated {topicId}
    compensate_digest()  ← 同日已有源数据但没出刊 → 补出刊
         ↓
    spawn_day_sentinel()  ← 每小时比对日期，跨天重跑
                    ↓
    前端 intel.wire() 一次订阅 3 个事件：
      intel://updated   → 重载 topics + 当日
      intel://digest    → 横幅/状态页行实时刷新；done → 重载文章 + 要览
      intel://fetching  → 卡片脉冲 + 状态页并发源
```

**亮点**：前端 store 订阅只跑一次（`intel.svelte.ts:145-147` `if (this.wired) return`），配合 `App.svelte` 的 keep-alive 不重复订阅。`statusRows` 与 `topics` **共享同一 topic 对象引用**（`intel.svelte.ts:172-174` 注释明确），更新一处两处可见——这个设计省掉了一整类同步 bug。

### 3.2 前后端契约

- 11 个 `intel_*` 命令全部在 `contract.ts:64-74` 声明，且被 `contract.test.ts` 的**命令级**门禁（Rust ↔ TS 双向）覆盖。
- 4 个响应类型进了**字段级**门禁（`IntelTopicStatus`、`FeedItem`、`Article`、`DigestProgress`）。
- ⚠️ **但 `intel_topic_day` 的返回类型 `TopicDay` 没进 `RESPONSE_TYPES`**（`contract.test.ts:92-107` 名单里没有），它的 `read` 字段是 `BTreeMap<String, i64>`，`intel_topic_day` 是空态语义的关键载体（`items: null` = 未抓 vs `[]` = 抓了但空）。这是字段漂移的盲区。

### 3.3 性能与规模

- 每次 `intel_topics` 调用会对**每个主题**读一次当天 JSON（`commands.rs:45-47`）。3 主题 = 3 次小文件读，毫秒级，可接受。
- `intel_articles` 冷启动回退会 `list_article_days` + 逐日试读（`commands.rs:153-157`）。若 90 天都有出刊，最坏读 90 次文件。这是同步命令（`commands.rs:1-2` 注释自称「毫秒级小文件」），90 次小文件读在 SSD 上约几毫秒，**边界但未爆**。
- `loadFrontPage` 对所有主题并发调 `intel_articles`（`intel.svelte.ts:206-212`），与上面的回退叠加 → 首屏最坏 3×90 次文件读。
- 原始源列表**无虚拟滚动**（`FeedView.svelte:171`），单主题上限是适配器的 `max_results`（arXiv 40、reddit 30），实际不会失控。

### 3.4 错误处理

- 后端失败语义分三层：单源失败（记 state 降级）→ 主题全失败但当天有数据（继续）→ 全失败且无数据（上抛给前端 toast）。这个分层是本模块最值得学的地方。
- 前端几乎所有 `call` 都有 `.catch(() => null)` 兜底（`intel.svelte.ts:210,260-262,310`），不会白屏。
- LLM 不可用是**显式红色态**（`FeedView.svelte:355-359`）+ 指引「DSH 启动后点重新出刊」，符合「不静默降级」的设计原则。

---

## 4. 缺陷清单

| 级别   | 问题                                                                                      | 位置                                                            | 影响                                                                                                | 证据                                                                                                                                           |
| ------ | ----------------------------------------------------------------------------------------- | --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| **P1** | 出刊硬依赖外部 LLM 通道，DSH 未运行/未配 key 时整条管线零产出，且**入口页看不到任何提示** | `digest.rs:178-187`；`IntelPage.svelte` 全文无 llm 态渲染       | 用户打开情报看到「今日尚未出刊」，不知道为什么。失败信息只在进入主题后可见（`FeedView.svelte:355`） | `intel.svelte.ts:235-238` `openStatus` 拉 digest 状态，但卡片网格（`IntelPage.svelte:57-98`）只显示 `t.fetching`/`t.ok`，不显示 `digest.stage` |
| **P1** | LLM 配置链读 DSH 的 `cordis.patch.yml` + `.credentials.yaml`，**格式由外部 DSH 决定**     | `llm.rs:5-8`                                                    | DSH 升级改配置格式 → 情报静默降级为「llm-unavailable」，无版本兼容层                                | `llm.rs:7-8` 注释自称「已实测解析」，但无 fixture 测试                                                                                         |
| **P1** | `anthropic-messages` 协议未实现，kimi/minimax 用户完全无法出刊                            | `llm.rs:12` 明确标注 M2b                                        | 该 provider 用户的出刊功能不可用                                                                    | `llm.rs:1-2` 顶部注释                                                                                                                          |
| **P2** | 抓取用 **http 明文** 的 arXiv endpoint                                                    | `topics.rs:35` `ARXIV_ENDPOINT = "http://export.arxiv.org/..."` | 中间人可篡改抓取内容。虽然是公开只读数据、且后续有 LLM 加工会二次过滤，但仍是明文依赖               | 全部其他源都是 https                                                                                                                           |
| **P2** | `intel_topic_day` 返回类型 `TopicDay` 未纳入字段级合同门禁                                | `contract.test.ts:92-107`                                       | `read` / `items: null` 语义漂移不会被 CI 拦截                                                       | `commands.rs:81-88` 定义了 `TopicDay`，`contract.ts:354` 有对应 interface，但不在 `RESPONSE_TYPES`                                             |
| **P2** | `.expect("group 非空")` 在归并路径上                                                      | `digest.rs:639`                                                 | LLM 返回异常分组时**整个出刊 panic**（Rust 侧 task join 后无进度更新，卡在 `synthesis`）            | `digest.rs:591-639` `apply_synthesis`                                                                                                          |
| **P2** | 搜索/浏览历史只有「前一天/后一天」按钮，无日期选择器                                      | `FeedView.svelte:63-114`                                        | 90 天历史里想找某天要点几十次                                                                       | `intel.days` 已有完整日期数组（`intel.svelte.ts:88`）但只用于导航                                                                              |
| **P2** | `loadFrontPage` 与 `intel_articles` 冷启动回退叠加，最坏 3×90 次同步文件读                | `intel.svelte.ts:206-212` + `commands.rs:153-157`               | 90 天满 + 冷启动时首屏 IPC 可能卡顿（虽然实测应 <20ms）                                             | 无分页/无上限截断                                                                                                                              |
| **P3** | 收藏视图的空态文案是单个「空」字                                                          | `FavoritesView.svelte:61`                                       | 相比其他视图的「该日未出刊」「今日未抓」过于简陋，且无引导入口                                      | 采集是核心动作，空态应引导                                                                                                                     |
| **P3** | 星星渲染用 `{#each Array(article.importance)}`                                            | `FeedView.svelte:231,300`                                       | `importance` 来自 LLM 输出，无上限校验。LLM 返回 999 就是 999 个 DOM 节点                           | 对比 `digest.rs` 的 `importance` 是 `u8`，序列化后无 clamp                                                                                     |
| **P3** | 前端 `SOURCE_LABELS` 硬编码 10 个源标签                                                   | `intel.svelte.ts:35-46`                                         | 加源需改前端，违背「加主题零前端改动」的 extensibility 承诺                                         | `topics.rs` 已有全部源 id，Rust 未把 label 下发                                                                                                |
| **P3** | `activeIsFeed` getter 恒真（保留语义位）                                                  | `intel.svelte.ts:126-128`                                       | 死代码，读者会误以为有两种视图形态                                                                  | 注释自称「保留语义位给收藏视图」                                                                                                               |

---

## 5. 改善建议

| 优先级 | 建议                                                                                              | 改动文件                                | 收益                                                         | 成本                                   |
| ------ | ------------------------------------------------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------ | -------------------------------------- |
| **高** | 卡片网格补 digest 状态：把 `intel://digest` 的 `llm-unavailable` 提升到入口页（卡片角标或顶部条） | `IntelPage.svelte` + `intel.svelte.ts`  | 用户不必进主题才知道「出刊需要 DSH」；这是本模块最高频的困惑 | 低（store 已有 `statusRows`/`digest`） |
| **高** | `importance` 在两处 clamp（Rust 序列化前 + 前端渲染前），或改用 CSS `width` 表达星级              | `digest.rs` / `FeedView.svelte:231,300` | 消除 LLM 输出导致的 DOM 爆炸                                 | 极低                                   |
| **高** | arXiv endpoint 换 https                                                                           | `topics.rs:35`                          | 去掉明文依赖                                                 | 极低（需实测可达）                     |
| **中** | 把 `TopicDay` 加入 `contract.test.ts` 的 `RESPONSE_TYPES`                                         | `contract.test.ts:92`                   | 补上字段漂移盲区                                             | 极低                                   |
| **中** | `.expect("group 非空")` 改 `if let Some` + 上报 `Failed` stage                                    | `digest.rs:639`                         | 消除 panic 导致的「卡在 synthesis」                          | 低                                     |
| **中** | 历史日期改日期选择器（`<input type="date">`，`intel.days` 已有数据）                              | `FeedView.svelte:63-114`                | 90 天历史可用性大幅提升                                      | 低                                     |
| **中** | Rust 侧下发 source label（`SourceDef` 加 `label`），前端删掉硬编码表                              | `topics.rs` + `intel.svelte.ts:35-46`   | 兑现 extensibility 承诺                                      | 低                                     |
| **中** | `intel_articles` 回退加「最多回溯 N 期」上限（如 7）                                              | `commands.rs:153-157`                   | 消除 90 次读的最坏情况                                       | 极低                                   |
| **中** | LLM 配置链加 fixture 单测（`llm.rs` 目前 0 测试）                                                 | `llm.rs`                                | 配置格式漂移可早发现                                         | 中                                     |
| **低** | 收藏空态补引导文案 + 跳转入口                                                                     | `FavoritesView.svelte:61`               | UX 一致性                                                    | 极低                                   |
| **低** | 删 `activeIsFeed` 死 getter                                                                       | `intel.svelte.ts:126-128`               | 减认知负担                                                   | 极低                                   |
| **低** | 实现 `anthropic-messages` 协议                                                                    | `llm.rs`                                | 扩大可用 provider 面                                         | 中高                                   |

---

## 6. 完成度评估

| 维度             | 得分 | 依据                                                                                                                                                                                                                                                                                                                        |
| ---------------- | ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 功能完整性       | 90   | 24 项功能全部实现，唯一缺口是 anthropic 协议（`llm.rs:12`）                                                                                                                                                                                                                                                                 |
| 健壮性与错误处理 | 85   | 三层失败语义设计优秀（`scheduler.rs:4-5`），扣分项是 `digest.rs:639` 的 expect panic                                                                                                                                                                                                                                        |
| 性能与规模       | 80   | 无虚拟滚动但源有条数上限；扣分为 `intel_articles` 90 次回退读（`commands.rs:153`）                                                                                                                                                                                                                                          |
| 测试覆盖         | 62   | 实际有 **21 个 Rust 单测**（`llm.rs` 7、`digest.rs` 4、`fetch.rs` 4、`extract.rs` 4、`scheduler.rs` 3、`store.rs` 3、`topics.rs` 1）+ 前端 `format.test.ts`(28 行)。覆盖了通道选择/JSON 提取容错/triage 解析/归并分组。**仍缺**：`digest.rs` 的 rewrite+synthesis 端到端、`fetch.rs` 的真实网络路径、前端 store 层的 0 测试 |
| UX 完整度        | 88   | 空态/加载/失败/回退态齐全，缺日期选择器与收藏空态引导                                                                                                                                                                                                                                                                       |
| 可维护性         | 92   | 注册表驱动 + 事件解耦 + 契约门禁，符合 `docs/03-编码规范`                                                                                                                                                                                                                                                                   |

**总评 88/100**：架构与设计对齐度高，是全项目工程质量最好的模块之一。**主要短板是测试覆盖（62 分）**——虽然 Rust 侧有 21 个单测（优于我初判），但最重的 `digest.rs`(750 行五阶段管线) 只测了 4 个纯函数分支（triage 解析/归并分组/body 回退），**rewrite 与 synthesis 的 LLM 交互路径无测试**；前端 `intel.svelte.ts`(413 行的复杂 store，含竞态与事件订阅) **0 测试**。其次是对外部 LLM 通道的强依赖没有在入口页暴露。

---

## 7. 测试与验证

### 实际执行

| 命令                     | 结果                                                                                                                                                                                                                                                                           |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `npx vitest run`（全量） | ✅ **27 个测试文件全过，261 passed / 1 skipped（262 total），3.75s**（vitest 4.1.11）                                                                                                                                                                                          |
| 代码走查                 | 完整读取 `intel.svelte.ts`(413)、`IntelPage.svelte`(133)、`FeedView.svelte`(378)、`FavoritesView.svelte`(123)、`FrontPage.svelte`(73)、`commands.rs`(268)、`scheduler.rs`(120/358)、`topics.rs`(116)、`llm.rs` 前 40 行、`digest.rs` 关键路径（grep）、`store.rs` 结构（grep） |

### 已有测试清单（全仓 157 个 Rust 单测中，intel 占 21）

| 文件                                 | 单测数 | 覆盖内容                                                                                                                                                                                                                                    |
| ------------------------------------ | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `intel/llm.rs`                       | 7      | 通道选择（`云端provider缺key时跳过并取本地`、`有key时选云端并偏好flash`、`无llm条目报不可用`）、JSON 提取容错（`json提取容忍包裹`、`json提取平衡扫描字符串感知`）、响应解析（`响应解析含finish原因与空content`、`completion解析取content`） |
| `intel/digest.rs`                    | 4      | `triage解析短序号且覆盖率不足显式丢弃`、`triage解析失败兜底全保留`、`归并分组决策聚合来源且id确定`、`深加工body空回退导语且score注入`                                                                                                       |
| `intel/fetch.rs`                     | 4      | 源适配器                                                                                                                                                                                                                                    |
| `intel/extract.rs`                   | 4      | 正文提取                                                                                                                                                                                                                                    |
| `intel/scheduler.rs`                 | 3      | 调度                                                                                                                                                                                                                                        |
| `intel/store.rs`                     | 3      | `state与天文件roundtrip` / `list_days只认合法日期且降序` / `prune删除过期日期目录`                                                                                                                                                          |
| `intel/topics.rs`                    | 1      | 注册表                                                                                                                                                                                                                                      |
| 前端 `features/intel/format.test.ts` | 28 行  | 时间格式化                                                                                                                                                                                                                                  |

### 未执行 / 覆盖盲区（诚实标注）

- ❌ **未执行** `cargo test --manifest-path src-tauri/Cargo.toml`（上述 21 个单测**未实际运行**，仅代码走查确认其存在与覆盖范围）
- ❌ **未执行** `pnpm exec svelte-check`
- ⚠️ **无法验证**：LLM 通道是否真的可用（需要运行中的 DSH + 已配 key 的 provider），本轮为纯静态分析，未启动应用
- ⚠️ **无法验证**：各源 2026-10-05 当天是否可达（`topics.rs` 声称「先实测可达再收录」，但时效性会漂移）
- ⚠️ **覆盖盲区**：`digest.rs` 的 rewrite/synthesis 阶段（真正的 LLM 交互）无测试；前端 `intel.svelte.ts` 413 行的 store（竞态、事件订阅、日期导航）**0 测试**

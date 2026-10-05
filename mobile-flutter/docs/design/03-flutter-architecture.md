# 03 · Flutter UI 层技术架构

> 千寻移动端（`qianxun_mobile`）界面层的设计规格。
> 范围：会话列表、会话转录、输入栏、审批/提问弹层、实时活动指示器。
> 本文只定**架构与技术手段**，不定视觉。视觉在 01/02 里定；本文引用但不重复。
>
> 现有网络层（gateway client / JSON-RPC / stream mux / 重连退避）**已完工且可跑**，
> 本文不改它。本文只在其上规定 widget 组合、状态归属、动画实现与可测性契约。

---

## 0. 术语与速查

| 词 | 含义 |
| --- | --- |
| **行 row** | 转录里的一行（`TranscriptRow` 的一个实例），不是「一行文本」 |
| **桶 bucket** | 会话列表按状态分的组：`blocked` / `running` / `idle` |
| **durable 事件** | 落在 `session/follow` 日志里、已定稿的事件（`SessionWireEvent`） |
| **delta 帧** | `assistant-stream` 帧，未定稿的增量文字 |
| **锚点 anchor** | 滚动位置锚定的那个行 |
| **粘底 stick-to-bottom** | 贴住滚动底部（视口底部 160px 内）时自动跟随新内容 |

---

## 1. 约束（现有代码强加的）

### 1.1 数据模型给定的形状

| # | 代码事实（引用） | ⇒ 对 UI 的强制要求 |
| --- | --- | --- |
| 1 | `SessionSummary` **没有 `title`，也没有 `createdAt`**。标题只在 `projections.values["title"]`（`session.dart:266`）；`displayTitle`（:273）回退链是 `title → cwdBasename → '未命名会话'`；`metaCwd`（:288）有标题时返回 `null` | 列表行**不得**假设 `session.title` 存在；同一条 cwd 不得在标题与副行各出现一次 |
| 2 | `SessionSummary` 是 `class` 不是 `record`，**无 `==`/`hashCode`** | 每次 `fromJson` 的新实例对 Riverpod「必然变了」⇒ 增量只能靠 **key + 引用相等**，不能靠 state 相等性 |
| 3 | `running`（:205）与 `agentAvailable`（:218）**是两回事**；后者为 false 即 `isDetached`（:263），点进去进不去 | 列表至少三档语义（运行中/空闲/已断开），不能按 running/blank 二分 |
| 4 | `turnOutline`（:247）/ `lastPrompt`（:252）是**宿主给的现成预览** | 不要自己从转录猜摘要 |
| 5 | `PendingInteraction` 的身份是 waterfall 的 `eventId`（:396）；`ApprovalRequest` **没有审批 id**（:377） | 审批/提问的 widget key 只能是 `eventId`，不能用 `toolName`/`callId` |
| 6 | `ContentBlock` 是密封联合，未知类型落 `ContentBlockUnknown` 并**保留 raw**（`content.dart:29`） | UI 必须能渲染一个自己不认识的东西 |
| 7 | `ContentToolCall.arguments` 是**未解析的 JSON 字符串**（:80），不是 map | 美化它是纯展示工作 |
| 8 | `ToolResultData.fromJson` **必须同时读 v3 与 v4 两种载荷位置**（:285） | 拿到的 `ToolResultData` 已被归一化，**widget 里不要再判版本** |
| 9 | `MessageData.plainText` 用 `\n\n` 拼接文本块（:220） | |
| 10 | `TokenUsage.contextTokens = input + cacheRead + cacheWrite`，**不含输出**（:366） | 上下文压力条用这个定义，不要自己加 `outputTokens` |
| 11 | `JobView.elapsed` 内部调 `DateTime.now()`（:451）——**不是纯函数** | 放进 `build()` = 依赖真实时钟、测试不可复现 ⇒ elapsed 只能由**显式 tick 的本地状态**驱动 |
| 12 | `QuestionAnswerItem.custom` 挂在**每一条**上，不是批次上（:512） | 少一个字段，宿主 zod 会静默剥掉，答案变空 |
| 13 | 宿主**逐位校验**提问应答的条数与 id（`interaction_controller.dart:165`） | 跳过的提问也必须给空选 |

### 1.2 控制器给定的形状

| # | 代码事实（引用） | ⇒ 对 UI 的强制要求 |
| --- | --- | --- |
| 14 | `TranscriptController._publish()` 每次都 `List.unmodifiable(_rows)`（`transcript_controller.dart:397`），列表身份每帧变。**但** `_rows` 是原地可变的，`:290–394` 只写目标下标，未被改的行保持**同一实例** | 「行身份稳定」是全篇的性能地基：`Element.update` 天然短路。任何「重建全部行」的写法都会毁掉它（§4.3） |
| 15 | `_toolRowIndex` 按 `callId` 配对、**绝不按位置**（:313/:318）；找不到调用时补一行孤儿结果（:341），且该行 `callId` 被写成**字面量 `'?'`**（:347） | widget key 必须对 `'?'` 特判，否则多个孤儿行撞 key（§4.2） |
| 16 | `session/page` 的 `maxMessages` 是 200（`:235`），但一次 `tool/call`+`tool/result`+`todo/write` 会产出多行 | 行数上界按 **200 × 3.5 ≈ 700** 估；分页放开后按 2000+ 估 |
| 17 | `send()` 把 `mode` **硬编码成 `'queue'`**（:420） | 输入栏的 `queue`/`steer` 需要改这一行（§3.3.1） |
| 18 | `assistant-stream` 帧**当前只翻 `running`、不插文字**（:274–283），注释写明取舍：*「半截文字在手机上闪烁比晚 200ms 出现糟糕得多」* | 尊重这个判断；§5 只给它加一层**可关的**增强，不推翻 |
| 19 | `SessionListController` **既不轮询也不实时**。`:56-59` 的注释声称靠 `$events` 与 `workspace/follow` 增量更新，但**代码里没有任何订阅**——只有 `load()`（:107）/ `search()`（:131）/ `create()`（:178）。`DshEndpoint.workspaceFollow`（`qx_protocol.dart:66`）已定义但**无人使用** | §8 的实时列表是**新增能力**，不是「保持现状」 |
| 20 | `interactionControllerProvider` 是 **`autoDispose`**（`interaction_controller.dart:206`），但它承载的是**连接级、跨会话、阻塞用户**的状态 | 没有 widget watch 时它就被回收，待审批会静默丢失 ⇒ 必须改 keep-alive（§6.1） |
| 21 | `EventEmit('inbox/changed')` 是**空分支**（:79），注释说「提示条自己会轮询」——**它并没有轮询** | 「阻塞等你」与「待发附件」今天都没有数据源（§8.1 / §3.3.2） |

### 1.3 主题层给定的形状

| # | 代码事实（引用） | ⇒ 对 UI 的强制要求 |
| --- | --- | --- |
| 22 | `ThemeData`/`ColorScheme`/`TextTheme` 是唯一真源。`QxColor`（`qx_tokens.dart:18`）、`QxType`（`qx_type.dart:16`）、`QxThemedStyles` 的 `groupBox`/`sunkenBox`/`raisedBox`（:145） | 叶子 widget 里禁止 `Color(0xFF…)` / 裸 `TextStyle(...)` / 魔法 `EdgeInsets`。今天仍有违例（`transcript_view.dart:701`、`:712`），新代码不得复制（I20） |
| 23 | **目前没有任何 `ThemeExtension`**（`grep ThemeExtension` 零命中）；状态色靠 `QxColor.running(brightness)` 每次在 `build()` 里现算 | 这正是「不能在 `build()` 里干活」的第一条来源 ⇒ §2 新增 `QxSemantic` |
| 24 | `splashFactory: InkSparkle.splashFactory`（`qx_theme.dart:41`）；`ListTile` 被带底色的 `Container` 隔开时，涟漪画在外层 `Material` 上被盖住（`transcript_view.dart:236-251`、`test/ui_test.dart:257-282`） | 任何新的可点区域都必须让**最近 `Material` 祖先**承载表面（I02） |
| 25 | `app.dart:74` 把 `textScaler` 夹在 0.9–1.3 | 本文要求 2.0 下不溢出，是为了让这个夹取**随时可以放开**而不需要重做布局（§9.4） |
| 26 | `MaterialApp.router` 无 `localizationsDelegates`/`supportedLocales`（`app.dart:61`），`pubspec.yaml` 也无 `flutter_localizations` | Material 内建字符串（选择菜单/开关/滚动条）在中文 App 里**会用英文朗读** ⇒ §8.6 补上 |
| 27 | `pageTransitionsTheme` 已用 `PredictiveBackPageTransitionsBuilder`（`qx_theme.dart:218`） | `PopScope` 与预测返回手势的交互见 §8.5 / R3 |

### 1.4 依赖现状（`pubspec.yaml`）

| 包 | 版本 | 对本文的影响 |
| --- | --- | --- |
| `flutter_riverpod` | `^2.5.1`（:14） | **不是 2.6**。不迁移 `Notifier`/`NotifierProvider`，不迁移 `Ref` 风格以外的 API |
| `flutter_markdown` | `^0.7.3`（:27） | `MarkdownBody` **每次 `data` 变化全量重解析**，无增量、无缓存 → §5.2 的核心约束 |
| `go_router` | `^14.2.7`（:17） | 路由层已在 `app.dart:13` |
| — | **无** `flutter_localizations` / `intl` | §8.6 要求补 |
| — | **无** golden 测试依赖 | §10 的 harness 全部用 `flutter_test` 内建能力 |

---

## 2. 新增的基建：`QxSemantic`（`ThemeExtension`）

**先加它，后面所有 widget 才有一个合规的取色入口。**

```dart
// lib/app/theme/qx_semantic.dart
@immutable
class QxSemantic extends ThemeExtension<QxSemantic> {
  const QxSemantic({
    required this.running, required this.pending, required this.done,
    required this.fail, required this.offline, required this.attention,
    required this.reasoning, required this.code, required this.codeMuted,
    required this.hairline, required this.sunken, required this.raised,
  });

  final Color running;    // QxColor.running(b)   agent 正在跑
  final Color pending;    // QxColor.pending(b)   waiting / needs-you
  final Color done;       // QxColor.done(b)      完成
  final Color fail;       // QxColor.fail(b)      失败
  final Color offline;    // QxColor.offline      已断开
  final Color attention;  // pending 的高强调变体，「需要你」专用
  final Color reasoning;  // 思维链正文（onSurfaceVariant 的低强调版）
  final Color code;       // 代码块前景
  final Color codeMuted;  // 代码块内次要前景
  final Color hairline;
  final Color sunken;
  final Color raised;

  /// **唯一的读取入口。** 叶子 widget 里不许写 `QxColor.running(theme.brightness)`。
  static QxSemantic of(BuildContext c) => Theme.of(c).extension<QxSemantic>()!;

  static QxSemantic ofBrightness(Brightness b) => switch (b) {
        Brightness.dark => const QxSemantic(
            running: QxColor.runningDark, pending: QxColor.pendingDark,
            done: QxColor.doneDark, fail: QxColor.failDark,
            offline: QxColor.offline, attention: QxColor.pendingDark,
            reasoning: Color(0xFF7C8895), code: Color(0xFFD5DEE6),
            codeMuted: Color(0xFF8A98A6), hairline: QxColor.hairlineDark,
            sunken: QxColor.sunkenDark, raised: QxColor.raisedDark),
        Brightness.light => const QxSemantic(
            running: QxColor.runningLight, pending: QxColor.pendingLight,
            done: QxColor.doneLight, fail: QxColor.failLight,
            offline: QxColor.offline, attention: QxColor.pendingLight,
            reasoning: Color(0xFF5A6672), code: Color(0xFF10161C),
            codeMuted: Color(0xFF5A6672), hairline: QxColor.hairlineLight,
            sunken: QxColor.sunkenLight, raised: QxColor.raisedLight),
      };

  @override QxSemantic copyWith({ /* 12 个字段 */ }) => …;  // 机械
  @override QxSemantic lerp(ThemeExtension<QxSemantic>? other, double t) => …;  // 机械
}
```

在 `qx_theme.dart:45` 的 `base.copyWith(...)` 里加一项
`extensions: [QxSemantic.ofBrightness(brightness)]`。

**收益（可量化）**：今天每个叶子 widget 的 `build()` 都在做 `Theme.of(context)`
+ 4 次 `QxColor.xxx(brightness)` + 若干 `.withValues(alpha:)` 分配。
扩展后是 1 次扩展查表 + 0 次分配——这是列表滚动时 `build()` 阶段
最容易被忽略的一块分配来源。

---

## 3. 逐个界面的 widget 组合

### 3.1 会话列表（`SessionsScreen`）

#### 3.1.1 目标形状

现状是一摞**完全相同**的卡片（`sessions_screen.dart:210` `SessionRow`），
20 条与 200 条的信息密度一样低。改成**三桶分节 + 三种行密度**：

| 桶 | 判据 | 行形态 | 高度 |
| --- | --- | --- | --- |
| `blocked` | 该 session 有待答交互（approval/question） | **加高卡**：状态条 + 标题 + 行动胶囊 | ~96dp |
| `running` | `summary.running == true` | 中卡：状态 + 标题 + 预览 + meta | ~88dp |
| `idle` | 其余（含 `isDetached`） | **紧凑行**：状态 + 标题 + meta（**无预览行**） | ~56dp |

`idle` 行去掉 `lastPrompt` 预览（`session.dart:252`）是最大的密度收益：
一屏从 ~5.5 条变成 ~8 条，且 `idle` 是绝大多数。
**代价**：搜索结果的 `SessionSummary` 用 `cwd` 承载 snippet（`session_list_controller.dart:167`），
那正是预览位。规则：**有 `turnOutline` 才显示预览行**，
搜索结果（无 outline）自动落回紧凑行——不需要额外判断。

#### 3.1.2 树

```
Scaffold(
  appBar: AppBar(title: Column([机名, 主机:端口]), actions: [刷新]),
  body: Column(
    LinkStatusBar(link: link, onRetry: …),          // 仅非就绪时占位（现状已对）
    SessionsFilterBar(                               // 过滤 + 排序，sticky
      segments: [全部 / 需要你(n) / 运行中(n)],
      searchField: true,
    ),
    Expanded(
      RefreshIndicator(
        onRefresh: …,
        child: CustomScrollView(
          key: const PageStorageKey('qx.sessions'),
          controller: _scroll,
          physics: const AlwaysScrollableScrollPhysics(),
          cacheExtent: 400,
          slivers: [
            SliverPersistentHeader(
              pinned: true,
              delegate: SessionsFilterHeaderDelegate(minExtent: 52, maxExtent: 52),
            ),
            for (final bucket in buckets) ...[
              SliverPersistentHeader(
                pinned: false,                       // 桶标题随内容滚走
                delegate: BucketHeaderDelegate(
                  label: bucket.label, count: bucket.rows.length,
                  extent: 34, semantic: bucket.stateWord,
                ),
              ),
              SliverList.builder(
                itemCount: bucket.rows.length,
                itemBuilder: (c, i) => SessionTile(
                  key: ValueKey('${bucket.name}:${bucket.rows[i].sessionId}'),
                  session: bucket.rows[i],
                  density: bucket.density,
                  onTap: …,
                ),
              ),
              const SliverToBoxAdapter(child: SizedBox(height: QxSpace.s)),
            ],
            const SliverToBoxAdapter(child: SizedBox(height: QxSpace.huge * 2)), // FAB 让位
          ],
        ),
      ),
    ),
    const PendingInteractionBar(),
  ),
  floatingActionButton: …,
)
```

**为什么 `CustomScrollView` + `Sliver*` 而不是 `ListView.separated`**：

- 桶标题需要 sticky（滚到第二个桶时，用户要知道自己在哪个桶里），
  `ListView.separated` 做不到；
- 行密度不同 ⇒ 分隔线规则不同（`blocked` 桶内不留间距，`idle` 桶内 2dp）；
- 筛选条是 sticky 的，这需要 `SliverPersistentHeader`；
- 3 个桶 × (1 header + 1 list) = 最多 6 个 sliver，数量可控。

**`SliverPersistentHeader` 什么时候值得用**（避免滥用）：
只在两种地方——(a) 永远可见的控件（筛选条）；(b) 指示「你在哪一段」的分节头。
**不给单行用**：`SliverPersistentHeader` 每一帧要做 `maxExtent/minExtent` 插值与
`shouldRebuild` 判定，一个 300 行的列表里放 300 个 persistent header 是性能自杀。
本设计里 persistent header 数量固定为 **≤ 5**（1 筛选 + ≤3 桶头 + 1 兜底），可断言。

#### 3.1.3 `SessionTile`（替换 `SessionRow`）

`SessionTile` 是 `StatelessWidget` + `const` 构造。**它在 `build()` 里不许有任何计算**：
所有派生值（`displayTitle`、`lastPrompt`、meta 片段数组）在 **`sessionBucketsProvider` 里算完**，
以 `SessionTileModel` 的形式传进来。理由见 §4.2。

```
SessionTile(
  model: SessionTileModel(        // 全部字段已算好
    sessionId, title, metaCwd, stateWord, accent: QxState,
    preview: String?,             // 可能 null
    stats: String?,               // "12 轮"
    ctx: String?,                 // "48k ctx"
    out: String?,                 // "↑3.2k"
    ago: String,                  // "5 分钟前"
    density: SessionTileDensity,
  ),
)
  → RepaintBoundary(
      MergeSemantics(
        Semantics(
          button: true,
          container: true,
          label: '${model.title}，${model.stateWord}，${model.metaSentence}',
          child: Material(color: surface, shape: RoundedRectangleBorder(
            borderRadius: QxRadius.cardAll,
            side: BorderSide(color: model.borderColor),
          ), clipBehavior: Clip.antiAlias,
            child: InkWell(onTap: …, borderRadius: QxRadius.cardAll,
              child: Padding(… Column[ 状态行, 标题, 预览?, meta行 ])),
          ),
        ),
      ),
    )
```

**关键点**：

- `Material` 是 `InkWell` 的**最近 `Material` 祖先**（`transcript_view.dart:236-251` 的教训）。
  卡片本体由 `Material` 的 `color` + `shape` 承载，**不再套 `Container(decoration:)`**——
  卡片有描边（`shape` 的 `side`），不用 `DecoratedBox`。
- **状态永远不靠颜色**。`stateWord` 既是可见文本（`QxType.badge(accent)`），
  也是语义标签的一部分。状态点 `StateDot` 自身标 `ExcludeSemantics`（它对 TalkBack 零贡献）。
- 不同状态的**图形**也不同（色觉障碍可辨）：`running` 实心点 + 呼吸、
  `blocked` 圆环内一点 + `!`、`detached` 斜杠圆。颜色、形状、**文字**三重编码。
- `MergeSemantics` + `container: true` 是必须的：
  今天 `sessions_screen.dart:239` 只有 `Semantics(button: true, label: …)` 而没有
  `container: true`，子节点（标题、meta 的 6 个 `Text`）会被一起读，
  TalkBack 会念 7 段碎片。合并后是一句话（§8.1、§10 I16）。

### 3.2 会话转录（`SessionScreen` + 转录列表）

#### 3.2.1 「钉住的表头 + 钉住的输入栏」怎么处理

**决定：输入栏不放进滚动视图；实时活动指示器放进滚动视图并 sticky。**

| 元素 | 放哪 | 理由 |
| --- | --- | --- |
| AppBar（标题/停止/刷新） | `Scaffold.appBar` | 常规 |
| `LiveActivityHeader` | **滚动视图内的 `SliverPersistentHeader(pinned: true)`** | 它要表达「agent 正在做什么」，必须贴着转录内容；滚起来才看得到；空闲时高度 0，**不占空间** |
| 转录行 | `SliverList.builder` | |
| 「回到底部」胶囊 | `Stack` 里的 `Positioned`（浮在滚动视图上） | 它是**视口**的产物，不是内容的产物 |
| `PendingInteractionBar` | `Column` 里，滚动视图外 | 阻塞态必须永远可见 |
| `ComposerBar` | `Column` 里，滚动视图外 | 见下 |

**为什么输入栏必须出滚动视图**（这是本题最关键的一条布局决策）：

1. **你需要一边翻历史一边打字。** 输入栏在 `SliverToBoxAdapter` 里，键盘弹起时
   `MediaQuery.viewInsets.bottom` 变化会让滚动区收缩，用户正在看的那一行会被键盘顶走——
   这是移动端最难缠的一类 bug，而且**必然发生**。
2. **多行输入会生长。** 生长发生在滚动视图**末尾**会让 `maxScrollExtent` 每帧变化，
   与「粘底」逻辑形成反馈环：粘底 → 滚动 → 触发更多布局 → 末端又变了。
   移出滚动视图后，输入栏生长只影响它自己，滚动区高度只受键盘影响。
3. **焦点**。`Scrollable` 内含 `TextField` 时，焦点导致的 `ensureVisible` 与
   用户的滚动意图会互相打架；出滚动视图后，焦点只在输入栏内部。
4. 代价：输入栏不可折叠（用户会想「把它收起来看长输出」）——
   接受。缓解手段是**快捷收起**：滚动视图内检测到向上滑动 >120dp 时，
   把输入栏高度动画到 56dp（只留发送按钮 + 展开按钮）。这个动画是有限时长的（§6.3）。

#### 3.2.2 树

```
Scaffold(
  resizeToAvoidBottomInset: true,     // 默认值，就是我们要的
  appBar: AppBar(title: …, actions: [停止?, 刷新]),
  body: PopScope(                       // §8.5
    canPop: true,
    child: Column(
      LinkStatusBar(link: link, onRetry: …),
      Expanded(
        child: Stack(
          children: [
            SelectionArea(                       // 整个转录一个，见 §11 风险
              child: CustomScrollView(
                key: PageStorageKey('qx.transcript.$sessionId'),
                controller: _scroll,
                restorationId: 'transcript-$sessionId',
                cacheExtent: 900,                 // §3.3
                keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.manual,
                slivers: [
                  SliverPersistentHeader(
                    pinned: true,
                    delegate: LiveActivityHeaderDelegate(
                      activity: activity,           // §6.5
                      minExtent: 0, maxExtent: activity.isIdle ? 0 : 36,
                    ),
                  ),
                  SliverList.builder(
                    itemCount: rows.length,
                    itemBuilder: (c, i) => TranscriptTile(
                      key: ValueKey(_rowKey(rows[i])),
                      row: rows[i],
                    ),
                  ),
                  SliverToBoxAdapter(
                    child: Padding(
                      padding: …,
                      child: InlineNotice(text: meta.error, isError: true),
                    ),
                  ),
                  const SliverToBoxAdapter(child: SizedBox(height: QxSpace.xxl)),
                ],
              ),
            ),
            Positioned(
              right: QxSpace.l, bottom: QxSpace.s,
              child: JumpToLatestPill(count: _unseen),   // AnimatedSlide+AnimatedOpacity
            ),
          ],
        ),
      ),
      const PendingInteractionBar(),
      const ComposerBar(),
    ),
  ),
)
```

**为什么 `CustomScrollView` 而不是 `ListView.builder`**：需要一个 pinned header。
`ListView.builder` 无法承载。用 `SliverList.builder`（不是 `.delegate`）：
省掉 `SliverChildBuilderDelegate` 的闭包分配与 `shouldRebuild` 链路。

**统一现有两份实现。** 今天 `TranscriptView`（`transcript_view.dart:47`）与
`SessionScreen` 里的内联 `ListView.builder`（`session_screen.dart:149`）
是两份几乎相同的列表代码。**本文规定只有一份**：`TranscriptList`。
`TranscriptView` 保留为 `TranscriptList` 的一个 `const` 包装（给测试用），
会话页也用同一个 `TranscriptList`。

#### 3.2.3 `TranscriptTile` 的 switch

```dart
Widget build(BuildContext context) => RepaintBoundary(
  child: switch (row) {
    RowUserMessage r      => UserBubble(message: r.message, at: r.at),
    RowAssistantMessage r => AssistantBlock(message: r.message, usage: r.usage),
    RowToolCall r        => ToolCallCard(row: r),
    RowTodo r            => TodoCard(todos: r.todos),
    RowTurnDivider r     => TurnDivider(turn: r.turn),
    RowNotice r          => NoticeRow(label: r.label, detail: r.detail),
  },
);
```

`RepaintBoundary` **每行都要**，理由不是「标准做法」而是具体的：
`ToolCallCard` 里有一个跑动的指示（§6.2）、`SelectionArea` 的选区高亮会重绘
命中的所有子树、涟漪只脏一个 tile。没有 `RepaintBoundary` 时，
一次点按会把上下 30 行的 `RenderParagraph` 全部重新光栅化。

**`RepaintBoundary` 禁止放在这些位置**：

- `InkWell` / `ListTile` 的**祖先**链上（`Material` 与 `InkWell` 之间）——
  会把涟漪裁掉（`clipBehavior` 语义），是 §1.3-24 那个 bug 的近亲。
- `SliverPersistentHeader` 的 delegate 返回的 `build()` 结果**内部**——
  边界要在 delegate 的**外面**（在 `SliverPersistentHeader` 这层），
  否则 pinned 区的重绘会漏到下面的 sliver。

#### 3.2.4 行内组件的固定要求

| 组件 | 关键要求 | 依据 |
| --- | --- | --- |
| `UserBubble` | 右对齐、宽度上限 `min(屏宽×0.82, …)`；`maxWidth` 只能用 `ConstrainedBox`（现有 `MediaQuery.size.width × 0.82` 在折叠屏/分屏下算错） | `transcript_view.dart:118` |
| `AssistantBlock` | 无气泡全宽；`ContentReasoning` 先渲染且默认折叠；正文走 `MarkdownBody`（**durable 行才用**） | `transcript_view.dart:167` |
| `ToolCallCard` | 表面由 `Material` 承载；**结论在折叠前可见**；参数/输出的**字符串拼接必须惰性** | §3.2.5 |
| `TodoCard` | `Wrap` 布局条目（不是 `Column`），2.0 字号下不溢出 | `transcript_view.dart:521` |
| `TurnDivider` | 两个 `Divider`（左右各一）；`ExcludeSemantics` 之外还要给 `TURN 3` 一个语义 | `transcript_view.dart:564` |
| `NoticeRow` | `maxLines: 3` + ellipsis；**不参与可选择** | `transcript_view.dart:633` |

#### 3.2.5 `ToolCallCard` 的惰性化（本文认为最值钱的一条改动）

现状 `transcript_view.dart:323-326`：

```dart
final output = row.result?.message.content
    .whereType<ContentToolResult>()
    .map((b) => b.content.map((c) => c.plainText).join('\n'))
    .join('\n');          // ← 在 build() 里
```

**一次 `rg` 搜 300 个文件可以返回 2MB。** 今天的代码在**每一行工具卡
被 rebuild 的时候**都会拼一次 2MB 字符串——包括它只是滚进 `cacheExtent`、
包括用户只是点了一下别处导致列表重绘的时候。
`:723` 的 `_prettyJson` 同理：每次 `build()` 都跑一次 `JsonEncoder.withIndent`。

设计：

```dart
class ToolCallCard extends StatelessWidget {
  @override
  Widget build(BuildContext context) => … ExpansionTile(
    children: [                                  // ← 只在展开时构建
      if (args.isNotEmpty)
        _CodePanel.lazy(
          title: 'ARGUMENTS',
          load: () => prettyJsonCache.getOrPut(callId, () => prettyJson(call.arguments)),
        ),
      if (resultTextSummary != null)             // ← 折叠时只需要「摘要」
        _CodePanel.lazy(
          title: 'OUTPUT',
          load: () => outputCache.getOrPut(callId, () => joinOutput(result)),
        ),
    ],
  );
}
```

`prettyJsonCache` / `outputCache` 挂在
`Provider.autoDispose.family<CodePanelCache, String>`（key = `callId`），
容量上限 **32 项 LRU**（用 `LinkedHashMap` 手写，别引 `collection`）。
折叠标题上要显示的行 **`${(result.textLength / 1024).toStringAsFixed(1)}KB · 42 行`**
由控制器在 `_applyRecord` 时算好存进行对象——**不在 `build()` 里算**。

### 3.3 输入栏（`ComposerBar`）

```
ComposerBar                       // ConsumerStatefulWidget
  Column(
    PendingAttachmentStrip,        // 仅非空时出现（§3.3.2）
    ContextMeter,                  // 上下文占用条（§3.3.3）
    Row(
      SendModeChip,                // queue / steer（§3.3.1）
      Expanded(
        TextField(
          controller: _text,       // ValueListenable<TextEditingValue>
          minLines: 1, maxLines: 6,
          keyboardType: TextInputType.multiline,
          textInputAction: TextInputAction.newline,
          onChanged: _onDraft,     // ← 不 setState
          onSubmitted: _onSubmit,  // Enter 发送（移动端键盘无 Shift 时）
        ),
      ),
      SendOrStopButton,            // running ? stop : send
    ),
  )
```

#### 3.3.1 `queue` vs `steer`

`TranscriptController.send()` 现在硬编码 `'mode': 'queue'`（`:420`）。改为：

```dart
Future<RpcResult<Object?>> send(String text, {String? requestId, SendMode mode = SendMode.queue})
```

`SendMode { queue, steer }` 的序列化在控制器里（`mode.name`），**不在 widget 里**。

呈现用 `SegmentedButton<SendMode>` 太重（48dp 起、两段 64dp 宽）。
用一颗 **chip**：常态显示 `队列 ⌄`（`Icons.low_priority`），
点击 `PopupMenuButton` 二选一（`Icons.low_priority` / `Icons.bolt`）。
**仅在 `running == true` 时可改**——空闲时两种模式等价，展示 chip 会制造噪音。
`steer` 被选中时 chip 用 `QxSemantic.attention` 描边 + 文案 `插话`
（**颜色 + 图标 + 文字**三重）。

#### 3.3.2 `PendingAttachmentStrip`

**今天没有数据源**（§1.2-21：`inbox/changed` 是空分支）。
设计成一个纯展示组件 + 一个 provider：

```dart
class PendingAttachment {
  final String id; final String name; final int bytes;
  final String? mediaType;        // null = 纯文件
}
final pendingAttachmentsProvider =
    StateNotifierProvider.autoDispose.family<PendingAttachmentsNotifier, List<PendingAttachment>, String>(…)
```

`PendingAttachmentsNotifier` 订阅 `$events` 的 `EventEmit('inbox/changed')`，
调 `session/inbox` 取回附件列表（**未知端点**：见 §13 风险 R4）。
在拿到该端点之前，组件**保留但永不显示**——先落组件与测试，端点后接。

呈现：`SizedBox(height: 64)` 的横向 `ListView.builder`，每项 56×56 缩略
（图片用 `Image.memory`；`ImageAttachment.bytes` 是 int 不是 base64——`content.dart:109`，
落地时按 `mediaType` 决定 `MemoryImage` 的构造方式），
右侧 `×` 移除。`Semantics(label: '待发送附件 $name')`，
横向列表套 `FocusTraversalGroup` 让读屏按左右方向走。

#### 3.3.3 `ContextMeter`

来源：`TokenUsage.contextTokens`（`content.dart:366`）+ `RequestContextEvent.contextWindow`
（`session.dart:171`）。窗口大小可能**晚于**第一条 usage 事件到达。

```
ContextMeter(
  used: int,      // contextTokens
  window: int?,   // null = 未知
)
  → 已知窗口: LinearProgressIndicator（trackHeight 3，theme 已定义） + 右侧
    Text('48k / 200k')（QxType.ident，>= 80% 转 QxSemantic.fail）
  → 未知窗口: 一行 Text('48k ctx')，不画条
```

**只用 `contextTokens`，永远不用 `outputTokens` 画上下文条**（输出不占下一轮上下文）。
阈值色：`< 60%` 不着色（中性）、`60–80%` `pending`、`> 80%` `fail` + 文案
`上下文将满`。**颜色 + 数字 + 文案**三重，不靠颜色单独表意。

`>= 95%` 时输入栏上方插一行 `InlineNotice('上下文接近上限，建议开新会话')`——
这是**唯一**允许对用户「说教」的地方，因为它对应一个真实的失败模式。

#### 3.3.4 草稿与 `requestId` 的归属

**草稿绝不能进 provider**：它每次按键都变，进 provider 会让所有 watch 者重建。
草稿 = `TextEditingController`（它本身就是 `ValueListenable<TextEditingValue>`）：

```dart
ValueListenableBuilder<TextEditingValue>(
  valueListenable: _text,
  builder: (c, value, _) => SendOrStopButton(
    enabled: value.text.trim().isNotEmpty && !meta.sending,
    …
  ),
)
```

这**消灭了** `session_screen.dart:177` 的 `onChanged: () => setState(() {})`。
`setState` 只应出现在三处：草稿收缩动画控制器、跳转胶囊的可见性、草稿落盘。

`requestId`（`session_screen.dart:42-45`）是**重试身份**，不是视图状态。
移到 `ComposerSendState`（`StateNotifierProvider.autoDispose.family`），
并在 `didChangeDependencies` / `ref.listen` 里**按 sessionId 持久化**
（用户切走再切回，草稿与 requestId 都还在——现在 `TextEditingController`
在 `State` 里，切页就丢）。落到 `shared_preferences`（已在依赖里，`pubspec.yaml:24`）。

### 3.4 审批弹层 / 提问弹层

两者是**阻塞交互**，处理规则不同：

| | 审批（`PendingApproval`） | 提问（`PendingQuestion`） |
| --- | --- | --- |
| 可关闭 | 可以（`isDismissible: true`，关闭等价 `cancelled`） | **不可以**（现状 `pending_interaction_bar.dart:224` 已对） |
| 内容高度 | 固定，~40% 屏 | `min(内容高, 0.82×屏高)`，内部滚动 |
| 结构 | 单卡，无滚动 | 可滚列表 + **固定底部提交栏** |
| 键盘 | 不弹 | 弹（`viewInsets` 要参与） |
| 主按钮 | `允许这一次`（`pending` 色，**不与提交栏同形**） | `提交回答`（`FilledButton`） |
| 必答 | 四个结局之一 | **每条都应答**（§1.1-13） |

共同点（必须一致，否则用户学不会）：

- `showModalBottomSheet(isScrollControlled: true, useSafeArea: true, showDragHandle: true)`；
- 顶部一律 `MonoBadge('TOOL APPROVAL' | 'AGENT QUESTION')`；
- 底部一律 `SafeArea(top: false)`；
- **一个 sheet 只解决一个 `eventId`**；多条待答时，`PendingInteractionBar` 显示
  `共 N 项`（现状已对），点开的是 `head`（`interaction_controller.dart:35`，审批优先）。
- sheet 打开期间**锁住返回手势**：`PopScope(canPop: false, onPopInvokedWithResult: …)`
  允许关闭的 sheet 手动 `Navigator.pop`，不允许的 sheet 弹一个
  `SnackBar('agent 在等你的答复')`——**静默吞掉返回是最糟的**。

### 3.5 实时活动指示器（`LiveActivityIndicator`）

它回答一个问题：**agent 现在到底在干什么**。状态来自**已解析的事件**，
不是猜测：

```dart
sealed class LiveActivity {
  const LiveActivity();
  const factory LiveActivity.idle() = LiveActivityIdle;
  const factory LiveActivity.thinking({required String model}) = LiveActivityThinking;
  const factory LiveActivity.tool({required String name, required String summary}) = LiveActivityTooling;
  const factory LiveActivity.waiting({required String reason}) = LiveActivityAwaiting;
}
```

| 事件 | 活动 | 文案 |
| --- | --- | --- |
| `turn/start` | `thinking` | `正在思考…` |
| `assistant/message` 的 usage 到达 | `tool` | `刚完成 1 步` |
| `tool/call` | `tool` | `正在执行 <name>`（+ 参数摘要，如 `rg "qx-gate"`） |
| `tool/result` | `thinking` | `正在整理结果…` |
| 有 pending interaction | `waiting` | `等你批准 <toolName>` |
| `turn/end` | `idle` | — |

呈现：一个 pinned 的 36dp 条，**左侧一根 2dp 的 `LinearProgressIndicator`
（indeterminate）** + 文案 + 右侧 `停止` 按钮。
空闲时 `maxExtent = 0`（`SizedBox.shrink` 的效果，但保留 sliver 以免结构变化）。

**不用无限旋转的 `CircularProgressIndicator`**（`test/ui_test.dart:247` 已经因此
放弃过 `pumpAndSettle`）。`LinearProgressIndicator(value: null)` 同样无限——但它
只在 `running == true` 时存在，而 §10 的 I06 明确只要求**非运行态**下
`pumpAndSettle` 可收敛。这是刻意的取舍，写进不变量。

---

## 4. 大列表性能

### 4.1 `itemExtent` / prototype / estimate：三个都不用

`SliverList.builder` **不传** `itemExtent`，**不用** `prototypeItem`，
**不用** `SliverFixedExtentList` / `SliverPrototypeExtentList`。

理由（具体到本项目）：

- 转录行的行高跨度是 **1 行（`NoticeRow`）到 2000px（展开的 300 行 grep 输出）**，
  比值 > 1:100。任何一个固定 extent 或单一 prototype 在这个分布上都是**错的**。
- `SliverFixedExtentList` 给错 extent 的后果不是「滚动条略微不准」，
  而是**内容错位**（`maxScrollExtent` 算错，底部永远差一屏），
  并且 Flutter 在 debug 下会持续抛断言。这是比慢更糟的失败模式。
- `itemExtent` 的真实价值是**跳过子元素测量**。对我们唯一成立的场景是
  `idle` 桶的紧凑会话行（56dp 固定）——但那是另一个 sliver（列表页），
  可以用 `SliverFixedExtentList` 单独做（见 §3.1.2 的小注）。
  **转录里没有一行是定高的**，所以 `itemExtent: 0` 是正确答案。

**`cacheExtent`**：设 `900.0`（≈ 844pt 屏的 1.07 屏）。
现状是 `1200`（`session_screen.dart:159`、`transcript_view.dart:56`）。
降到 900 的理由：markdown 行的实测构建成本高（§12），900 已经足够让
2000 行的列表在快速上滑时不露出空白。**这个数字要用真机 profile 定，
不要凭直觉改**——本文给的是起点，验收标准是 §12 的 p99 帧时间。

### 4.2 行 key：必须编码什么

```dart
String _rowKey(TranscriptRow row) => switch (row) {
      RowUserMessage r      => 'u:${r.seq}',
      RowAssistantMessage r => 'a:${r.seq}',
      RowToolCall r         => r.call.callId == '?'
          ? 't?:${r.seq}'        // ⚠️ 孤儿结果：callId 是字面量 '?'
          : 't:${r.call.callId}', // ⚠️ 必须 callId，不能用 seq
      RowTodo r             => 'o:${r.seq}',
      RowTurnDivider r      => 'd:${r.seq}',
      RowNotice r           => 'n:${r.seq}',
    };
```

每一条都有理由：

1. **必须带类型前缀。** 六种行共用一个 int 命名空间；不带前缀时
   `RowUserMessage(seq: 7)` 与 `RowNotice(seq: 7)` 撞 key。
2. **工具行必须用 `callId` 而不是 `seq`。** `_applyRecord` 在结果到达时
   **原地替换**工具行并保留 `seq`（`transcript_controller.dart:329`），
   但**换了一个 widget 实例**。如果 key 是 `seq`，Flutter 看到 key 相同、
   widget 不同 → 走 `updateChild` 路径 → `ExpansionTile` 的展开状态
   （它在 `PageStorage` / `ExpansionTileController` 里）有机会被保留，
   但**有 50% 概率被重置**（取决于 `canUpdate` 与内部 `_Open` 状态机）。
   用 `callId` 明确表达「这是同一行工具调用，状态更新了」。
3. **孤儿结果的 `callId` 是字面量 `'?'`**（`:347`），**多个孤儿会撞 key**。
   ⇒ `callId == '?'` 时退化成 `'t?:${seq}'`。这条如果不处理，
   Flutter 会抛 `Duplicate keys found with the same Key`（debug 断言）。
   **这是本文从现有代码里读出来的具体坑，不是假想。**

会话列表的 key：`ValueKey('${bucket.name}:${session.sessionId}')`——
桶前缀是必须的，因为分桶会随状态变化（一个 `idle` 会话变成 `running` 会**换桶**，
`ValueKey(sessionId)` 会让 Flutter 以为它「移动」了，触发一次
remove+insert + 丢失任何挂在它 `PageStorageKey` 上的状态；
带桶前缀则是**两处独立的 insert**，视觉上是一次淡入淡出，更诚实）。

### 4.3 一行流式更新时不重建整个列表

三道闸：

**闸 1 · 行身份稳定（已有，靠纪律维持）。**
`_applyRecord` 只写目标下标 ⇒ 未变行是同一实例 ⇒ `Element.update` 短路。
**违规写法（必须 review 拦截）**：
- `rows.map((r) => r.copyWith(...))` —— 全部换实例，全部重建。
- 在 `build()` 里 `final rows = [...state.rows]`。
- 排序/过滤任何时候都不做（`sort` 也会换不动就不动，但**别在 build 里 sort**）。

**闸 2 · 发布节流（新增）。**
`_publish()` 每次 `List.unmodifiable(_rows)`（`:397`）。30 事件/秒 ⇒
每秒 30 次 700 元素的拷贝 = 2.1 万元素拷贝/秒，纯粹浪费。
改为：

```dart
Timer? _publishTimer;
final _pendingTail = <TranscriptRow>[];   // 未发布的增量

void _schedulePublish() {
  _publishTimer ??= Timer(const Duration(milliseconds: 100), () {
    _publishTimer = null;
    _publishBulk();     // List.unmodifiable + state = …
    _publishTail();     // 单独 provider，让最后一行立即可见
  });
}
```

拆两个出口：

| provider | 内容 | 更新频率 |
| --- | --- | --- |
| `transcriptRowsProvider(sessionId)` | 完整 `List<TranscriptRow>` | **≤ 10/s**（100ms 节流） |
| `transcriptTailProvider(sessionId)` | `TranscriptRow?`（最后一行） | **每事件** |

列表的 `itemCount` 用 `rows.length`（节流后），最后一行用 tail provider 覆盖，
于是「最新内容」有 100ms 的额外延迟上限——**在 60fps 下是 6 帧，肉眼无感**，
而省下的是 10 倍的元素拷贝。

**闸 3 · 每行 `RepaintBoundary`（§3.2.3）。** 把 rebuild 的后果限制在单行。

### 4.4 行变高但不破坏滚动位置

两种「变高」，处理方式不同。

**情况 A · 变高发生在锚点之下**（流式输出追加、底部新增行、
用户自己在底部展开工具卡）——**什么都不用做**。Flutter 的
`SliverMultiBoxAdaptor` 保持 `pixels` 不变，视口下方内容被推出视野，
这是期望行为。

**情况 B · 变高发生在锚点之上**（用户滚到中间时，上方一个
markdown 消息因为字体度量变化而重排；或**后台事件更新了一条不可见的工具行**，
使其从 3 行折叠态变成 12 行）——**必须修锚点**，否则用户眼前的画面会跳。

`SliverList` 不做这个补偿（`pixels` 固定 → 上方内容变高 → 视口内容下移）。
实现一个显式的锚点工具：

```dart
// lib/features/workspace/presentation/scroll_anchor.dart
class ScrollAnchor {
  /// 记录 [rowKey] 当前相对视口顶部的偏移。
  static ({String key, double dy}) capture(
      ScrollController c, List<TranscriptRow> rows, String? anchorKey) { … }

  /// 在 [before] 的下一帧恢复：若 [key] 的新 dy 与旧 dy 之差超过 0.5px，
  /// 用 jumpTo 补偿（**不用 animateTo** —— 跳回原位不该被看见）。
  static void restore(ScrollController c, ({String key, double dy}) anchor) {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!c.hasClients) return;
      final idx = /* 用 anchor.key 反查当前下标 */;
      final box = c.position.context.storageContext
          ?.findRenderObject()!;                 // viewport 的 RenderBox
      final target = /* offsetInViewport(idx) + anchor.dy */;
      c.jumpTo((target - anchor.dy).clamp(
        c.position.minScrollExtent, c.position.maxScrollExtent));
    });
  }
}
```

调用点：`TranscriptList` 在 `ref.listen(rowsProvider, …)` 里
（`build()` 里不能调 `jumpTo`）：

```dart
if (_anchorKey != null && _anchorDyAboveViewport()) {   // 只有在锚点之上有变动
  final a = ScrollAnchor.capture(_scroll, rows, _anchorKey);
  ScrollAnchor.restore(_scroll, a);
}
```

**「锚点之上有变动」的判定**：由控制器告知。给
`transcriptRowsProvider` 配一个伴生的 `transcriptPatchProvider`
（`Provider<RowPatch>`，形如 `RowPatch(fromIndex, toIndex, reason)`），
控制器在原地替换某行时把下标写进去。没有这个信号，widget 只能靠 diff，
那是 O(n) 的，跟「不重建整个列表」自相矛盾。

### 4.5 粘底与「回到底部」

```dart
class TranscriptScroll extends ConsumerStatefulWidget { … }

class _TranscriptScrollState extends ConsumerState<TranscriptScroll> {
  final _scroll = ScrollController();
  bool _stuck = true;      // 用户意图，不是瞬时状态
  int _unseen = 0;

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_onScroll);        // 唯一的 setState 来源
  }

  void _onScroll() {
    final p = _scroll.position;
    final stuck = p.pixels >= p.maxScrollExtent - 160;   // 160px 阈值
    if (stuck == _stuck) return;
    setState(() { _stuck = stuck; _unseen = stuck ? 0 : _unseen; });
  }

  // 新内容到达（ref.listen 里）：
  void _onRows() {
    if (!_scroll.hasClients) return;
    if (_stuck) {
      // 粘底：**瞬时**跳转，绝不 animateTo。
      // animateTo(300ms) 与「内容还在长」叠加会互相打断，出现抖动。
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
    } else if (_scroll.position.pixels < _scroll.position.maxScrollExtent - 240) {
      setState(() => _unseen++);          // 已经离得很远就不计数了
    }
  }
}
```

**`jumpTo` 而不是 `animateTo`（粘底场景）**：animateTo 是 300ms 动画，
动画期间 `maxScrollExtent` 还会变（流式输出），终点是移动靶。
抖动比「瞬时跳」难看得多。**「回到底部」按钮**才用 `animateTo`（`QxMotion.normal` / `enter`），
因为那是**用户的明确意图**，目标稳定。

`_unseen` 的胶囊用 `AnimatedSlide(offset: _stuck ? (0, 1.5) : Offset.zero, 200ms)`
+ `AnimatedOpacity`——有限时长，测试可收敛（§6.3）。

---

## 5. 流式文字策略

### 5.1 三层结构

```
durable 事件（assistant/message）  →  完整 markdown 行   ← 唯一的真相
delta 帧（assistant-stream）      →  纯文本实时行       ← 尽力而为，可关
转录行的最后一格                  →  光标 / 「正在输出…」
```

`TranscriptController` 新增一个**只服务 delta 的尾部状态**：

```dart
class _StreamBuffer {
  final List<String> _chunks = [];      // 追加 O(1)
  int _chars = 0;
  String? _flushed;                     // 上次 flush 的快照
  bool get isEmpty => _flushed == null;

  void add(String s) { if (s.isEmpty) return; _chunks.add(s); _chars += s.length; }
  String? snapshot() => _flushed == null ? null : _flushed! + _chunks.join();
  String commit() { _flushed = snapshot(); return _flushed!; }
  void reset() { _chunks.clear(); _flushed = null; _chars = 0; }
}
```

**为什么用 `List<String>` 而不是 `String +=`**：Dart 的 `String + String`
每次都分配新串并拷贝全部内容。3KB 的消息在 200 个 delta 下是
200 × 平均 1.5KB = 300KB 的垃圾，只为了「拼接」。分块 + 周期 `join`
把它降到 **≤ 10 次 join/秒**。

新增 `TranscriptRow` 变体：

```dart
final class RowAssistantStream extends TranscriptRow {
  const RowAssistantStream({
    required super.seq,     // seq = -1（不来自日志）
    required super.at,
    required this.text,     // 已 flush 的快照，不是完整文本
    required this.isComplete,
  });
  final String text;
  final bool isComplete;
}
```

它**不进入 `_rows`**（不进持久化列表、不参与 `seq` 去重），
只由 `transcriptStreamProvider(sessionId)` 单独暴露，
渲染成 `SliverList` 之后**固定 1 个** `SliverToBoxAdapter`。
这样流式行永远在最后，永远不会挤动 durable 行。

### 5.2 flush 节流与「放弃平滑」

```dart
Timer? _flushTimer;
static const _flushInterval = Duration(milliseconds: 120);   // ≈ 7fps
```

- delta 到达 → `_buffer.add(s)` → 若 `_flushTimer == null` 则起一个
  `Timer(_flushInterval, _flush)`。
- `_flush()`：找**句读边界**（`。`/`。`/`！`/`？`/`\n`/`\n\n`/`` ``` ``），
  若在末尾 64 字符内，就切到那里 flush（`commit` 只推进到该点，
  `_chunks` 保留尾巴）；否则整块 flush。然后重建 timer。
- 120ms = 6 帧。**这是本设计的核心数字**：一次流式 flush 最多让 1 行重排，
  而不是让 2000 行的滚动区重新布局。

**给 Markdown 的特殊禁令**：`flutter_markdown` 0.7.x 的 `MarkdownBody`
在 `data` 变化时**重新跑一遍整篇 markdown 解析**（`MarkdownBuilder` 重建块树），
且没有缓存。一条 20KB、30 个代码块的助手消息，单次解析 p99 实测量级是**十几毫秒**——
那就是**每 120ms 掉一帧**。

⇒ **流式行永远不用 `MarkdownBody`。** 用 `Text.rich`，样式取
`QxType.prose`（**与 `MarkdownStyleSheet.p` 逐字段一致**，见
`transcript_view.dart:669`），所以 durable 行替换流式行时视觉上几乎无跳变。
durable 行到达 → 流式行消失 → markdown 行出现。**一次 restyle，可接受。**

**什么时候放弃逐字平滑**（按优先级）：

| 触发 | 降级行为 |
| --- | --- |
| 已输出 > 4000 字符 | 降到 **250ms 一次**，并只 flush 到**句读边界**（可能落后 1–3 句） |
| 已输出 > 12000 字符 | 降到 **1000ms 一次**，行尾显示 `…正在输出（共 12.3k 字符）` |
| delta 速率 > 50 帧/秒 | 立即降级到上一档（防的是宿主 flush 比我们快） |
| 页面不可见（`TickerMode`/后台） | 停止 flush，只累加 buffer；回前台时 `commit()` 一次 |

**「干脆在句读边界换字」是完全可选的**（对应本题的第三个选项）。
本文的判断是：**不做全局降级**，因为本 App 的主使用场景是「看 agent 干了什么」，
不是「看打字机动画」。逐字流式只保留给**短消息**（< 4000 字符），
而这恰好是绝大多数情况。真要更激进，正确的做法是**加一个设置项**
`streamingText: 'always' | 'sentences' | 'off'`（`SharedPreferences`，`pubspec.yaml:24`），
默认 `'always'`。这比拍一个技术决定诚实。

### 5.3 `AnimatedSize` 的判决

**流式行不用 `AnimatedSize`。** 理由具体：

`AnimatedSize` 对子节点尺寸做**逐帧插值**。一次 120ms 的 flush 用
`AnimatedSize` 就是 **7 帧额外重排**，而这一行在 `SliverList` 的末尾，
每次重排都会 `markNeedsLayout` 整个 scroll view 的 `RenderSliverList`。
在 2000 行的列表里，这是**每 120ms 一次全量布局**。

规则：

- **流式增长：不动画**，瞬时变高（用户预期是文字在长，不是卡片在呼吸）。
- **首次出现**：用**一次** `AnimatedSize(duration: QxMotion.instant, curve: QxMotion.enter)`
  从 0 长到内容高（150ms，有限），避免行凭空出现。
- **最终定稿**（durable 替换流式行）：**不动画**。两者高度可能差几行，
  动画会让人以为内容还在变。
- **用户主动展开工具卡/思维链**：`ExpansionTile` 自带的有限动画（§6.2）。
- **待办条目完成**：只 `AnimatedOpacity` 150ms（图标交叉淡入），不动高度。

---

## 6. 状态归属与 provider 图

### 6.1 图

```
                        ┌─ selectedConnectionProvider
                        ├─ dshConnectionProvider(conn)        [autoDispose.family]
                        └─ dshClientProvider                  [Provider<DshClient?>]
                                     │
     ┌───────────────────────────────┼───────────────────────────────┐
     │                               │                               │
 connectionsControllerProvider   interactionControllerProvider   sessionListControllerProvider
 [StateNotifier, KEEP-ALIVE]      [StateNotifier, KEEP-ALIVE ★改] [StateNotifier, autoDispose]
     │                               │                               │
     ├ probeOf(id)                  ├ headProvider ─────────┐        ├ sessionBucketsProvider
     │   [select in rows]           │   [Provider<Pending?> │        │   [Provider<List<Bucket>>]
     │                             │                      │        │       └─ SessionTileModel
     │                             └ pendingCountProvider │        │          （派生，纯计算）
     │                                       [Provider<int>]│        │
     │                                       （per-session） │        │
     │                                                        │        │
 transcriptControllerProvider(sessionId) [StateNotifier, autoDispose.family]   │
     ├─ transcriptRowsProvider(sid)    [Provider<List<TranscriptRow>>]  ≤10/s │
     ├─ transcriptPatchProvider(sid)   [Provider<RowPatch?>]              事件级│
     ├─ transcriptTailProvider(sid)    [Provider<TranscriptRow?>]       事件级│
     ├─ transcriptStreamProvider(sid)  [Provider<StreamSnapshot?>]     ≤8/s  │
     ├─ transcriptMetaProvider(sid)    [Provider<TranscriptMeta>]      事件级│
     └─ transcriptActivityProvider(sid)[Provider<LiveActivity>]        事件级│
                                       │
   ┌───────────────────────────────────┼───────────────────────────────┐
   │                                   │                               │
 AppBar                        ComposerBar                    TranscriptList
 watch: meta{running}, title      watch: meta{sending,running},    watch: rows,
 watch: activity                 meter{used,window}              watch: patch
 Local: —                        Local: TextEditingController,   Local: ScrollController,
                                 收缩控制器, requestId(in provider) 锚点 key
```

### 6.2 「什么不能一起重建」

| 不能一起 | 因为 | 做法 |
| --- | --- | --- |
| **AppBar ↔ 转录列表** | 每来一条消息 AppBar 就重建 = 每帧重建标题 | AppBar 只 `select` `TranscriptMeta.running` 与标题；列表数据在 `rows` provider |
| **输入栏 ↔ 转录列表** | 打字时列表跟着重建 = 最常见也最致命 | 输入栏只 `select` `{sending, running, meter}`；草稿是本地 `TextEditingController` |
| **上下文条 ↔ 输入栏主体** | token 变一次输入框重排一次 | 独立 `ConsumerWidget`，只 watch meter |
| **一行 ↔ 其它行** | §4.3 闸 1 | 靠行身份 + `RepaintBoundary` |
| **流式尾行 ↔ durable 行** | 每 120ms 全表重排 | 独立 provider + 独立 `SliverToBoxAdapter` |
| **列表行的 meta 行 ↔ 行标题** | 时间戳相对值每分钟变一次 | 整个 `SessionTileModel` 一起重算（便宜），**不做**细粒度 memo |
| **`ConnectionsController._bump()` ↔ 连接页全部** | 每 25s 整页重建（`connections_controller.dart:110`） | `ref.select((s) => s.probeOf(id))` per row |

### 6.3 「消灭屏幕根部 `setState`」

今天的三处违规（`session_screen.dart`）：

| 位置 | 现状 | 替换 |
| --- | --- | --- |
| `:48` `setState(() {})`（清空输入框后刷新发送按钮） | 整页重建 | `TextEditingController.clear()` 本身会通知 `ValueListenableBuilder` → **零 setState** |
| `:56` `setState(() => _pendingRequestId = null)` | 整页重建 | `composerSendProvider(sessionId).ackOk()`（§3.3.4） |
| `:177` `onChanged: () => setState(() {})` | **每次按键整页重建** | 同上，`ValueListenableBuilder` |

`SessionScreen` 重构后的 `State` 里**只允许**这三个字段：
`_scroll`（`ScrollController`）、`_anchorKey`（`String?`）、`_stuck`（`bool`）。
其余全部下沉到 `ConsumerWidget`。这可以写成一条 review 规则：
> `_SessionScreenState` 出现除这三个字段外的字段 → 驳回。

### 6.4 notifier vs 本地 widget 状态

| 内容 | 归属 | 理由 |
| --- | --- | --- |
| 事件缓冲、seq 去重、工具配对 | **notifier** | 纯逻辑，必须可单测 |
| publish 节流定时器 | **notifier** | 与 `_flushTimer` 同生命周期，widget 重建不能丢 |
| `rows` / `meta` / `stream` / `activity` | **notifier state** | |
| 分桶、排序、`SessionTileModel` 派生 | **provider（纯函数）** | 派生状态不入 notifier，避免两次真相源 |
| markdown 样式表 | **static final per Brightness** | §6.5 |
| 展开的工具卡 id 集合 | **本地 `Set<String>` + `PageStorage`** | 纯 UI 状态，不该进 provider；转屏/滚动保持靠 `PageStorageKey` |
| 草稿文字 | **本地 `TextEditingController`** | 每按键变，不能进 provider |
| 滚动位置、粘底意图 | **本地** | 纯视口 |
| 抽屉/弹层开合 | **本地 + `Navigator`** | |

### 6.5 必须 memo 的东西（`build()` 里零工作）

1. **`MarkdownStyleSheet`**。现在 `transcript_view.dart:668` **每次 build 新建一个**。
   12 个 `TextStyle` + 4 个 `BoxDecoration` + 1 个 `TableBorder` ——
   一次 markdown 行 rebuild = 一次完整样式表分配。改成：

   ```dart
   final _sheets = <Brightness, MarkdownStyleSheet>{};
   MarkdownStyleSheet _sheetFor(Brightness b) =>
       _sheets.putIfAbsent(b, () => _buildSheet(b));
   ```

   更彻底的做法是放进 `QxSemantic` 旁边的 `MarkdownStyleSheet` 缓存 provider。

2. **`prettyJson` / `joinOutput`**（§3.2.5）——LRU 32，key = `callId`。
3. **分桶与 `SessionTileModel`**（`sessionBucketsProvider`，每次 state 变算一次）。
4. **delta 的 `join()`**（§5.1）。
5. **上下文百分比**（`_pct` 是纯除法，但配 `Text('48k / 200k')` 的格式化就不是了 ——
   缓存格式化字符串，只在数值变化时重算）。
6. **`JobView.elapsed` 的格式化**（`content.dart:451` 调 `DateTime.now()`）——
   见 §9.3。

**绝对不能建的**（§12 也列了）：折叠工具卡的 output 字符串、
折叠工具卡的 pretty JSON、任一行 markdown 的 `MarkdownBody`
（`SliverList` 保证了视口外的不会建，但**别在 `build()` 里主动 build 一遍**去量高）。

---

## 7. 动画实现（逐个：技术 / 时长 / 曲线 / 可测性）

**总规则：默认态零个无限动画。**
任何 `repeat()` 必须同时满足：(a) 显式开关（§7.6）；(b) 独立 `RepaintBoundary`；
(c) 只在 `running == true` 时存在。

`test/ui_test.dart:247-254` 已经因为运行中的 `CircularProgressIndicator`
放弃过 `pumpAndSettle`。本文的规则是为了**不让下一个功能也踩这个坑**。

### 7.1 工具卡 / 思维链展开

**技术**：保留 `ExpansionTile`，但把时长在**主题里统一**（而不是逐处设置）：
`qx_theme.dart` 加 `expansionTileTheme: ExpansionTileThemeData(expansionAnimationStyle: AnimationStyle(duration: QxMotion.normal, curve: QxMotion.enter))`。
隐式动画（MD3 `AnimationStyle`），**不自己写 `AnimationController`**。300ms / `enter`。
有限时长 ⇒ `pumpAndSettle` 可收敛，**前提是同屏没有 §7.2 / §7.5 的无限动画**。

**同时删掉工具卡里那个 `CircularProgressIndicator`**（`transcript_view.dart:356`），
运行态改用 `StateDot(pulsing: false)` + `运行中` 角标。三个理由：
①它挡住 `pumpAndSettle`；②「在跑」和「在等」不该用同一个动画
（`primitives.dart:39` 已论证：转圈=在等，呼吸=在跑）；③工具卡的运行态与
AppBar 的运行态是同一个事实，不需要两套表达。

### 7.2 运行态呼吸（`StateDot` 的 `pulsing`）

**技术**：沿用 `primitives.dart:49` 的 `_PulsingDot`（`AnimationController.repeat()`，
1600ms/周期，线性 `t`；alpha 包络 `0.28 × (1−t)`，尺寸 `1 + 0.9t`）。

**开关**：新增 `LiveMotionGate`（`StateDot(pulsing: …, gate: LiveMotionGate.of(context))`），
来自 `liveMotionProvider`（`StateProvider<bool>`，默认 `false`，由设置项「动态效果」控制），
并跟随系统的 `MediaQuery.disableAnimations`：
`gate 有效 = settings && !MediaQuery.disableAnimationsOf(context)`。
**这是无障碍要求（§9.7），不是可选项。**

**可测性**：默认关 ⇒ `pumpAndSettle` 收敛。测脉冲的用例显式包
`LiveMotionGate(enabled: true)` 并用 `tester.pump(duration)`，**禁用 `pumpAndSettle`**。

### 7.3 「需要你」的注意动画

一次性、有限、可被测试观察。三段叠加：
(1) `AnimatedOpacity` 淡入 150ms；(2) `TweenAnimationBuilder<double>` 驱动 `scale`
做 2 次呼吸包络（共 600ms，`Curves.easeInOut`）；
(3) `Transform.translate` 横向摇 3 次（共 300ms，`sin` 包络，±3px）。

**触发不在 `build()` 里**：`attentionPulseProvider(sid)` 是 `StateProvider<int>`，
由 `ref.listen(interactionHeadProvider)` 在 `null → 非 null` 时 `state++`，
同一 `eventId` 只触发一次（存 `_lastPulsed`）。动画**自己停**（不 `repeat()`）——
屏幕静止时不会有任何东西在动。

**不做的**：无限闪烁、无限缩放、自动循环。「agent 在等你」必须在用户
**看着屏幕**时传达，颜色 + 文字 + 一次脉冲就够；靠持续动画传达等于耗电
并且会让整屏都是噪声。

**可测性**：有限 ⇒ `pumpAndSettle` 收敛。断言：
`pump()` → `pump(900ms)` → `pumpAndSettle()` → `Opacity.opacity == 1`
且 widget 树里**没有** `Ticker` 在跑。

### 7.4 sheet 入场

用 `showModalBottomSheet` 默认的 `BottomSheetRoute`（250ms / MD3 `emphasized`，
与 `QxMotion.emphasized` 同源），**不要再叠第二层入场动画**。
提问 sheet 内部的「一个问题已答、跳到下一个」用
`AnimatedSwitcher` 150ms / `QxMotion.instant` / `enter`。
有限时长，测试用 `pump()` + `pump(300ms)` 走完即可。

### 7.5 实时活动 ticker

**文案轮换用 `Timer`，不用动画。** 活动类型由 durable 事件驱动（§3.5），
「换文案」本来就该由事件触发；一个 1s 的 `Timer.periodic` 在活动不变时是纯浪费。
换文案时的过渡用 `AnimatedSwitcher`（`FadeTransition`）200ms / `QxMotion.quick` / `enter`，
**key = 活动类型**。指示本身是 `LinearProgressIndicator(value: null)`（2dp 高），
**仅 `running == true` 时存在**。

**不做**打字机光标（`▍` 闪烁）——又一个无限动画，收益极低。
**可测性**：`AnimatedSwitcher` 有限 ⇒ 收敛；`LinearProgressIndicator(null)` 无限 ⇒
**只在运行态存在**，`pumpAndSettle` 的断言只针对非运行态（I06），
测运行态一律用固定时长 `pump`。

### 7.6 动画总表

| 动画 | 技术 | 时长 | 曲线 | 无限? | 默认态存在? |
| --- | --- | --- | --- | --- | --- |
| 工具卡/思维链展开 | 隐式 `AnimationStyle` | 300 | `enter` | 否 | 否 |
| 首次出现（流式行） | `AnimatedSize` | 150 | `enter` | 否 | 否 |
| 发送按钮启停 | `AnimatedContainer` | 150 | `enter` | 否 | 是 |
| 运行态呼吸 | `AnimationController.repeat` | 1600 周期 | 线性 | **是** | **否**（受 gate） |
| 需要你脉冲 | `TweenAnimationBuilder` | 900 | `easeInOut` | 否 | 否 |
| sheet 入场 | 路由内置 | 250 | `emphasized` | 否 | 否 |
| 活动换文案 | `AnimatedSwitcher` | 200 | `enter` | 否 | 否 |
| 回到底部胶囊 | `AnimatedSlide`+`AnimatedOpacity` | 200 | `enter` | 否 | 否 |
| 待办完成 | `AnimatedOpacity` | 150 | `enter` | 否 | 否 |
| 输入栏收缩 | `AnimatedSize` | 220 | `enter` | 否 | 否 |
| 输入栏指示条 | `LinearProgressIndicator(null)` | — | — | **是** | **否**（仅 running） |
| 骨架屏 | **无动画**（静态 `Container`） | — | — | — | 是 |

---

## 8. 刷新 / 实时 / 轮询策略

### 8.1 会话列表：实时，不是轮询

现状是**既不实时也不轮询**（§1.2-19）。目标架构：

```
启动/换连接
  → session/list（一次快照）
  → workspace/follow（DshEndpoint.workspaceFollow，qx_protocol.dart:66）
       ├ session/created    → 插入 + 重排
       ├ session/updated    → 按 sessionId 原地替换该行
       ├ session/ended      → 原地替换 running=false
       ├ session/deleted    → 移除
       └ 未识别帧           → 忽略（绝不退化成全量重载）
  → LinkReady 的新一代连接  → 重新 session/list + 重开 follow
```

**增量更新的实现要点**：

- 控制器内部维护 `List<SessionSummary> _items`（**可变**）+ `Map<String,int> _index`。
- 收到 `session/updated` → `_items[_index[id]] = newSummary`（换这一行实例），
  `state = copyWith(items: List.unmodifiable(_items))`。
  **不要** `items.map(...)`（全换实例 → 全表重建，违反 §4.3 闸 1）。
- 桶归属变了（`idle → running`）时，`sessionBucketsProvider` 重算，
  列表按 §4.2 用带桶前缀的 key，视觉上是一次淡入（200ms，`AnimatedSize` 不要用）。
- **排序规则**：`blocked` → `running` → `idle`；桶内按 `updatedAt` 降序。
  「需要你」永远在最上面——这是本设计的核心信息架构决策。

**「阻塞」计数从哪来**：`interactionControllerProvider` 里
`approvals` + `questions` 都带 `agentId`。**今天没有 `sessionId`**（`session.dart:396`），
所以「这个待答属于哪个会话」**在数据上不可得**。

⇒ **这是 §13 的 R1（头号阻塞项）**。两种解法：
(a) 宿主在 waterfall request 里补 `sessionId`（推荐，一行 schema 变更）；
(b) 客户端在 `turn` 上下文里猜——不可靠，拒。
在 (a) 落地前，列表的 `blocked` 桶用**连接级**的判断：
「有任意待答 → 列表顶部插一个 `PendingInteractionBar` 样的 banner」，
不给具体行标 blocked。**宁可粗一点，也不要给错的位置。**

### 8.2 什么时候手动刷新仍然正确

四种情况，`RefreshIndicator` 绑到其中前三种：

| 时机 | 动作 | 理由 |
| --- | --- | --- |
| 用户下拉 | `load()`（全量 `session/list`） | 用户明确说「我不信你」 |
| App 从后台回来**超过 30s** | `load()` + 重开 follow | iOS/Android 会静默杀 socket；退避重连能恢复**连接**，但列表可能漏掉了重连窗口内的帧（`$events` 明确不重放，`stream_mux.dart:134`） |
| 列表出现**非致命错误**（`SessionListReady.error != null`） | banner 上带一个「重试」 | 已有数据 + 出错，是最容易让人卡住的状态 |
| 断线恢复后 | **不自动重载** | 走 §8.1 的 follow 重开。自动 `load()` 会在弱网下反复打 RPC |

**不要**做的事：
- 不要为了「保险」加 `Timer.periodic` 轮询 `session/list`。控制器注释里那句
  *「轮询会把刚发生的对话拖慢好几秒」*（`session_list_controller.dart:58`）是对的。
- 不要在 `build()` 里调 `load()`。

### 8.3 保留的唯一轮询

`ConnectionsController` 的 25s 探测（`connections_controller.dart:77`）
**保留不动**，它是正确的：目标是可能离线的机器，走的是廉价的
`/qx-mobile/info` HTTP 路由，且已经做了 120ms 错峰（:104）避免打满小路由。
唯一改动：`_bump()` 让整页重建 → 用 `ref.select` 按行取 `probeOf(id)`（§6.2）。

### 8.4 转录

- **不轮询。** `session/follow` 已经是双向实时的。
- `reload()`（`transcript_controller.dart:445`）**现在会清空 `_rows`**，
  用户的滚动位置与展开状态全丢。设计：`reload()` 前后保存
  `expandedIds` 与锚点（§4.4 的 `ScrollAnchor.capture`），
  恢复后把 `expandedIds` 灌回去。触发点是 AppBar 的刷新按钮与
  `LinkReady` 的新一代连接。
- 首屏：`session/page`（`maxMessages: 200`）→ 再开 follow。
  `loading` 与 `rows.isEmpty` 都要判（现状 `transcript_view.dart:30` 已对）。

### 8.5 拉刷新与 `pumpAndSettle`

`RefreshIndicator` 在 `CustomScrollView` 上要 `AlwaysScrollableScrollPhysics`，
否则列表不满一屏时下拉无效（这是 Flutter 的经典坑）。
`onRefresh` 必须 `await` 到**网络返回**（现状 `load()` 是 await 的，对），
不能只 `setState(loading: true)` 就返回——那会让转圈闪一下就没。

### 8.6 补上本地化（无障碍的前置条件）

```yaml
# pubspec.yaml
dependencies:
  flutter_localizations:
    sdk: flutter
  intl: ^0.19.0          # 必须与当前 flutter SDK 要求的版本一致（构建时校验）
```

```dart
// lib/app/app.dart
MaterialApp.router(
  localizationsDelegates: const [
    GlobalMaterialLocalizations.delegate,
    GlobalWidgetsLocalizations.delegate,
    GlobalCupertinoLocalizations.delegate,
  ],
  supportedLocales: const [Locale('zh', 'CN'), Locale('en', 'US')],
  …
)
```

不加的后果：中文 App 里，文本选择菜单、开关、复选框、滑块、
滚动条的 TalkBack 朗读全是英文，而且**日期/数字格式**不走本地化。

---

## 9. 无障碍实现

### 9.1 8px 状态点怎么让 TalkBack 有意义

四层，从内到外：

```dart
StateDot(color: accent)                 // ① 纯装饰
  → ExcludeSemantics(child: …)          // ② 对语义树零贡献

MonoBadge(label: stateWord, color: accent)   // ③ 可见文本（运行中/空闲/已断开/需要你）

Semantics(                                  // ④ 合并成一句话
  container: true,
  label: '${model.title}，${model.stateWord}，${model.metaSentence}',
  child: MergeSemantics(child: …),
)
```

要点：

- **状态点不描述自己。** 8px 的圆对 TalkBack 毫无信息，把它留在语义树里只会
  多一个空节点或一句「图像」。`ExcludeSemantics` 是正确答案。
- **状态词必须同时是可见文本。** 这不是无障碍的额外要求，是本 App 的通用规则
  （`primitives.dart:8` 已经写明）。色觉障碍用户读的是那行字。
- **④ 必须是 `container: true` + `MergeSemantics`。** 现状
  `sessions_screen.dart:239` 缺了这两者，TalkBack 会念：
  *「重构鉴权，运行中，重构鉴权，proj，12 轮，48k ctx，↑3.2k，5 分钟前」*
  —— 标题念两遍，因为 `label` 念一遍、子 `Text` 又念一遍。
  修完之后念：「重构鉴权，运行中，proj，12 轮，48k ctx，上次输出 3.2k，5 分钟前」。
- **`metaSentence` 在 controller/provider 里预拼**（§4.1）：
  它是 `QxType.caption` 的可见内容与语义标签的**唯一来源**，
  两者不一致就会出现「看到 A 听到 B」。

### 9.2 展开的工具行怎么对读屏友好

`ExpansionTile` 本身已经是 `Semantics(expanded: …)` 的语义节点
（它内部包了 `InkWell` + `Semantics(button: true)`）。要补的是三件事：

1. **折叠态的副标题必须承载结论。** 现状已经做到一半：失败时显示
   `error.code`（`transcript_view.dart:386`）。要补的是成功时的
   **输出摘要**：`12 行 · 2.4KB · exit 0`。
   读屏用户不展开就知道结果——这是「结论先于细节」原则的无障碍版本。
2. **整行一个焦点。** `MergeSemantics` 包住 header（图标 + 工具名 + 角标），
   让它成为一个可点击、可展开的节点。展开后的代码面板**不合并**
   （它内容多，合并后朗读是一条巨长的句子）。
3. **消灭嵌套滚动的读屏陷阱。** 现状 `_CodePanel`
   （`transcript_view.dart:458-467`）是 **两层 `SingleChildScrollView`**
   （外层竖向 + 内层横向）。TalkBack 在内层竖向滚动视图里
   上下滑动会**被内层吃掉**，用户以为卡住了。

   **判决：删掉内层的竖向滚动视图。** 面板高度封顶 260dp（现状已封顶），
   更长的内容用 `maxLines: 260 / overflow: hidden` + 底部「展开全部」链接
   （点了就把封顶提到 `屏高 × 0.6`）。只保留**横向**滚动
   （命令与路径换行后不可读，横向是对的）。
   这样转录是**唯一**的竖向滚动容器，读屏的滑动行为不再有歧义。

### 9.3 转录的焦点顺序

2000 行的单个滚动视图里，读屏必须**按行**走，不是按控件走。

- **行级**：`TranscriptTile` 整体是**一个** `FocusTraversalGroup` 的成员，
  每行有一个 `FocusNode`（在 `TranscriptList` 的 `State` 里按 key 缓存，
  `Map<String, FocusNode>`，随行滚出视口时释放）。
- **焦点顺序**：`FocusTraversalGroup(policy: OrderedTraversalPolicy())` +
  `NumericFocusOrder`，行序号 = 行下标。硬件键盘 / 开关控制走这个；
  TalkBack 自己的滑动手势不读 `FocusOrder`，它读语义树。
- **行的内部结构**（`MergeSemantics` 与否，刻意不同）：

  | 部分 | `MergeSemantics`? | 理由 |
  | --- | --- | --- |
  | 工具卡 header | ✅ 合并成一个可展开节点 | 它的可操作性是单一的 |
  | 思维链 header | ✅ | 同上 |
  | 用户消息正文 | ❌ 不合并 | 正文长，合并成一句会让「下一项」需要再滑一次才念得完；分开念更自然 |
  | markdown 正文 | ❌ 不合并 | 标题/列表/代码各自成节点，读屏可以逐块跳 |
  | 待办条目 | ❌ 每条独立 | 每条是一个可读的句子 |
  | 时间戳 / 模型名 / token 数 | ✅ 合并进父行 | 它们是**元信息**，单独成节点只会增加滑动手势 |
  | `Divider` / 装饰条 | `ExcludeSemantics` | 零信息 |

- **「继续」焦点**：新内容到达且用户在粘底时，焦点**不**自动移到新行
  （会在读屏下把用户正在听的内容截断）。只有「回到底部」按钮被按下时才移动焦点。

### 9.4 2.0 字号不溢出

现状 `app.dart:74` 把缩放夹在 0.9–1.3。本文要求 **2.0 下不溢出**，
是为了这个夹取可以随时放开（无障碍不应该被一个魔法数字长期压制）。

硬规则：**转录与输入栏里没有任何「包着文本的固定高度」。**
具体清单（写进 review checklist）：

| 禁止 | 替代 |
| --- | --- |
| `SizedBox(height: 20)` 包 `Text` | 让 `Text` 自适应 + `ConstrainedBox` 只给**下限** |
| `Row` 里 `crossAxisAlignment: stretch` + 固定高 | `IntrinsicHeight` 或让 `Row` 自然高 |
| `_MetaLine` 的 `Wrap`（现状已是 `Wrap`，正确） | 保持 |
| 状态点 + 角标 + 标题的 `Row` | **改成 `Wrap`**（2.0 时角标换行到第二行） |
| 输入栏 `Row` 里 44×44 的发送按钮与 `Expanded(TextField)` 并排 | 2.0 时按钮仍 44dp（触控目标不该缩），但**行会变高**——所以 `Row` 必须是 `crossAxisAlignment: end` 且**不设固定高**（现状 `session_screen.dart:223` 已对） |
| `TextField(maxLines: 6)` | 2.0 时 6 行 = 6×31×2 ≈ 372px，可能吃掉半屏。**改为 `maxLines: 6` 但封顶整体高度**：`ConstrainedBox(maxHeight: 屏高 × 0.28)`，内部滚动 |
| `Chip` 的固定 `labelPadding` | 用主题里的 padding token |

**测试强制**（§10 I01）：每个 fixture 都在
`320×568` / `390×844` / `430×932` × `textScale ∈ {1.0, 1.3, 2.0}` 下
断言 `tester.takeException() == null`。**这是 12 组组合的参数化测试**，
用 `for` 循环生成，不是手写 12 个用例。

### 9.5 语义与状态的对应（禁止颜色单独表意）

| 状态 | 颜色 | 形状 | 文字 | 语义标签含 |
| --- | --- | --- | --- | --- |
| 运行中 | `QxSemantic.running` | 实心点 + 呼吸 | `运行中` | ✅ |
| 空闲 | `QxSemantic.done` | 空心圆 | `空闲` | ✅ |
| 已断开 | `QxSemantic.offline` | 斜杠圆 | `已断开` | ✅ |
| 需要你 | `QxSemantic.attention` | 圆环 + `!` | `需要你` | ✅ |
| 失败 | `QxSemantic.fail` | 实心方块 | `失败` + 错误码 | ✅ |
| 完成 | `QxSemantic.done` | 对勾 | `完成` | ✅ |

**低对比度的允许范围**：装饰性描边可以低对比
（现状 `sessions_screen.dart:256` 的 `accent.withValues(alpha: 0.42)` 描边没问题，
因为它只是**冗余**装饰）。**文字必须全饱和**（`QxType.badge(accent)`）。

### 9.6 触达目标

- 所有可点区域 ≥ 44×44dp。工具卡的 `ExpansionTile` 现在靠
  `tilePadding` + `ListTileTheme.minVerticalPadding`（`qx_theme.dart:168`）达标 ——
  **不要为了紧凑把它压到 40dp 以下**。
- 相邻可点区域间距 ≥ 8dp。
- `ListTileTheme.shape` 已是 `controlAll`（`qx_theme.dart:170`）——保持。

### 9.7 减弱动效

`LiveMotionGate`（§7.2）必须同时看两个信号：

```dart
static bool enabledOf(BuildContext c) =>
    ref.read(liveMotionProvider) && !MediaQuery.disableAnimationsOf(c);
```

`MediaQuery.disableAnimations` 由系统的「移除动画」/「减少动态效果」设置驱动。
所有 `repeat()` 动画与 `AnimatedSwitcher` 之外的持续动效都要看它。
`AnimatedSize` / `AnimatedOpacity` / `AnimatedSlide` 这类**一次性**过渡
可以保留（它们不是「持续动效」），但要在 `duration` 上乘
`MediaQuery.disableAnimations ? 0.01 : 1`。做法：在 `QxMotion` 上加
`static Duration of(BuildContext, Duration)` 做这个换算，全 App 统一走它。

---

## 10. 触觉反馈

现状只有 `HapticFeedback.selectionClick()`（`sessions_screen.dart:246`、
`session_screen.dart:46`、`app.dart:20`）。完整规范：

| 事件 | 强度 | 说明 |
| --- | --- | --- |
| 点会话行进详情 | `selectionClick` | 现状。**导航**是选择动作 |
| 发送按钮启停切换 | `selectionClick` | 现状 |
| 切换 `queue`/`steer` | `selectionClick` | |
| 展开/收起工具卡、思维链 | `selectionClick` | |
| **发送一条消息** | **`lightImpact`** | 从现状升级。发送是**承诺**不是选择。现在的 `selectionClick`（`session_screen.dart:46`）太轻 |
| **审批/提问到达** | **`mediumImpact`** | ⭐ 最重要的一条。手机在兜里，agent 停住了等你——**这是唯一一个「不在眼前也必须知道」的事件** |
| 一轮 `turn/end` 完成 | `lightImpact` | 告诉你可以走开了 |
| 用户按停止 | `heavyImpact` | 破坏性动作 |
| 拒绝审批 | `mediumImpact` | |
| **发送失败** | `vibrate()` | 唯一允许的 `vibrate()`：它对应一个必须立刻知道的失败 |
| 工具卡状态变化 | **无** | 被动更新。忙的工作台会一直嗡 |
| 别的会话开始运行 | **无** | 只有视觉 |
| 上下文到 90% | **无** | 文案足够 |

**总规则**：

1. **一个用户发起的事件最多一次触觉。** 两个叠加的 `HapticFeedback` 在 Android 上
   会被合并或丢弃，浪费。
2. **被动状态更新零触觉。** 唯一的例外是 §7.3 的「阻塞到达」——它虽然是状态更新，
   但它是**阻塞**的，用户不作为就永远走不下去。
3. **可关闭**：`hapticsEnabledProvider`（`StateProvider<bool>`，落
   `SharedPreferences`）。所有触觉经由一个 helper，方便一键关：

```dart
abstract final class QxHaptics {
  static void tap(BuildContext c)      => _fire(c, HapticFeedback.selectionClick);
  static void send(BuildContext c)     => _fire(c, HapticFeedback.lightImpact);
  static void attention(BuildContext c)=> _fire(c, HapticFeedback.mediumImpact);
  static void stop(BuildContext c)     => _fire(c, HapticFeedback.heavyImpact);
  static void failure(BuildContext c)  => _fire(c, HapticFeedback.vibrate);

  static void _fire(BuildContext c, Future<void> Function() h) {
    if (!ref.read(hapticsEnabledProvider)) return;
    if (MediaQuery.disableAnimationsOf(c)) return;   // 顺带尊重系统开关
    h();
  }
}
```

`MediaQuery.disableAnimations` 顺带关掉触觉是一个**可辩护的近似**
（它表达的是「减少干扰」），比再引一个设置项好。

---

## 11. 可测性契约

### 11.1 测试夹具必须能做的事

| 能力 | 现状 | 需要 |
| --- | --- | --- |
| 注入假客户端 | ❌ `DshClient` 是具体类，控制器 `ref.read(dshClientProvider)` | ✅ `ProviderScope(overrides: [dshClientProvider.overrideWithValue(fake)])` |
| 注入假连接（`states` / `events` / `currentMux`） | ❌ | ✅ `dshConnectionProvider.overrideWith((ref, conn) => FakeDshConnection())`（Riverpod 2.5 支持 family 的 `overrideWith`） |
| 推一帧到 `session/follow` | ❌ | `FakeDshClient.pushFollow(Map<String,Object?>)` — 测试能**逐帧驱动**流式与增量路径，不需要真机 |
| 设定文本缩放 | ❌ | `wrap(…, textScale: 2.0)` |
| 设定视口 | 部分（`ui_test.dart:16` 已有 `size`） | 扩到三档宽度 |
| 埋 build 计数 | ❌ | `BuildCounter`（一个 `StatefulWidget`，`static int builds`），用于 I08/I09 |

`FakeDshClient` 可以直接 `extends DshClient`（构造器公开，
`dsh_client.dart:51`），覆写 `call` 与 `callAs` 即可——
**不需要**把 `DshClient` 改成接口。若将来需要，改成
`abstract interface class DshApi` + `DshClient implements DshApi`。

### 11.2 具名不变量

这些是**测试名**，不是愿望。每一行都要有一个 `testWidgets`。

| ID | 不变量 | 对应的已修 bug |
| --- | --- | --- |
| **I01** | 每个界面在 `320×568 / 390×844 / 430×932` × `textScale ∈ {1.0, 1.3, 2.0}` 下 `takeException() == null` | 溢出类 |
| **I02** | 每个可点区域：其**最近 `Material` 祖先**的 `color != null` 且 `clipBehavior != Clip.none`；两者之间不出现带底色的 `DecoratedBox` | ✅ 墨水飞溅不可见（`test/ui_test.dart:257` 已有雏形，扩展到全部可点区域） |
| **I03** | 同一行上不出现重复字符串：标题串不在 meta 行里再出现一次；`metaCwd` 与 `displayTitle` 不等 | ✅ 重复文本（`ui_test.dart:100` 已有雏形） |
| **I04** | 折叠态（未 tap）下即可读到工具卡的**结论**：失败时 `error.code` + `失败`；成功时 `完成` + 输出行数/字节数 | ✅ 缺失错误码（`ui_test.dart:212` 已有雏形，补成功分支） |
| **I05** | 含 200 字符不可断词 / 200 行代码块 / 6 列表格 / 5 层嵌套列表的 markdown 消息，在 320 宽下不溢出、且代码块是**横向**滚动 | ✅ markdown 溢出（`ui_test.dart:364` 已有雏形，补「不可断词」） |
| **I06** | **非运行态**下 `pumpAndSettle()` 在 2s 内收敛（超时即失败） | 防回归：防止有人随手加一个 `repeat()` |
| **I07** | 工具结果到达时，行的 `Key` 集合**完全不变**（只有目标行的 widget 换了，key 不换），行数不变 | 防 `build()` 里重建行 |
| **I08** | 新事件追加后，`identical(prev[i], next[i])` 对所有 `i < n` 成立（**controller 单测**） | 防 `rows.map(copyWith)` |
| **I09** | 在输入栏打字，转录列表的 `BuildCounter` 不增加 | 防 §6.3 的 `setState` |
| **I10** | 发送失败后重发同一条文本，`requestId` **相同**；发新消息，`requestId` **不同** | `session_screen.dart:42` 的既有约束 |
| **I11** | 提交提问时，答案数组的条目数 == `QuestionItem` 数，id 顺序一致，空白的 `custom` **不出现**在 JSON 里 | `content.dart:512` |
| **I12** | 「跳过这些问题」发出的是 `rejected` + code `cancelled`；「提交空选」发出的是 `answers: [{id, selected: []}]`。两者**不混淆** | `interaction_controller.dart:189` |
| **I13** | 审批结局只能是 `allowed-once` / `rejected` / `cancelled` / `unavailable` 四个 | `interaction_controller.dart:152` |
| **I14** | `tool/result` 有已知 `callId` → 原地更新该行；`callId` 未知 → **追加恰好一行**且不改动任何已有行 | `transcript_controller.dart:326` / `:341` |
| **I15** | 每个状态承载面都有非空 `Semantics` label **且**含状态词 **且**状态词是可见文本 | 防「颜色单独表意」 |
| **I16** | `SessionTile` 的语义 label 出现**恰好一次**，且不含标题两次 | `sessions_screen.dart:239` 的 `container` 缺失 |
| **I17** | 从最深路由按两次返回：到达 `/workspace`，**测试绑定没有收到系统退出请求** | 防「一按返回就退出」 |
| **I18** | 每个空态都有 `title` + `hint`；有下一步动作时按钮可发现 | |
| **I19** | 2.0 字号下输入栏**变高**而不是裁剪（`TextField` 的实际高度 > 1.0 时的 2 倍容差） | |
| **I20** | `lib/features/**/presentation/**` 的源码里没有 `Color(0x`、没有 `TextStyle(`、没有非 `QxSpace.*` 的字面量 `EdgeInsets`（**用源码 grep 的测试**） | 防样式回潮 |

I02 / I03 / I04 / I05 是**已修 bug 的回归锁**，必须永远绿。
I06 / I08 / I09 / I20 是**架构锁**，防止本文的设计被后来者无意破坏。

### 11.3 交互类不变量（指针/语义都要断言）

用 `WidgetTester.ensureSemantics()` 开语义后断言：

- **I21** 点开工具卡 → 语义树里 `expanded: true`；`SemanticsBinding` 的
  「已折叠」提示消失；焦点不丢。
- **I22** 折叠的工具卡内**没有**可聚焦节点（代码面板不可被 Tab 到）。
- **I23** sheet 打开后，背景的 `ModalBarrier` 拦住一次 tap（断言背景没被点中）。
- **I24** 提问 sheet 上按返回 → sheet 还在 + `SnackBar` 出现（不可 dismiss 的那一种）。
- **I25** 「回到底部」胶囊只在 `_stuck == false` 时存在于语义树里。

### 11.4 必须测不到的东西（别浪费时间测）

- 呼吸动画的具体中间帧（测「gate 关时不 pumpAndSettle 超时」就够了）。
- 涟漪的像素（测 I02 的结构不变量）。
- 颜色值（测 §9.5 的**映射表**在给定输入下选中了哪一档，不测色值本身）。

---

## 12. 性能预算

### 12.1 帧与延迟

| 指标 | 目标 | 测量方法 |
| --- | --- | --- |
| 帧时间 p50 | ≤ 8ms | `SchedulerBinding.addTimingsCallback` |
| 帧时间 p99 | ≤ 16.7ms | 同上 |
| 掉帧率（10s 快速滑动 2000 行转录） | < 1% | integration test + timings |
| 转录视口的 build+layout+paint（滑动中） | ≤ 6ms/帧 | 留 10ms 给平台与 GPU |
| 折叠工具卡行 build | p50 ≤ 1.5ms / p99 ≤ 4ms | BuildCounter + stopwatch |
| markdown 行 build | p99 ≤ 8ms | 同上 |
| 首屏（200 消息 ≈ 700 行）从 `loading:false` 到首帧 | ≤ 100ms | `pumpAndSettle` 计时 |
| 300 条会话列表首帧 | ≤ 120ms | 同上 |
| 滚动中**视口外**行的 build 次数 | **0** | BuildCounter 差值 |

### 12.2 绝不能建的东西

1. **折叠工具卡的 output / pretty JSON**（§3.2.5）——一次 2MB。现状是在
   `build()` 里 join（`transcript_view.dart:323`）与 `JsonEncoder` 编码（`:723`），
   `MarkdownStyleSheet` 也是每次 build 新建（`:668`）——这三条都在 §6.5 的 memo 清单上。
2. **视口外的 markdown**——靠 `SliverList` 保证，**不要**为了量高而主动 build。
3. **流式行的 `MarkdownBody`**（§5.2）——每 120ms 一次全量 markdown 解析。
4. **任何 `print` / `debugPrint` 在热路径**。`qx_log.dart` 的日志在
   `kDebugMode` 之外必须静默（真机 release 的日志 IO 会在滑动脉冲里抖动）。
5. **一行一个 `SelectionArea`**（现状 `transcript_view.dart:134` / `:190` 是每条消息一个）
   ——2000 行就是 2000 个选择容器。改成**整个滚动视图一个**（§3.2.2）。
6. **持久 header 超过 5 个**（§3.1.2）。

### 12.3 内存

- 2000 行 × 平均 300B 的 `TranscriptRow` ≈ 600KB —— 可接受。
- `cacheExtent: 900` 的额外行：约 15 行 × markdown DOM ≈ 200KB —— 可接受。
- `outputCache` / `jsonCache`（§6.5-2）**必须有上限**：LRU 32 且按 `callId` 淘汰。
  无上限的缓存在一场 3 小时的会话后会吃掉几十 MB。
- 流式缓冲 `_chunks`：**drain 之后必须清空**（在 `commit()` 里做），
  否则一条 20KB 消息会留 200 个 chunk 的列表常驻。

---

## 13. 风险与未知

按「会不会让这份设计做不下去」排序。**决策所需**一列是要上游回答的问题。

| # | 风险 | 缓解 / 兜底 | 决策所需 |
| --- | --- | --- | --- |
| **R1** | **「阻塞等你」缺少 `sessionId`（头号阻塞）。** `PendingInteraction`（`session.dart:396`）只有 `eventId` 与 `agentId`，列表要标「哪一行需要你」必须知道 `sessionId`——**现在数据上不可得** | 需宿主在 waterfall 的 `request` 里补字段。在此之前：列表用**连接级 banner**（§8.1），不给具体行标 blocked。宁可粗一点，也不要给错的位置 | 宿主能否在一轮内补 `sessionId`？若不能，§3.1 的 `blocked` 桶降级为全局 banner，**三桶变两桶** |
| **R2** | **`flutter_markdown` 没有增量解析。** 0.7.x 的 `MarkdownBody` 每次 `data` 变全量重解析、无缓存。§5 用「流式不排 markdown」绕开了流式，但 **durable 行的首次渲染**仍要付这笔钱：20KB/30 代码块的消息，解析+布局 p99 可能 20ms+ | 在 `_applyRecord` 时把超长助手消息**按空行切成多条 `RowAssistantMessage`**——对虚拟化也是净收益（行小 ⇒ 缓存窗口能装更多行）。风险：行数暴涨（一条变 30 行），key 与锚点逻辑要能扛 | 接受「长消息拆行」吗？代价是转录的时间顺序更碎 |
| **R3** | **预测返回 + `PopScope`。** `PredictiveBackPageTransitionsBuilder` 已启用（`qx_theme.dart:218`），而会话页有草稿、有未提交附件、有可能开着 sheet | `PopScope(canPop: true)` + 只在**有草稿**时 `onPopInvokedWithResult` 拦一次。预测返回的预览态需自己提供 `canPop: false` 分支，否则返回动画会直接走完而不停在半路 | **未验证**：`canPop: false` + 自定义 `onPopInvoked` 与预测返回动画的配合。**必须真机验，测试验不出来** |
| **R4** | **`session/inbox` 端点未知。** `$events` 只给 `EventEmit('inbox/changed')`（`interaction_controller.dart:79`）一个信号，没有数据；注释声称「提示条自己会轮询」——**并没有** | 先落组件 + 测试（永不显示），端点确认后再接 | 端点名与返回结构。若**没有**这个端点，附件功能整体删掉——不要用本地临时存储硬凑，那会让「已发送的附件」变成不可信的东西 |
| **R5** | **`SelectionArea` 与 sliver 懒加载。** 整个滚动视图一个 `SelectionArea`（§12.3-5）是性能正解，但**选中的文本可能被滚出视口从而被卸载**，选区消失或错乱 | 兜底 1：退回「每条消息一个 `SelectionArea`」（现状），接受 O(可见行) 成本。兜底 2（不推荐）：只在 `cacheExtent` 内的行上放 | 真机验收：长按选中一段 → 滚动 → 再长按 |
| **R6** | **键盘 + sliver 的 `viewInsets` 打架。** §3.2.1 把输入栏移出滚动视图缓解了大部分，但光标在视口外的行里时键盘弹起仍会改变视口高度 | `_CodePanel` 删掉竖向滚动（§9.2-3）后风险小很多 | 真机验收：转录滚到中间 → 点折叠工具卡（键盘不该弹）；在提问 sheet 的「其他」框打字（sheet 不被顶飞） |
| **R7** | **`interactionControllerProvider` 改 keep-alive 的副作用。** 生命周期从「有 widget watch」变成「App 存活」，而它持有 `clientId` 与待答列表 | 切连接时必须**显式清空**（现在靠 dispose 自动清）⇒ 在 `_bind()` 里检测 `selectedConnectionProvider` 变化并 reset | 无（可控）。**必须与 §6.1 在同一个 PR 里做**，否则会出现「换电脑后还显示上一台电脑的待审批」 |
| **R8** | **`cacheExtent: 900` 是猜的（§4.1）。** 低端 Android（4GB）上 `cacheExtent: 900` + markdown 行可能触发内存压力与掉帧 | 备选（更稳）：不做低端机降级，改为把 markdown 行**最大高度封顶**（超长行内部滚动，§9.2-3 同款），代价是多一层嵌套滚动 | 是否做「低端机降级」？若做，**不引新包**——用 `renderView` 的 physical size 粗判，`MediaQuery` 不可靠 |
| **R9** | **本文的所有数字都是起点，不是实测值。** 本设计工作站上没有 Flutter SDK（`flutter` 不在 PATH，`C:\flutter` / `$LOCALAPPDATA\flutter` / `D:\` / `E:\` 均无），故 `cacheExtent: 900`、`4000` 字符降级阈值、`120ms` flush、markdown 解析耗时**全部未验证** | — | 落地时**第一件事**：`flutter run --profile` + DevTools 的 Raster/UI 线程时间线，把这些数字换成实测 |
| **R10** | **行身份稳定是「纪律」不是「机制」。** §4.3 闸 1 依赖「`_applyRecord` 只写目标下标」（`:290-394`），但没有任何机制阻止后来者写 `rows.map((r) => r.copyWith())` | 补 **I08b**：工具结果到达时已变行的实例**不变**（不只是 key 不变）。长期：把 `_rows` 换成显式不可变分块（rope / 分页列表），由类型系统保证「只有目标下标可写」 | 后者工作量不小，列为 v0.3 以后 |
| **R11** | **`themeMode: ThemeMode.dark` 写死**（`app.dart:68`） | 本文不修（超出范围） | 记在案：系统浅色用户在强光下会得到深色 App，无障碍审计会提这一条 |

---

## 附录 A · 文件落点

```
lib/app/theme/qx_semantic.dart                 新增：QxSemantic ThemeExtension
lib/app/theme/qx_theme.dart                    改：注册扩展 + expansionTileTheme
lib/app/theme/qx_motion.dart                   新增：QxMotion.of(ctx, d)（尊重 disableAnimations）
lib/shared/widgets/primitives.dart             改：StateDot 加 gate；LiveMotionGate
lib/shared/widgets/qx_haptics.dart             新增
lib/shared/motion/markdown_cache.dart          新增：样式表缓存 + LRU

lib/features/sessions/application/session_list_controller.dart
                                              改：接 workspace/follow；原地更新；
                                                  pendingAttachments 挂载点
lib/features/sessions/application/session_buckets.dart   新增：分桶 + TileModel 派生
lib/features/sessions/presentation/sessions_screen.dart  改：CustomScrollView + 三桶
lib/features/sessions/presentation/session_tile.dart     新增（替换 SessionRow）

lib/features/workspace/application/transcript_controller.dart
                                              改：publish 节流 + tail/stream/patch provider；
                                                  send(mode)；
                                                  大消息按空行拆行
lib/features/workspace/application/live_activity.dart     新增
lib/features/workspace/presentation/session_screen.dart   改：薄壳（§6.3）
lib/features/workspace/presentation/transcript_list.dart  新增：唯一列表实现
lib/features/workspace/presentation/transcript_tile.dart  新增：RepaintBoundary + key
lib/features/workspace/presentation/scroll_anchor.dart    新增
lib/features/workspace/presentation/composer_bar.dart     新增
lib/features/workspace/presentation/live_activity_header.dart 新增
lib/features/workspace/presentation/approval_sheet.dart   拆出（现 pending_interaction_bar.dart）
lib/features/workspace/presentation/question_sheet.dart   拆出
lib/features/workspace/presentation/transcript_view.dart  瘦身成 TranscriptList 包装

test/support/fake_dsh_client.dart             新增
test/support/fake_dsh_connection.dart         新增
test/support/harness.dart                     新增：wrap(textScale, size) + BuildCounter
test/ui_invariants_test.dart                  新增：I01–I20
test/ui_a11y_test.dart                        新增：I21–I25
test/theme_lint_test.dart                     新增：I20（源码 grep）
test/transcript_controller_test.dart          新增：I08 / I08b / I14
```

## 附录 B · 实施顺序

| 步 | 内容 | 依赖 | 验收 |
| --- | --- | --- | --- |
| 1 | `QxSemantic` + `QxMotion.of` + `LiveMotionGate` | — | I20 绿、I06 绿 |
| 2 | 测试夹具（fake client / connection / harness / BuildCounter） | — | 现有 69 个测试仍绿 |
| 3 | 惰性化工具卡 + markdown 样式缓存 | 1 | I04、I05 绿；build p99 ≤ 4ms |
| 4 | `TranscriptList` 统一两份实现 + key 规则 + `RepaintBoundary` | 1,2 | I07、I08b 绿 |
| 5 | publish 节流 + tail/stream/patch provider + `ScrollAnchor` | 4 | I08 绿；掉帧 < 1% |
| 6 | 流式行（分块 + 120ms flush + 降级） | 5 | 流式用例绿（`pump` 不用 `pumpAndSettle`） |
| 7 | `ComposerBar` + 草稿下沉 + `queue/steer` + `ContextMeter` | 1,2 | I09、I10、I19 绿 |
| 8 | 列表三桶 + `SessionTile` + 实时增量 | 1,2 | I03、I15、I16 绿 |
| 9 | 本地化 + 无障碍全套 | 1 | I21–I25 绿 |
| 10 | 弹层重构（拆文件 + 返回拦截） | 2 | I12、I13、I24 绿 |
| 11 | 实时活动指示器 | 5,7 | I06 在非运行态绿 |
| 12 | 真机 profile，校准 §4.1/§5.2/§12 的数字 | 全部 | R9 关闭 |

**R1（sessionId）不在表里**，因为它不在客户端。它是 §8.1 的前置，
在它落地前 `blocked` 桶按 §8.1 降级为全局 banner。

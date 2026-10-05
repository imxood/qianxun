import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../app/theme/qx_semantic.dart';
import '../../../data/models/session.dart';
import '../../workspace/application/interaction_controller.dart';
import 'session_list_controller.dart';

/// 一条会话在列表里的**呈现态**。
///
/// 状态判定与排序都收在这里，UI 层只管画。理由有两条：
///
/// 1. 05 §2 规定五态**每一个都必须有数据源**，不许凭空造。判定逻辑
///    散在 widget 里就会被「顺手加个 if」逐步污染，收在一个纯函数里
///    才可能被测试穷举。
/// 2. 04 C10 的残留风险写明：`userQuestions.active`（投影快照）与
///    `$events` waterfall（实时）**可能不一致**，必须显式处理优先级，
///    否则会出现「轨说等你、列表说不用等」的抖动。判决是**实时优先**。
class SessionPresentation {
  const SessionPresentation({
    required this.session,
    required this.state,
    required this.secondLine,
  });

  final SessionSummary session;
  final QxSessionState state;

  /// 第二行那句「agent 最近做的那件事」。查不到 → null，**不占位**。
  ///
  /// 字段名不叫 `runningTool`：05 §4.3 规定 idle 时也显示（刚收工那一轮
  /// 的回复首行），只有待答时才退化成工具名。叫 `runningTool` 会让人
  /// 以为它只在 running 下有值，从而在 idle 分支里漏掉。
  final String? secondLine;

  /// 05 §3.3：排序优先级。`等你` 永远最前，这是**强制规则**，不是可关的
  /// 排序模式。
  int get priority => qxStatePriority(state);
}

/// 判定一条会话的呈现态。
///
/// [localPending] 是本地 waterfall 里**属于该会话**的待答交互。
/// 归属靠 `agentId == sessionId`（已实测：`agentTeam.members[].id`
/// 就是 `sessionId`）。
///
/// 判定顺序是刻意的，**不能调换**：
/// 1. `已断开` 优先于一切 —— agent 根本进不去，指望它「在等你」是骗人。
/// 2. `等你` 次之 —— 本地 waterfall 优先于投影快照（实时 > 快照）。
/// 3. `运行中` —— 服务端说在跑就是在跑。
/// 4. `已完成` —— 兜底。
///
/// ⚠️ **列表里没有「失败」态，这是有意的。**
/// 05 §11 把 `failed` 列进「明确不做 · 无数据源，砍掉」。真实的数据源是
/// `turn/end` 事件的 `reason.kind`（`TurnEndEvent.isBadEnding`），而那是
/// **事件流**里的东西 —— `session/list` 的快照里根本没有轮次事件。
///
/// 曾经这里用「agent 回复的开头是不是 `error:`」来判，那是在**猜**：
/// 一次正常完成的轮次，agent 完全可以回一句
/// `Error: ECONNREFUSED 127.0.0.1:5432`（它是在告诉你数据库连不上），
/// 于是那条会话会显示成「失败」并被排到所有健康会话之上 —— 这正是
/// 05 §2 说的「凭空捏造的状态」，代价是用户的信任额度。
///
/// `failed` 态本身仍然存在，转录的工具卡在**有事件流**的地方用得起它。
/// 要让列表也显示失败，需要服务端在 `session/list` 里带一个
/// 「上一轮结果」投影；那是一次协议变更，得先探针确认。
SessionPresentation presentSession(
  SessionSummary session, {
  List<PendingInteraction> localPending = const [],
}) {
  // 只看属于这条会话的待答。agentId 就是 sessionId（已实测）。
  final mine = localPending.where((p) => p.agentId == session.sessionId);

  final state = switch (null) {
    // agent 不可用 → 无论它在报什么，都是进不去的「已断开」。
    _ when session.isDetached => QxSessionState.detached,
    // 实时优先：本地有未答的 waterfall 就是等你，哪怕投影还没同步。
    _ when mine.isNotEmpty => QxSessionState.waiting,
    // 没有本地 waterfall 时才看服务端投影。
    _ when session.awaitsUser => QxSessionState.waiting,
    _ when session.running => QxSessionState.running,
    _ => QxSessionState.done,
  };

  return SessionPresentation(
    session: session,
    state: state,
    secondLine: secondLineOf(session, mine),
  );
}

/// 第二行显示什么（05 §4.3）。
///
/// - 待答中 → 工具名（它现在正卡在这个动作上）
/// - 否则 → 最近一轮回复的**首行**
/// - 查不到 → null（**不占位**）
///
/// ⚠️ 绝不能拿用户输入的回声来填（05 §4.3）。`turnOutline.prompt` 就是
/// 用户自己说的话，它出现在列表第二行是**方向性错误**：用户已经知道自己
/// 说了什么，他不知道的是 agent 据此做了什么。
String? secondLineOf(SessionSummary session, Iterable<PendingInteraction> mine) {
  for (final p in mine) {
    if (p is PendingApproval) {
      final name = p.request.toolName;
      if (name.isNotEmpty) return name;
    }
  }
  // 05 §4.3：「刚收工 → `turnOutline.last.response` 的首行」。
  return session.lastResponseFirstLine;
}

/// 把会话排成列表顺序。
///
/// 05 §3.3 采纳为**强制规则**：`等你` 永远排在最前。同一优先级内按
/// `updatedAt` 倒序（最近动的在上），与直觉一致。
List<SessionPresentation> sortForList(List<SessionPresentation> items) {
  final out = List<SessionPresentation>.of(items);
  out.sort((a, b) {
    final byPriority = b.priority.compareTo(a.priority);
    if (byPriority != 0) return byPriority;
    return b.session.updatedAt.compareTo(a.session.updatedAt);
  });
  return out;
}

/// 当前所有会话的呈现态，按列表顺序排好。
///
/// 挂在 Riverpod 上而不是每次在 `build` 里现算：`InteractionController`
/// 是 keep-alive 的（Batch 1 修的 P0），它的每次变更都要让列表重排。
/// 做成 provider 才能让「瀑布流到达 → 对应会话立刻置顶」成为一次
/// 声明式重建，而不是靠 widget 里手动 `ref.listen`。
final sessionPresentationsProvider = Provider.autoDispose<List<SessionPresentation>>((ref) {
  final list = ref.watch(sessionListControllerProvider);
  final interactions = ref.watch(interactionControllerProvider);

  final pending = [...interactions.approvals, ...interactions.questions];

  return sortForList(
    switch (list) {
      SessionListReady(:final items) =>
        items.map((s) => presentSession(s, localPending: pending)).toList(),
      _ => const <SessionPresentation>[],
    },
  );
});

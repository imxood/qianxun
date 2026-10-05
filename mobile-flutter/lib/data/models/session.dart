import '../dsh/rpc_envelope.dart';
import 'content.dart';

/// 一条会话事件（`session/follow` 上的持久化日志条目）。
///
/// 线格式：
/// ```json
/// {"type":"assistant/message","seq":42,"time":1730000000000,"data":{…}}
/// ```
///
/// 判别字段 `type`。桌面端的 `SessionEvent` 只建模了 29 个类型，但
/// `EventClassification` 列出了约 60 个——未建模的那些会以
/// `UnknownSessionEvent` 到达。本实现**保留 raw 透传**，并对一组常见但
/// 未建模的类型（`session/title`、`command/run`、`command/done` 等）
/// 做专门解析，因为它们直接决定列表行的标题。
class SessionWireEvent {
  const SessionWireEvent({
    required this.type,
    required this.seq,
    required this.time,
    required this.data,
    this.ignorable,
  });

  final String type;
  final int seq;
  final int time;
  final Map<String, Object?> data;
  final bool? ignorable;

  factory SessionWireEvent.fromJson(Map<String, Object?> json) => SessionWireEvent(
        type: (json['type'] as String?) ?? '',
        seq: (json['seq'] as num?)?.toInt() ?? 0,
        time: (json['time'] as num?)?.toInt() ?? 0,
        data: asMap(json['data']),
        ignorable: json['ignorable'] as bool?,
      );
}

/// 会话事件的语义视图。
///
/// 保留 [wire] 是为了让 UI 在遇到尚未建模的事件类型时仍能展示点什么，
/// 而不是静默丢帧。
sealed class SessionEventView {
  const SessionEventView(this.wire);

  final SessionWireEvent wire;

  int get seq => wire.seq;
  int get time => wire.time;

  factory SessionEventView.fromWire(SessionWireEvent wire) => switch (wire.type) {
        'user/message' => UserMessageEvent(wire, MessageData.fromJson(wire.data)),
        'assistant/message' => AssistantMessageEvent(
            wire,
            AssistantMessageData.fromJson(wire.data),
          ),
        'assistant/attempt' => AssistantAttemptEvent(wire),
        'tool/call' => ToolCallEvent(wire, ToolCallData.fromJson(wire.data)),
        'tool/result' => ToolResultEvent(wire, ToolResultData.fromJson(wire.data)),
        'turn/start' => TurnStartEvent(wire, (wire.data['turn'] as num?)?.toInt() ?? 0),
        'turn/end' => TurnEndEvent(wire, wire.data),
        'step/start' => StepEvent(
            wire,
            turn: (wire.data['turn'] as num?)?.toInt() ?? 0,
            step: (wire.data['step'] as num?)?.toInt() ?? 0,
            isStart: true,
          ),
        'step/end' => StepEvent(
            wire,
            turn: (wire.data['turn'] as num?)?.toInt() ?? 0,
            step: (wire.data['step'] as num?)?.toInt() ?? 0,
            isStart: false,
          ),
        'todo/write' => TodoWriteEvent(
            wire,
            (wire.data['todos'] as List? ?? const [])
                .map((e) => TodoItem.fromJson(asMap(e)))
                .toList(growable: false),
          ),
        // 未被 SessionEvent 联合建模、但 UI 必须用到的。
        'session/title' => SessionTitleEvent(
            wire,
            (wire.data['title'] as String?) ?? '',
          ),
        'command/run' => CommandEvent(
            wire,
            label: (wire.data['label'] as String?) ?? (wire.data['command'] as String?) ?? '',
            done: false,
          ),
        'command/done' => CommandEvent(
            wire,
            label: (wire.data['label'] as String?) ?? (wire.data['command'] as String?) ?? '',
            done: true,
          ),
        'request/context' => RequestContextEvent(
            wire,
            contextWindow: (wire.data['contextWindow'] as num?)?.toInt(),
          ),
        _ => UnknownSessionEvent(wire),
      };
}

final class UserMessageEvent extends SessionEventView {
  const UserMessageEvent(super.wire, this.message);
  final MessageData message;
}

final class AssistantMessageEvent extends SessionEventView {
  const AssistantMessageEvent(super.wire, this.data);
  final AssistantMessageData data;
}

final class AssistantAttemptEvent extends SessionEventView {
  const AssistantAttemptEvent(super.wire);
}

final class ToolCallEvent extends SessionEventView {
  const ToolCallEvent(super.wire, this.data);
  final ToolCallData data;
}

final class ToolResultEvent extends SessionEventView {
  const ToolResultEvent(super.wire, this.data);
  final ToolResultData data;
}

final class TurnStartEvent extends SessionEventView {
  const TurnStartEvent(super.wire, this.turn);
  final int turn;
}

/// `turn/end`。轮次收尾，**这是「失败」态唯一真实的数据源**。
///
/// ⚠️ 形状容错：kind 既可能在 `data['reason']['kind']`，也可能在
/// `data['kind']`。原实现把整个 `data` 当成 `reason` 再读 `reason['kind']`，
/// 也就是只认 `data['kind']` —— 若实际形状是嵌套的那种，`isError` 永远
/// false，转录里的失败态从来没生效过。两种都读。
final class TurnEndEvent extends SessionEventView {
  const TurnEndEvent(super.wire, this.data);

  /// 事件的整个 `data`。
  final Map<String, Object?> data;

  /// `reason` 子对象（形状 A）；不存在时退回整个 `data`（形状 B）。
  Map<String, Object?> get reason => asMap(data['reason']).isNotEmpty
      ? asMap(data['reason'])
      : data;

  String get reasonKind => (reason['kind'] as String?) ?? 'completed';
  bool get isError => reasonKind == 'error';
  bool get isAborted => reasonKind == 'aborted';

  /// 这一轮是不是没收好（报错或被中止）。
  bool get isBadEnding => isError || isAborted;
}

final class StepEvent extends SessionEventView {
  const StepEvent(
    super.wire, {
    required this.turn,
    required this.step,
    required this.isStart,
  });

  final int turn;
  final int step;
  final bool isStart;
}

final class TodoWriteEvent extends SessionEventView {
  const TodoWriteEvent(super.wire, this.todos);
  final List<TodoItem> todos;
}

final class SessionTitleEvent extends SessionEventView {
  const SessionTitleEvent(super.wire, this.title);
  final String title;
}

final class CommandEvent extends SessionEventView {
  const CommandEvent(super.wire, {required this.label, required this.done});
  final String label;
  final bool done;
}

final class RequestContextEvent extends SessionEventView {
  const RequestContextEvent(super.wire, {required this.contextWindow});
  final int? contextWindow;
}

/// 未建模事件：保留 raw，不丢。
final class UnknownSessionEvent extends SessionEventView {
  const UnknownSessionEvent(super.wire);
}

/// 会话摘要（`session/list` 的一行）。
///
/// ⚠️ **没有 `title` 字段，也没有 `createdAt`。**
/// 标题只存在于 `projections.values["title"]`（`session/title` 事件的投影），
/// 所以列表行必须从投影里取标题，取不到再回退到 `cwd` 或 `sessionId`。
/// 这一点与直觉相反，是最容易写错的地方。
class SessionSummary {
  const SessionSummary({
    required this.sessionId,
    required this.updatedAt,
    required this.running,
    required this.blank,
    this.parentSessionId,
    this.origin,
    this.cwd,
    this.agentPreset,
    this.agentAvailable,
    this.projections = const {},
    this.userQuestions = const {},
  });

  final String sessionId;
  final int updatedAt;

  /// 附着的 agent 是否在跑。冷会话/未附着会话**恒为 false**。
  final bool running;

  /// 还没有任何轮次跑过。
  final bool blank;
  final String? parentSessionId;
  final String? origin;
  final String? cwd;
  final String? agentPreset;

  /// 这个会话当前**有没有可用的 agent**。
  ///
  /// 与 `running` 是两回事：子 agent 会话在父会话结束后会变成
  /// `agentAvailable: false`，此时点进去是进不去的。
  final bool? agentAvailable;

  /// 投影基线 `{asOfSeq, values}`。
  final Map<String, Object?> projections;

  /// `userQuestions` 投影，逐会话下发（05 §0.2 已实测：`session/list`
  /// 每项都带它）。
  ///
  /// 它是「**等你**」态的**服务端权威来源** —— 服务端直接告诉客户端哪条
  /// 会话在等人，不必客户端推断。
  ///
  /// ⚠️ 形状容错：顶层与 `projections.values` 两处都读。
  /// 05 §0.2 只记了「每项都有」，没记它挂在哪一层；两处都读的成本是
  /// 零，漏读一处的代价是整个「等你」态永不点亮（而这是本 App 的第二
  /// 签名级元素）。**探针一旦确认层级，就把 fallback 删掉。**
  final Map<String, Object?> userQuestions;

  factory SessionSummary.fromJson(Map<String, Object?> json) {
    final projBlock = asMap(json['projections']);
    final values = asMap(projBlock['values']);
    final direct = json['agentPreset'] as String?;
    final projected = values['agentPreset'] as String?;
    // 顶层优先，缺失时从投影里取（与 agentPreset 同一套路）。
    //
    // ⚠️ 判「有没有」要按**键是否存在**，不是按 map 非空。若顶层给了一个
    // 空的 `{}` 而投影里留着上一轮的快照，用 `isNotEmpty` 会去读那个
    // **过期的**投影，把已经解决掉的提问继续显示成「等你」—— 而误报
    // 「等你」会把一条早就没人管的会话顶到列表最前面。
    final uqTop = json.containsKey('userQuestions')
        ? asMap(json['userQuestions'])
        : asMap(values['userQuestions']);
    return SessionSummary(
      sessionId: (json['sessionId'] as String?) ?? '',
      updatedAt: (json['updatedAt'] as num?)?.toInt() ?? 0,
      running: json['running'] == true,
      blank: json['blank'] == true,
      parentSessionId: json['parentSessionId'] as String?,
      origin: json['origin'] as String?,
      cwd: json['cwd'] as String?,
      // 顶层优先，缺失时从投影里取。
      agentPreset: direct ?? projected,
      agentAvailable: json['agentAvailable'] as bool?,
      projections: values,
      userQuestions: uqTop,
    );
  }

  /// 投影里的 `turnOutline`：每一轮的「问了什么 / 答了什么」。
  ///
  /// 这是 DSH 给的**现成预览**，比客户端自己从转录里猜要准——它就是宿主
  /// 认定的摘要。列表行的第二行取它的 `response`（见
  /// [lastResponseFirstLine]），**不取** `prompt`。
  List<TurnOutlineItem> get turnOutline => (projections['turnOutline'] as List? ?? const [])
      .map((e) => TurnOutlineItem.fromJson(asMap(e)))
      .toList(growable: false);

  /// 最近一轮的**提问**。
  ///
  /// 05 §4.3：「第二行永远是『agent 最近做的那件事』……**永远不是用户
  /// 自己说过的话**。」所以列表**不许**用它（用 [lastResponseFirstLine]）。
  ///
  /// ⚠️ 当前 `lib/` 里没有调用方：它是对协议的忠实建模，且转录/详情页
  /// 迟早要用「这一轮问了什么」。留着，但别拿它当列表摘要。
  String? get lastPrompt {
    final t = turnOutline;
    if (t.isEmpty) return null;
    final p = t.last.prompt.trim();
    return p.isEmpty ? null : p;
  }

  /// 列表第二行：agent 最近做的那件事（05 §4.3）。
  ///
  /// - 刚收工 → `turnOutline.last.response` 的首行
  /// - 查不到 → null（**不占位**，空就空着，不要拿用户输入的回声填）
  ///
  /// 在跑时的「当前工具名 + 参数」来自 `$events` 的 waterfall，客户端侧
  /// 由调用方合并（列表控制器持有实时状态，模型本身不持有）。
  String? get lastResponseFirstLine {
    final t = turnOutline;
    if (t.isEmpty) return null;
    final r = t.last.response.trim();
    if (r.isEmpty) return null;
    return r.split('\n').first.trim();
  }

  /// 服务端说这条会话**正在等人回答**。
  ///
  /// ⚠️ 这只是**两路来源之一**。另一路是本地 `$events` waterfall
  /// （`InteractionController`），它更实时。04 C10 的残留风险写明了两路
  /// 可能不一致，判决是**实时优先**：列表控制器用本地 waterfall 覆盖它。
  bool get awaitsUser => (userQuestions['active'] as List? ?? const []).isNotEmpty;

  /// 会话是否「孤儿」：子 agent，但父 agent 已经不在。
  ///
  /// 与 `running` 是两回事——`running:false` 只表示现在没在跑，
  /// `agentAvailable:false` 表示根本进不去。
  bool get isDetached => agentAvailable == false;

  /// 标题：只能来自投影。
  String? get title {
    final t = projections['title'];
    if (t is String && t.trim().isNotEmpty) return t;
    return null;
  }

  /// 列表行展示名。
  String get displayTitle {
    final t = title;
    if (t != null) return t;
    return cwdBasename ?? '未命名会话';
  }

  /// 展示用的短 cwd 末段。`cwd` 已经被当作标题时返回 null——
  /// 否则同一条路径会在标题和副行各出现一次，纯冗余。
  String? get cwdBasename {
    final c = cwd;
    if (c == null || c.isEmpty) return null;
    return c.split(RegExp(r'[/\\]')).lastWhere((s) => s.isNotEmpty, orElse: () => c);
  }

  /// 副行该显示的 cwd。无标题时不要重复。
  String? get metaCwd => title != null ? cwdBasename : null;

  /// 统计数据投影。
  SessionStats? get stats {
    final raw = projections['sessionStats'];
    return raw == null ? null : SessionStats.fromJson(asMap(raw));
  }

  /// token 用量投影。
  TokenUsage? get tokenUsage {
    final raw = projections['tokenUsage'];
    return raw == null ? null : TokenUsage.fromJson(asMap(raw));
  }

  /// 上下文压力投影。
  int? get contextTokens {
    final raw = asMap(projections['contextPressure'])['projectedTokens'];
    return (raw as num?)?.toInt();
  }

  /// 是否子 agent 会话。
  bool get isSubagent => parentSessionId != null;
}

/// 摘要里的一轮（投影 `turnOutline`）。
class TurnOutlineItem {
  const TurnOutlineItem({
    required this.turn,
    required this.seq,
    required this.prompt,
    required this.response,
  });

  final int turn;
  final int seq;
  final String prompt;
  final String response;

  factory TurnOutlineItem.fromJson(Map<String, Object?> json) => TurnOutlineItem(
        turn: (json['turn'] as num?)?.toInt() ?? 0,
        seq: (json['seq'] as num?)?.toInt() ?? 0,
        prompt: (json['prompt'] as String?) ?? '',
        response: (json['response'] as String?) ?? '',
      );
}

/// 会话统计投影。
class SessionStats {
  const SessionStats({
    required this.turns,
    required this.steps,
    required this.llmMs,
    required this.toolMs,
    required this.ttftMs,
  });

  final int turns;
  final int steps;
  final int llmMs;
  final int toolMs;

  /// 首 token 延迟之和（跨 step 求和）。
  final int ttftMs;

  factory SessionStats.fromJson(Map<String, Object?> json) => SessionStats(
        turns: (json['turns'] as num?)?.toInt() ?? 0,
        steps: (json['steps'] as num?)?.toInt() ?? 0,
        llmMs: (json['llmMs'] as num?)?.toInt() ?? 0,
        toolMs: (json['toolMs'] as num?)?.toInt() ?? 0,
        ttftMs: (json['ttftMs'] as num?)?.toInt() ?? 0,
      );
}

/// 提问事件载荷（`user-questions/request` 的 request 槽）。
class AskUserQuestionsRequest {
  const AskUserQuestionsRequest(this.questions);

  final List<QuestionItem> questions;

  factory AskUserQuestionsRequest.fromJson(Map<String, Object?> json) => AskUserQuestionsRequest(
        (json['questions'] as List? ?? const [])
            .map((e) => QuestionItem.fromJson(asMap(e)))
            .toList(growable: false),
      );
}

/// 审批事件载荷（`approval/request` 的 request 槽）。
///
/// **没有审批 id**——身份就是 waterfall 帧的 `eventId`。
class ApprovalRequest {
  const ApprovalRequest({
    required this.toolName,
    this.callId,
    this.reason,
  });

  final String toolName;
  final String? callId;
  final String? reason;

  factory ApprovalRequest.fromJson(Map<String, Object?> json) => ApprovalRequest(
        toolName: (json['toolName'] as String?) ?? '未知工具',
        callId: json['callId'] as String?,
        reason: json['reason'] as String?,
      );
}

/// 待答 waterfall。一个事件要么要审批，要么要提问。
sealed class PendingInteraction {
  const PendingInteraction({required this.eventId, required this.agentId});

  final String eventId;
  final String agentId;
}

final class PendingApproval extends PendingInteraction {
  const PendingApproval({
    required super.eventId,
    required super.agentId,
    required this.request,
  });

  final ApprovalRequest request;
}

final class PendingQuestion extends PendingInteraction {
  const PendingQuestion({
    required super.eventId,
    required super.agentId,
    required this.request,
  });

  final AskUserQuestionsRequest request;
}

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../data/dsh/dsh_client.dart';
import '../../../data/dsh/rpc_envelope.dart';
import '../../../data/dsh/stream_mux.dart';
import '../../../data/models/content.dart';
import '../../../data/models/session.dart';
import '../../connections/application/connections_controller.dart';
import 'dart:async';

/// 待答交互的当前集合。
///
/// `$events` 上的 waterfall 是**全连接范围**的，不属于某个会话——所以
/// 这个状态挂在连接级，而不是会话级：哪怕用户正在看 A 会话，B 会话的
/// 审批请求也必须能弹出来。
class InteractionState {
  const InteractionState({
    this.approvals = const [],
    this.questions = const [],
    this.clientId,
  });

  /// 待审批（工具执行许可）。
  final List<PendingApproval> approvals;

  /// 待提问。
  final List<PendingQuestion> questions;

  /// 本代 clientId，回应 waterfall 时必须带上。
  final String? clientId;

  bool get isEmpty => approvals.isEmpty && questions.isEmpty;

  /// 最紧急的一条：审批优先于提问（审批挡住的是 agent 的动作）。
  PendingInteraction? get head {
    if (approvals.isNotEmpty) return approvals.first;
    if (questions.isNotEmpty) return questions.first;
    return null;
  }

  InteractionState copyWith({
    List<PendingApproval>? approvals,
    List<PendingQuestion>? questions,
    String? clientId,
  }) =>
      InteractionState(
        approvals: approvals ?? this.approvals,
        questions: questions ?? this.questions,
        clientId: clientId ?? this.clientId,
      );
}

/// 交互控制器。
///
/// ⚠️ **P0：`autoDispose` 在这里是 bug，不是优化。**
///
/// 待答审批是**连接级、阻塞性**的状态：agent 卡住直到人回答。而全仓只有
/// `pending_interaction_bar.dart` 一个 watcher，用户停在连接页、日志页
/// 或任何没有它的界面上时，notifier 会被回收、`_bind()` 的订阅被取消。
/// 此刻到达的审批**根本不会被看见**——而 `$events` 不重放（harness 语义：
/// 通知只在连接存活期间投递），于是 agent 永久挂着，界面上没有任何解释。
///
/// 修法：不用 `autoDispose`，由 [connectionRootProvider] 在 App 根无条件
/// `watch` 一次。待答状态的生命周期**不应该**跟着任何界面走。
class InteractionController extends StateNotifier<InteractionState> {
  InteractionController(this._ref) : super(const InteractionState()) {
    _bind();
  }

  final Ref _ref;
  StreamSubscription<Object?>? _sub;

  void _bind() {
    _sub?.cancel();
    final conn = _ref.read(selectedConnectionProvider);
    final link = conn == null ? null : _ref.read(dshConnectionProvider(conn));
    if (link == null) return;

    _sub = link.events.listen((frame) {
      if (!mounted) return;
      switch (frame) {
        case EventWaterfall w:
          _onWaterfall(w);
        case EventCancel c:
          // 别的客户端先答了，或宿主调用方取消了：**不回答**地撤掉。
          _retire(c.eventId);
        case EventEmit e:
          // 宿主通知（例如「工具需要许可」的旁路广播）。真正的请求走
          // waterfall，这里不重复处理，避免一次请求弹两个框。
          if (e.event == 'inbox/changed') {
            // 提示条自己会轮询，这里无需动作；保留分支是为了让新的
            // 通知类型有明确落点。
          }
        case EventReady():
        case EventUnknown():
          break;
      }
    });

    if (link.clientId != null) {
      state = state.copyWith(clientId: link.clientId);
    }
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  void _onWaterfall(EventWaterfall w) {
    switch (w.event) {
      case 'approval/request':
        state = state.copyWith(
          clientId: _ref.read(selectedConnectionProvider) == null ? null : _clientId,
          approvals: [
            ...state.approvals,
            PendingApproval(
              eventId: w.eventId,
              agentId: w.agentId,
              request: ApprovalRequest.fromJson(w.request),
            ),
          ],
        );
      case 'user-questions/request':
        state = state.copyWith(
          clientId: _clientId,
          questions: [
            ...state.questions,
            PendingQuestion(
              eventId: w.eventId,
              agentId: w.agentId,
              request: AskUserQuestionsRequest.fromJson(w.request),
            ),
          ],
        );
      default:
        // 未识别的 waterfall：默认**不**认领，让宿主的后续监听器接手。
        // 猜错比不猜贵。
        break;
    }
  }

  void _retire(String eventId) {
    state = state.copyWith(
      approvals: state.approvals.where((a) => a.eventId != eventId).toList(),
      questions: state.questions.where((q) => q.eventId != eventId).toList(),
    );
  }

  String? get _clientId {
    final conn = _ref.read(selectedConnectionProvider);
    return conn == null ? null : _ref.read(dshConnectionProvider(conn)).clientId;
  }

  /// 回答审批。
  ///
  /// 四个取值是宿主认识的全部结局：`allowed-once` / `rejected` /
  /// `cancelled` / `unavailable`。
  Future<void> answerApproval(String eventId, String outcome) async {
    final client = _ref.read(dshClientProvider);
    final clientId = _clientId;
    _retire(eventId);
    if (client == null || clientId == null) return;
    await client.respondWaterfall(
      clientId: clientId,
      eventId: eventId,
      outcome: DshClient.outcomeResult({'outcome': outcome}),
    );
  }

  /// 回答提问。
  ///
  /// ⚠️ 两个必须做对的细节：
  /// 1. `custom` 挂在**每一条**上，不是挂在批次上——放错位置会被宿主的
  ///    zod 解析静默剥掉，答案会变成空。
  /// 2. 宿主会**逐位**校验条数与 id，任一不符就整份拒收，工具会一直
  ///    挂着。所以每条提问都必须给出应答，哪怕用户跳过了它（空选）。
  Future<void> answerQuestion(String eventId, List<QuestionAnswerItem> answers) async {
    final client = _ref.read(dshClientProvider);
    final clientId = _clientId;
    _retire(eventId);
    if (client == null || clientId == null) return;
    await client.respondWaterfall(
      clientId: clientId,
      eventId: eventId,
      outcome: DshClient.outcomeResult({
        'answers': answers.map((a) => a.toJson()).toList(),
      }),
    );
  }

  /// 关闭提问。
  ///
  /// 必须是 `rejected` + code 恰为 `cancelled`：代理只认这一个码，其他
  /// 任何码都会变成 `bad-response` 并让等待一直挂着。
  ///
  /// 注意这与「每条都答空选」是**相反**的语义：空选是一个合法回答，
  /// 模型读成「没有偏好」；关闭才是「我不参与了」。
  Future<void> dismissQuestion(String eventId) async {
    final client = _ref.read(dshClientProvider);
    final clientId = _clientId;
    _retire(eventId);
    if (client == null || clientId == null) return;
    await client.respondWaterfall(
      clientId: clientId,
      eventId: eventId,
      outcome: DshClient.outcomeRejected(
        name: 'Error',
        message: '用户关闭了这次提问',
        code: RpcErrorCode.cancelled,
      ),
    );
  }
}

final interactionControllerProvider =
    StateNotifierProvider<InteractionController, InteractionState>((ref) {
  return InteractionController(ref);
});

/// App 根持有的连接级保活锚点。
///
/// 存在的唯一理由：在 `MaterialApp` 的 builder 里 `watch` 一次
/// [interactionControllerProvider]，让待答状态在整个 App 生命周期内
/// 保持存活，不管当前停在哪个界面。
///
/// **不要删。** 删掉它 = 把 P0 缺陷放回去。
final connectionRootProvider = Provider<void>((ref) {
  ref.watch(interactionControllerProvider);
  // 连接级控制器同样不该跟着界面走：`DshConnection` 负责重连，
  // 它的状态机必须在任何界面之外一直转。
  final conn = ref.watch(selectedConnectionProvider);
  if (conn != null) {
    ref.watch(dshConnectionProvider(conn));
  }
});

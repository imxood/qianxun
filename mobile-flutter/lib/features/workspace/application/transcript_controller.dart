import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../data/dsh/dsh_client.dart';
import '../../../data/dsh/dsh_connection.dart';
import '../../../data/dsh/qx_protocol.dart';
import '../../../data/dsh/rpc_envelope.dart';
import '../../../data/dsh/stream_mux.dart';
import '../../../data/models/content.dart';
import '../../../data/models/session.dart';
import '../../connections/application/connections_controller.dart';
import 'dart:async';

/// 转录里的一行。
///
/// 事件流被折叠成**行**——手机屏幕装不下 29 种事件的原始形态，用户要
/// 的是「谁说了什么 / agent 做了什么」，不是事件学。
sealed class TranscriptRow {
  const TranscriptRow({required this.seq, required this.at});

  final int seq;
  final DateTime at;
}

/// 用户消息。
final class RowUserMessage extends TranscriptRow {
  const RowUserMessage({
    required super.seq,
    required super.at,
    required this.message,
  });

  final MessageData message;
}

/// 助手消息（含思维链与工具调用块）。
final class RowAssistantMessage extends TranscriptRow {
  const RowAssistantMessage({
    required super.seq,
    required super.at,
    required this.message,
    this.usage,
  });

  final MessageData message;
  final TokenUsage? usage;
}

/// 工具执行。
final class RowToolCall extends TranscriptRow {
  const RowToolCall({
    required super.seq,
    required super.at,
    required this.call,
    this.result,
    this.error,
  });

  final ToolCallData call;

  /// 结果事件到达后填充。null 表示还在跑。
  final ToolResultData? result;
  final ToolErrorRef? error;

  bool get isRunning => result == null && error == null;
}

/// 待办清单快照。
final class RowTodo extends TranscriptRow {
  const RowTodo({
    required super.seq,
    required super.at,
    required this.todos,
  });

  final List<TodoItem> todos;
}

/// 回合边界（轮次分隔线）。给长会话一个可读的骨架。
final class RowTurnDivider extends TranscriptRow {
  const RowTurnDivider({required super.seq, required super.at, required this.turn});
  final int turn;
}

/// 未建模但需要露面的事件（如命令、计划模式）。
final class RowNotice extends TranscriptRow {
  const RowNotice({
    required super.seq,
    required super.at,
    required this.label,
    this.detail,
  });

  final String label;
  final String? detail;
}

/// 会话转录状态。
class TranscriptState {
  const TranscriptState({
    this.rows = const [],
    this.loading = true,
    this.error,
    this.sending = false,
    this.running = false,
  });

  final List<TranscriptRow> rows;
  final bool loading;
  final String? error;
  final bool sending;

  /// agent 是否正在跑（用于停止按钮）。
  final bool running;

  TranscriptState copyWith({
    List<TranscriptRow>? rows,
    bool? loading,
    String? error,
    bool? sending,
    bool? running,
    bool clearError = false,
  }) =>
      TranscriptState(
        rows: rows ?? this.rows,
        loading: loading ?? this.loading,
        error: clearError ? null : (error ?? this.error),
        sending: sending ?? this.sending,
        running: running ?? this.running,
      );
}

/// 会话转录控制器。
///
/// 数据流：
/// 1. `session/page` 拉一段历史（`throughSeq: -1` = 空日志，全量）。
/// 2. 打开 `session/follow` 逻辑流，接后续增量。
/// 3. 两路都折叠进同一个 [TranscriptState.rows]。
///
/// **不轮询。** 会话是双向实时的，用轮询会把刚发生的对话拖慢好几秒。
class TranscriptController extends StateNotifier<TranscriptState> {
  TranscriptController(this._ref, this.sessionId) : super(const TranscriptState()) {
    _bind();
  }

  final Ref _ref;
  final String sessionId;

  DshStream? _follow;
  StreamSubscription<Object?>? _sub;
  final List<TranscriptRow> _rows = [];
  final Map<String, int> _toolRowIndex = {};
  int _lastSeq = 0;

  void _bind() {
    _sub?.cancel();
    final conn = _ref.read(selectedConnectionProvider);
    if (conn == null) {
      state = const TranscriptState(loading: false, error: '尚未连接到电脑。');
      return;
    }
    // `watch` 而非 `read`：见 session_list_controller 里的同一条说明。
    final link = _ref.watch(dshConnectionProvider(conn));
    _sub = link.states.listen((s) {
      switch (s) {
        case LinkReady():
          unawaited(_openFollow(link));
        case LinkNeedsPairing(:final message):
          state = state.copyWith(loading: false, error: message);
        case LinkConnecting():
        case LinkReconnecting():
        case LinkIdle():
        case LinkClosed():
          break;
      }
    });
    if (link.isReady) unawaited(_openFollow(link));
  }

  @override
  void dispose() {
    _follow?.cancel();
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _openFollow(DshConnection link) async {
    await _loadHistory();
    final mux = link.currentMux;
    if (mux == null) return;
    try {
      final follow = mux.open(
        DshEndpoint.sessionFollow,
        args: {
          'address': {'kind': 'session', 'sessionId': sessionId},
          // 打开实时 delta：没有这个，助手文字要等整条消息落定才出现。
          'assistantStream': true,
        },
      );
      _follow = follow;
      unawaited(_pump(follow));
    } catch (_) {
      state = state.copyWith(error: '打开会话实时流失败。');
    }
  }

  Future<void> _pump(DshStream follow) async {
    try {
      while (true) {
        final signal = await follow.receive();
        if (signal == null) break;
        switch (signal) {
          case StreamItem(:final value):
            _onFrame(asMap(value));
          case StreamEnded():
            return;
          case StreamFailed():
            return;
        }
      }
    } catch (_) {
      // 流断了。连接管理器会重建 socket 并再开一条 follow，历史由
      // `_loadHistory` 补齐——这里静默退出即可。
    }
  }

  Future<void> _loadHistory() async {
    final client = _ref.read(dshClientProvider);
    if (client == null) return;
    state = state.copyWith(loading: true, clearError: true);

    final res = await client.call('session/page', request: {
      'address': {'kind': 'session', 'sessionId': sessionId},
      // -1 = 空日志，即从头开始。
      'throughSeq': -1,
      'maxMessages': 200,
    });

    if (!mounted) return;
    res.map(
      (value) {
        _rows.clear();
        _toolRowIndex.clear();
        _lastSeq = 0;
        final records = asMap(value)['records'] as List? ?? const [];
        for (final raw in records) {
          _applyRecord(asMap(raw));
        }
        _publish();
        state = state.copyWith(loading: false, clearError: true);
        return value;
      },
      (err) {
        state = state.copyWith(loading: false, error: err.message);
        return err;
      },
    );
  }

  /// 处理 `session/follow` 的一帧。
  ///
  /// `SessionFollowFrame` 只有三个具名 tag：`snapshot` / `assistant-stream`
  /// / 其它（就是一条 record 本身）。这样设计是为了让将来新增的 record
  /// 类**永远不会被误认成**开场帧。
  void _onFrame(Map<String, Object?> frame) {
    switch (frame['type']) {
      case 'snapshot':
        _rows.clear();
        _toolRowIndex.clear();
        for (final raw in (frame['records'] as List? ?? const [])) {
          _applyRecord(asMap(raw));
        }
        _lastSeq = (frame['cursor'] as num?)?.toInt() ?? _lastSeq;
        state = state.copyWith(loading: false, running: true);
      case 'assistant-stream':
        // 实时 delta：本 App 用 durable 事件落定后再渲染，这里只更新
        // 「正在跑」这个事实，不插入半截文字——半截文字在手机上闪烁
        // 比晚 200ms 出现糟糕得多。
        final f = asMap(frame['frame']);
        if (f['type'] == 'start') {
          state = state.copyWith(running: true);
        } else if (f['type'] == 'end') {
          state = state.copyWith(running: false);
        }
      default:
        _applyRecord(frame);
    }
    _publish();
  }

  void _applyRecord(Map<String, Object?> record) {
    final inner = asMap(record['event']);
    if (inner.isEmpty) return;
    final wire = SessionWireEvent.fromJson(inner);
    if (wire.seq <= _lastSeq && wire.seq > 0) return;
    if (wire.seq > 0) _lastSeq = wire.seq;

    final at = DateTime.fromMillisecondsSinceEpoch(wire.time);
    final view = SessionEventView.fromWire(wire);

    switch (view) {
      case UserMessageEvent v:
        _rows.add(RowUserMessage(seq: wire.seq, at: at, message: v.message));

      case AssistantMessageEvent v:
        _rows.add(RowAssistantMessage(
          seq: wire.seq,
          at: at,
          message: v.data.message,
          usage: v.data.usage,
        ));
        state = state.copyWith(running: false);

      case ToolCallEvent v:
        _toolRowIndex[v.data.callId] = _rows.length;
        _rows.add(RowToolCall(seq: wire.seq, at: at, call: v.data));

      case ToolResultEvent v:
        // ⚠️ 按 `callId` 配对，**绝不按位置**——并发工具调用下位置
        // 完全没有意义。`source.callId` 是唯一的关联键。
        final callId = v.data.message.source.callId ??
            (v.data.message.content
                .whereType<ContentToolResult>()
                .map((b) => b.toolCallId)
                .firstOrNull);
        final target = callId == null ? null : _toolRowIndex[callId];
        if (target != null && target < _rows.length) {
          final existing = _rows[target];
          if (existing is RowToolCall) {
            _rows[target] = RowToolCall(
              seq: existing.seq,
              at: existing.at,
              call: existing.call,
              result: v.data,
              error: v.data.error,
            );
          }
        } else {
          // 找不到对应调用（断线期间的结果、或跨会话的孤儿结果）：单独
          // 补一行。总比静默丢掉一次工具结果强——那会让用户以为 agent
          // 什么都没做。
          _rows.add(RowToolCall(
            seq: wire.seq,
            at: at,
            call: ToolCallData(
              turn: v.data.turn,
              step: v.data.step,
              callId: callId ?? '?',
              name: 'tool',
              arguments: '',
            ),
            result: v.data,
            error: v.data.error,
          ));
        }

      case TurnStartEvent v:
        _rows.add(RowTurnDivider(seq: wire.seq, at: at, turn: v.turn));
        state = state.copyWith(running: true);

      case TurnEndEvent v:
        if (v.isError || v.isAborted) {
          _rows.add(RowNotice(
            seq: wire.seq,
            at: at,
            label: v.isAborted ? '已中断' : '出错停止',
            detail:
                (v.reason['error'] != null ? asMap(v.reason['error'])['message'] as String? : null),
          ));
        }
        state = state.copyWith(running: false);

      case TodoWriteEvent v:
        _rows.add(RowTodo(seq: wire.seq, at: at, todos: v.todos));

      case SessionTitleEvent v:
        if (v.title.isNotEmpty) {
          _rows.add(RowNotice(seq: wire.seq, at: at, label: '会话改名为「${v.title}」'));
        }

      case CommandEvent v:
        _rows.add(RowNotice(
          seq: wire.seq,
          at: at,
          label: v.done ? '命令完成' : '执行命令',
          detail: v.label,
        ));

      case AssistantAttemptEvent():
      case StepEvent():
      case RequestContextEvent():
      case UnknownSessionEvent():
        // 这些是过程信号，不该在转录里占行。
        break;
    }
  }

  void _publish() {
    if (mounted) state = state.copyWith(rows: List.unmodifiable(_rows));
  }

  /// 发送一条提示。
  ///
  /// [requestId] 必须由这里生成并**持久化在宿主那条消息上**：同一条消息
  /// 的重试要带同一个 id，两条不同的消息绝不能共用。所以它在调用前
  /// 生成一次，重试路径复用同一个值。
  Future<RpcResult<Object?>> send(String text, {String? requestId}) async {
    final client = _ref.read(dshClientProvider);
    if (client == null) {
      return const RpcErr<Object?>(
        RpcError(code: RpcErrorCode.internal, message: '尚未连接到电脑'),
      );
    }
    final id = requestId ?? mintPromptRequestId();
    state = state.copyWith(sending: true, clearError: true);

    final res = await client.call('session/prompt', request: {
      'requestId': id,
      'sessionId': sessionId,
      // `queue` 排队等当前轮次跑完；`steer` 立刻插进当前轮次。
      'mode': 'queue',
      'content': [
        {'type': 'text', 'text': text},
      ],
    });

    if (mounted) {
      state = state.copyWith(
        sending: false,
        running: res.isOk,
        error: res.errorOrNull?.message,
      );
    }
    return res;
  }

  /// 中断当前轮次。保留排队中的收件箱内容。
  Future<void> cancel() async {
    final client = _ref.read(dshClientProvider);
    if (client == null) return;
    await client.call('session/cancel', request: {'sessionId': sessionId});
    if (mounted) state = state.copyWith(running: false);
  }

  /// 重新加载历史（断线重连后）。
  Future<void> reload() => _loadHistory();
}

/// 某会话的转录。
final transcriptControllerProvider = StateNotifierProvider.autoDispose
    .family<TranscriptController, TranscriptState, String>((ref, sessionId) {
  return TranscriptController(ref, sessionId);
});

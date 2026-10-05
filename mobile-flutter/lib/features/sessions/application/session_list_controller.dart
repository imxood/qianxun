import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../data/dsh/dsh_client.dart';
import '../../../data/dsh/dsh_connection.dart';
import '../../../data/dsh/rpc_envelope.dart';
import '../../../data/models/session.dart';
import '../../connections/application/connections_controller.dart';
import 'dart:async';

/// 会话列表状态。
sealed class SessionListState {
  const SessionListState();
}

class SessionListLoading extends SessionListState {
  const SessionListLoading();
}

class SessionListError extends SessionListState {
  const SessionListError(this.message, {this.needsPairing = false});
  final String message;
  final bool needsPairing;
}

class SessionListReady extends SessionListState {
  const SessionListReady(
    this.items, {
    this.loading = false,
    this.error,
    this.query = '',
  });

  final List<SessionSummary> items;
  final bool loading;

  /// 非致命错误（一次刷新失败但旧数据还在）。
  final String? error;
  final String query;

  SessionListReady copyWith({
    List<SessionSummary>? items,
    bool? loading,
    String? error,
    String? query,
    bool clearError = false,
  }) =>
      SessionListReady(
        items ?? this.items,
        loading: loading ?? this.loading,
        error: clearError ? null : (error ?? this.error),
        query: query ?? this.query,
      );
}

/// 会话列表控制器。
///
/// 数据来源：开局一次 `session/list` 拿快照，之后靠 `$events` 与
/// `workspace/follow` 增量更新。**不做轮询**——轮询会把刚发生的对话
/// 拖慢好几秒才显示。
class SessionListController extends StateNotifier<SessionListState> {
  SessionListController(this._ref) : super(const SessionListLoading()) {
    _bind();
  }

  final Ref _ref;
  StreamSubscription<Object?>? _sub;
  Timer? _debounce;

  DshClient? get _client => _ref.read(dshClientProvider);

  void _bind() {
    _sub?.cancel();
    final conn = _ref.read(selectedConnectionProvider);
    if (conn == null) {
      state = const SessionListError('还没有选择电脑。');
      return;
    }
    // ⚠️ 必须是 `watch` 而不是 `read`：`dshConnectionProvider` 是
    // `autoDispose` 的，`read` 不建立依赖，provider 可能在下一次重建时
    // 就被回收——socket 随之关闭，而界面还以为自己连着。
    final link = _ref.watch(dshConnectionProvider(conn));
    _sub = link.states.listen((s) {
      switch (s) {
        case LinkReady():
          unawaited(load());
        case LinkNeedsPairing(:final message):
          state = SessionListError(message, needsPairing: true);
        case LinkConnecting():
          if (state is! SessionListReady) state = const SessionListLoading();
        case LinkReconnecting():
        case LinkIdle():
        case LinkClosed():
          break;
      }
    });
    if (link.isReady) unawaited(load());
  }

  @override
  void dispose() {
    _sub?.cancel();
    _debounce?.cancel();
    super.dispose();
  }

  /// 拉取会话列表。
  Future<void> load() async {
    final client = _client;
    if (client == null) return;
    final current = state;
    state =
        current is SessionListReady ? current.copyWith(loading: true) : const SessionListLoading();

    final res = await client.callAs<List<SessionSummary>>(
      'session/list',
      (value) => (asMap(value)['items'] as List? ?? const [])
          .map((e) => SessionSummary.fromJson(asMap(e)))
          .toList(),
    );

    if (!mounted) return;
    state = res.map(
      (list) => SessionListReady(list) as SessionListState,
      (err) => current is SessionListReady
          ? current.copyWith(loading: false, error: err.message)
          : SessionListError(err.message, needsPairing: err.needsRepairing),
    );
  }

  /// 搜索。带防抖——每敲一个字打一次会打爆小路由上的手机。
  void search(String query) {
    _debounce?.cancel();
    final current = state;
    if (current is SessionListReady) {
      state = current.copyWith(query: query);
    }
    if (query.trim().isEmpty) {
      unawaited(load());
      return;
    }
    _debounce = Timer(const Duration(milliseconds: 350), () => _search(query));
  }

  Future<void> _search(String query) async {
    final client = _client;
    if (client == null) return;
    final res = await client.call('session/search', request: {'query': query});
    if (!mounted) return;
    final current = state;
    if (current is! SessionListReady) return;
    state = res.map(
      (value) {
        final items = (asMap(value)['items'] as List? ?? const []);
        // 搜索结果没有完整摘要，用 snippet 造一条最小可渲染的行。
        state = current.copyWith(
          loading: false,
          clearError: true,
          items: items.map((e) {
            final m = asMap(e);
            final id = (m['sessionId'] as String?) ?? '';
            final snippet = (m['snippet'] as String?) ?? '';
            return SessionSummary(
              sessionId: id,
              updatedAt: DateTime.now().millisecondsSinceEpoch,
              running: false,
              blank: false,
              cwd: snippet,
            );
          }).toList(),
        );
        return current;
      },
      (err) => current.copyWith(loading: false, error: err.message) as SessionListState,
    );
  }

  /// 新建会话。
  Future<RpcResult<String>> create({String? cwd, String? workspaceId}) async {
    final client = _client;
    if (client == null) {
      return const RpcErr<String>(
        RpcError(code: RpcErrorCode.internal, message: '尚未连接到电脑'),
      );
    }
    final res = await client.callAs<String>(
      'session/create',
      (value) => (asMap(value)['sessionId'] as String?) ?? '',
      request: {
        if (cwd != null) 'cwd': cwd,
        if (workspaceId != null) 'workspaceId': workspaceId,
      },
    );
    if (res.isOk) unawaited(load());
    return switch (res) {
      RpcOk<String>(:final value) => RpcOk<String>(value),
      RpcErr<String>(:final error) => RpcErr<String>(error),
    };
  }
}

/// 选中连接的会话列表。
final sessionListControllerProvider =
    StateNotifierProvider.autoDispose<SessionListController, SessionListState>((ref) {
  return SessionListController(ref);
});

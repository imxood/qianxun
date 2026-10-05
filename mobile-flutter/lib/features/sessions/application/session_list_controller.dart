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
  const SessionListError(
    this.message, {
    this.needsPairing = false,
    this.needsMachineChoice = false,
  });

  final String message;
  final bool needsPairing;

  /// 保存了多台电脑、但**还没选哪一台**。
  ///
  /// ⚠️ 这与「连不上」是**两件事**，曾经混在一起，于是多机用户每次冷启动
  /// 都看到「连不上这台电脑」加一个「重试」按钮 —— 什么都没有坏，
  /// 重试一万次也没用。App 只是不肯替他挑一台（05 §5.2：多台时不自动选，
  /// 那是他要做的决定）。把两者分开，这里就该引导他去**选**，不是去**重试**。
  final bool needsMachineChoice;
}

class SessionListReady extends SessionListState {
  const SessionListReady(
    this.items, {
    this.loading = false,
    this.error,
  });

  final List<SessionSummary> items;
  final bool loading;

  /// 非致命错误（一次刷新失败但旧数据还在）。
  final String? error;

  SessionListReady copyWith({
    List<SessionSummary>? items,
    bool? loading,
    String? error,
    bool clearError = false,
  }) =>
      SessionListReady(
        items ?? this.items,
        loading: loading ?? this.loading,
        error: clearError ? null : (error ?? this.error),
      );
}

/// 会话列表控制器。
///
/// 数据来源：开局一次 `session/list` 拿快照。**不做轮询**——轮询会把刚
/// 发生的对话拖慢好几秒才显示。
///
/// ⚠️ 这里**刻意没有搜索**。`session/search` 在本部署是 `openAt "never"`
/// （已实测），是一个必然返回空/报错的死功能；05 §11 把它和搜索框一起
/// 列为「明确不做」。曾经这里有 `search()` + 350ms 防抖 + 一个把
/// `snippet` 硬塞进 `cwd` 的临时行构造函数——那套代码让搜索框看起来
/// 能用，实际一搜就空。现在要做的是排序（`等你` 置顶），见
/// `session_presentation.dart`。
class SessionListController extends StateNotifier<SessionListState> {
  SessionListController(this._ref) : super(const SessionListLoading()) {
    _bind();
  }

  final Ref _ref;
  StreamSubscription<Object?>? _sub;

  DshClient? get _client => _ref.read(dshClientProvider);

  void _bind() {
    _sub?.cancel();
    final conn = _ref.read(selectedConnectionProvider);
    if (conn == null) {
      // 「还没选机器」≠「连不上」。见 [SessionListError.needsMachineChoice]。
      final saved = _ref.read(connectionsControllerProvider);
      final many = saved is ConnectionsReady ? saved.list.length > 1 : false;
      state = SessionListError(
        many ? '选一台电脑，就能看到它的会话了。' : '还没有选择电脑。',
        needsMachineChoice: many,
      );
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

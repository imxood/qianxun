import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../data/dsh/dsh_client.dart';
import '../../../data/dsh/dsh_connection.dart';
import '../../../data/dsh/rpc_envelope.dart';
import '../../../data/models/session.dart';
import '../../../data/store/connection_store.dart';
import '../../connections/application/connections_controller.dart';
import 'dart:async';

/// 会话列表状态。
sealed class SessionListState {
  const SessionListState();
}

class SessionListLoading extends SessionListState {
  const SessionListLoading();
}

/// 会话列表「出不去」的原因。
///
/// **刻意做成单一判别式，而不是几个布尔量。** 早先这里是
/// `needsPairing` + `needsMachineChoice` 两个 bool，能组成 4 种组合，
/// 其中「既要重新配对、又要选机器」是自相矛盾的，而控制器完全可能
/// 产出它 —— 于是 UI 落到 `else` 分支，显示「连不上这台电脑」。
/// 一个枚举字段从结构上消掉了这类不可能状态。
enum ListBlockReason {
  /// 首次使用：一台电脑都没配。下一步是**配对**。
  noMachines,

  /// 配过，但 token 失效 / 被撤销。下一步是**重新配对**。
  needsRepair,

  /// 存了多台，还没选哪一台。下一步是**选**。
  needsMachineChoice,

  /// 选好了也连不上。下一步是**重试**。
  unreachable,
}

class SessionListError extends SessionListState {
  const SessionListError(this.message, {required this.reason});

  final String message;
  final ListBlockReason reason;

  /// 兼容旧调用点。⚠️ 新的代码请用 [reason] —— 这个 getter 只为
  /// 过渡期保留，它**无法**区分 noMachines 与 needsRepair，
  /// 而两者的文案不同（「还没有电脑」vs「需要重新配对」）。
  @Deprecated('用 reason —— 这个 getter 分不清「没配过」和「要重配」')
  bool get needsPairing =>
      reason == ListBlockReason.needsRepair || reason == ListBlockReason.noMachines;
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
    // ⚠️ 必须监听「选中哪台机器」，变化时**重新绑定**。
    //
    // 曾经只在构造里 `_bind()` 一次，于是冷启动（还没配对）时状态被定成
    // `noMachines` 错误态；用户配好对、选中那台机器之后，没有任何东西
    // 重新求值，界面就一直停在「还没有电脑 / 去配对」——
    // 而实际上机器已经选好了、链路也已经就绪。实测在模拟器上：连接是通的，
    // 列表页却还在喊「没有电脑」，点「去配对」会把用户送回他已经做完的事。
    _ref.listen<Connection?>(selectedConnectionProvider, (_, __) => _bind());
    _bind();
  }

  final Ref _ref;
  StreamSubscription<Object?>? _sub;

  DshClient? get _client => _ref.read(dshClientProvider);

  void _bind() {
    _sub?.cancel();
    final conn = _ref.read(selectedConnectionProvider);
    if (conn == null) {
      // ⚠️ 这里必须分**四种**，不是两种。
      //
      // 曾经只分「多台 / 其余」，于是**全新安装、一台电脑都没配**的用户
      // 看到的是「连不上这台电脑」+「重试」—— 什么都没坏，一台机器都
      // 还没有，重试一万次也没用。首次体验是最不能搞砸的一屏。
      //
      // 四者的下一步完全不同：配对 / 选 / 重试。
      final saved = _ref.read(connectionsControllerProvider);
      final n = saved is ConnectionsReady ? saved.list.length : 0;
      state = switch (n) {
        0 => const SessionListError(
            '还没有电脑。先在这台手机上配对一次，之后就能看到它的会话了。',
            reason: ListBlockReason.noMachines,
          ),
        1 => const SessionListError(
            '还没有选择电脑。',
            reason: ListBlockReason.unreachable,
          ),
        _ => const SessionListError(
            '选一台电脑，就能看到它的会话了。',
            reason: ListBlockReason.needsMachineChoice,
          ),
      };
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
          state = SessionListError(message, reason: ListBlockReason.needsRepair);
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
          : SessionListError(
              err.message,
              reason: err.needsRepairing
                  ? ListBlockReason.needsRepair
                  : ListBlockReason.unreachable,
            ),
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

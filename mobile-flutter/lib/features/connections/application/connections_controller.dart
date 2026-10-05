import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../data/dsh/dsh_client.dart';
import '../../../data/dsh/dsh_connection.dart';
import '../../../data/dsh/gateway_client.dart';
import '../../../data/dsh/pair_url.dart';
import '../../../data/store/connection_store.dart';
import 'dart:async';

/// 连接仓库。覆盖写之前先读，避免两个写者互相覆盖。
final connectionStoreProvider = Provider<ConnectionStore>((ref) => ConnectionStore());

/// 当前选中的连接 id。
final selectedConnectionIdProvider = StateProvider<String?>((ref) => null);

/// 连接列表状态。
sealed class ConnectionsState {
  const ConnectionsState();
}

class ConnectionsLoading extends ConnectionsState {
  const ConnectionsLoading();
}

class ConnectionsReady extends ConnectionsState {
  const ConnectionsReady(this.list, {this.notice, this.noticeIsError = false});

  final List<Connection> list;

  /// 一次性提示语（添加/更新/删除的结果）。
  final String? notice;
  final bool noticeIsError;

  ConnectionsReady copyWith({
    List<Connection>? list,
    String? notice,
    bool? noticeIsError,
    bool clearNotice = false,
  }) =>
      ConnectionsReady(
        list ?? this.list,
        notice: clearNotice ? null : (notice ?? this.notice),
        noticeIsError: noticeIsError ?? this.noticeIsError,
      );
}

/// 连接列表控制器。
///
/// 职责：加载/持久化/增删改 + **在线状态探测**。
/// 探测用 `GatewayClient`，因为它只需要 `/qx-mobile/info` 这一个路由，
/// 不值得为了一次点检就建一整条 DSH 连接。
class ConnectionsController extends StateNotifier<ConnectionsState> {
  ConnectionsController(this._store) : super(const ConnectionsLoading()) {
    _bootstrap();
  }

  final ConnectionStore _store;
  final _gateway = GatewayClient();

  /// 每台电脑的最近探测结果。key = connection id。
  final Map<String, ProbeResult> _probes = {};
  Timer? _probeTimer;
  bool _probing = false;

  @override
  void dispose() {
    _probeTimer?.cancel();
    _gateway.dispose();
    super.dispose();
  }

  Future<void> _bootstrap() async {
    final list = await _store.load();
    if (mounted) {
      state = ConnectionsReady(_sorted(list));
      unawaited(refreshStatuses());
      _probeTimer = Timer.periodic(
        const Duration(seconds: 25),
        (_) => unawaited(refreshStatuses()),
      );
    }
  }

  List<Connection> _sorted(List<Connection> list) => [...list]..sort(Connection.compareFresh);

  void _persist(List<Connection> list) {
    final current = state;
    if (current is! ConnectionsReady) return;
    state = ConnectionsReady(_sorted(list),
        notice: current.notice, noticeIsError: current.noticeIsError);
    unawaited(_store.save(list));
  }

  /// 重新探测全部连接的在线状态。
  ///
  /// 逐个错开 120ms：一排同时发起的请求会让小路由上的手机瞬间打满，
  /// 而且返回顺序乱掉会让状态点闪烁。
  Future<void> refreshStatuses() async {
    final current = state;
    if (current is! ConnectionsReady || _probing || current.list.isEmpty) return;
    _probing = true;
    try {
      for (final conn in current.list) {
        await Future<void>.delayed(const Duration(milliseconds: 120));
        if (!mounted) return;
        final result = await _gateway.probe(conn.url);
        // 存了兑换 token 的连接直接查，绕过一次性 code。
        _probes[conn.id] = result;
      }
      if (mounted) _bump();
    } finally {
      _probing = false;
    }
  }

  void _bump() {
    final current = state;
    if (current is ConnectionsReady) {
      state = ConnectionsReady(current.list,
          notice: current.notice, noticeIsError: current.noticeIsError);
    }
  }

  /// 某条连接的探测结果。
  ProbeResult? probeOf(String connectionId) => _probes[connectionId];

  /// 添加一条连接。
  ///
  /// 离线也允许保存——用户可能在飞机上先记下来，落地再连。状态点会
  /// 在能连上时自动点亮。
  Future<OpResult> add(String rawUrl, {String? manualName}) async {
    final url = rawUrl.trim();
    final pair = PairUrl.tryParse(url);
    if (pair == null) {
      return const OpResult.fail(
        '链接形态不对：请扫电脑端「远程访问」的配对二维码，或完整粘贴配对链接。\n'
        '如果链接是 http://:23090/… 这种没有主机名的，说明电脑端还没开启远程访问。',
      );
    }

    final current = state;
    if (current is ConnectionsReady && current.list.any((c) => c.url == pair.normalized)) {
      return const OpResult.fail('这条连接已经存在。');
    }

    final result = await _gateway.probe(url);
    if (!mounted) return const OpResult.ok('已添加。');

    // 鉴权失败：链接本身是坏的，**不该**保存成一个永远连不上的条目。
    if (result case ProbeUnauthorized(:final message)) {
      return OpResult.fail('配对失败：$message');
    }

    // 在线时用电脑的真实身份做默认名；离线时退回到地址里的主机名。
    final ProbeOk? ok = result is ProbeOk ? result : null;
    final autoName = ok?.info.hostname;

    final now = DateTime.now().millisecondsSinceEpoch;
    final manual = manualName?.trim() ?? '';
    final draft = Connection(
      id: ConnectionStore.newId(),
      name: manual.isNotEmpty ? manual : (autoName ?? pair.host),
      url: pair.normalized,
      addedAt: now,
      qxToken: ok?.qxToken,
      hostname: ok?.info.hostname,
      appVersion: ok?.info.version,
    );

    final list = current is ConnectionsReady ? current.list : <Connection>[];
    _probes[draft.id] = result;
    _persist([...list, draft]);
    _bump();

    return switch (result) {
      ProbeOk(:final info) => OpResult.ok(
          '已添加「${draft.name}」${info.dshReady ? '，DSH 运行中' : '（DSH 未运行，稍后可连）'}',
        ),
      ProbeUpstreamDown() => const OpResult.ok('已按地址保存，但电脑端 DSH 没在运行。'),
      _ => const OpResult.ok('暂时连不上这台电脑（可能离线），已按地址保存，连上后状态点亮。'),
    };
  }

  void rename(String id, String name) {
    final trimmed = name.trim();
    if (trimmed.isEmpty) return;
    final current = state;
    if (current is! ConnectionsReady) return;
    _persist(current.list.map((c) => c.id == id ? c.copyWith(name: trimmed) : c).toList());
  }

  /// 更新配对链接（等同于桌面端的「吊销 + 重新配对」）。
  OpResult updateUrl(String id, String url) {
    final pair = PairUrl.tryParse(url);
    if (pair == null) {
      return const OpResult.fail('链接形态不对：应来自千寻设置页的配对二维码/复制链接。');
    }
    final current = state;
    if (current is! ConnectionsReady) return const OpResult.ok('');
    // 换了链接 → 旧 token 作废，必须清掉，否则会用旧凭据连新主机。
    _persist(current.list
        .map((c) => c.id == id ? c.copyWith(url: pair.normalized, clearToken: true) : c)
        .toList());
    _probes.remove(id);
    unawaited(refreshStatuses());
    return const OpResult.ok('配对链接已更新，正在重新握手。');
  }

  void remove(String id) {
    final current = state;
    if (current is! ConnectionsReady) return;
    _probes.remove(id);
    _persist(current.list.where((c) => c.id != id).toList());
    _bump();
  }

  /// 标记「刚刚用过」。
  void touch(String id) {
    final current = state;
    if (current is! ConnectionsReady) return;
    _persist(current.list
        .map((c) => c.id == id ? c.copyWith(lastUsedAt: DateTime.now().millisecondsSinceEpoch) : c)
        .toList());
  }

  /// 清掉一次性提示。
  void clearNotice() {
    final current = state;
    if (current is ConnectionsReady && current.notice != null) {
      state = current.copyWith(clearNotice: true);
    }
  }
}

final connectionsControllerProvider =
    StateNotifierProvider<ConnectionsController, ConnectionsState>((ref) {
  return ConnectionsController(ref.watch(connectionStoreProvider));
});

/// 当前选中的连接。
final selectedConnectionProvider = Provider<Connection?>((ref) {
  final id = ref.watch(selectedConnectionIdProvider);
  final state = ref.watch(connectionsControllerProvider);
  if (id == null || state is! ConnectionsReady) return null;
  for (final c in state.list) {
    if (c.id == id) return c;
  }
  return null;
});

/// 选中连接对应的 DSH 连接（`$events` + 重连代际）。
///
/// 只要选中的连接变了就整条重建——凭据、origin、token 三个都不该跨连接复用。
///
/// ⚠️ **必须在这里 `start()`**。`DshConnection` 是「惰性」的：构造它只会
/// 建对象、建退避定时器，**不会**去连 WebSocket。谁负责发起第一次握手是
/// 这个 provider 的责任——之前漏掉这一句，连接对象就永远停在 `LinkIdle`，
/// 界面表现是「未连接」加一个永远转不完的圈。
final dshConnectionProvider = Provider.autoDispose.family<DshConnection, Connection?>((ref, conn) {
  final connection = DshConnection(
    origin: conn?.pair?.origin ?? 'http://127.0.0.1:1',
    qxToken: conn?.effectiveToken ?? '',
  );
  // 离开工作台时真正关掉 socket 与重连定时器。
  ref.onDispose(connection.dispose);
  if (conn != null) {
    connection.start();
  }
  return connection;
});

/// DSH 一元 RPC 客户端。未就绪时为 null。
final dshClientProvider = Provider<DshClient?>((ref) {
  final conn = ref.watch(selectedConnectionProvider);
  if (conn == null) return null;
  return ref.watch(dshConnectionProvider(conn)).client;
});

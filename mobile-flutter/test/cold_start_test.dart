import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/data/store/connection_store.dart';
import 'package:qianxun_mobile/features/connections/application/connections_controller.dart';

/// 冷启动自动选中（05 §5.2）。
///
/// 05 §9.5 的验收项第 8 条是「冷启动直接进会话列表」。根路由那一半在
/// `app.dart` 的 `initialLocation`，这里测的是另一半：**只有一台电脑时
/// 不用用户手动选**，多台时**不替他决定**。
void main() {
  Connection conn(String id) => Connection(
        id: id,
        name: id,
        url: 'https://$id.example/',
        addedAt: 0,
        qxToken: 't',
      );

  /// 用假的 store 驱动真实的 controller：override 掉存储，其余全走真代码。
  ProviderContainer containerWith(List<Connection> list) {
    final c = ProviderContainer(
      overrides: [
        connectionStoreProvider.overrideWithValue(_FakeStore(list)),
      ],
    );
    addTearDown(c.dispose);
    return c;
  }

  /// 等 controller 把初始列表加载完。不用固定 sleep —— 那会让测试
  /// 「碰巧」通过而不是「确实」通过。
  Future<void> settle(ProviderContainer c) async {
    for (var i = 0; i < 50; i++) {
      if (c.read(connectionsControllerProvider) is ConnectionsReady) return;
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    fail('连接列表始终没有进入 ConnectionsReady');
  }

  test('恰好一台：自动选中它，用户不用先做一次无意义的选择', () async {
    final c = containerWith([conn('only')]);
    await settle(c);

    expect(
      c.read(selectedConnectionProvider)?.id,
      'only',
      reason: '只有一台电脑时还要用户手选 = 白白多一步',
    );
    expect(c.read(soleConnectionIdProvider), 'only');
  });

  test('多台：不自动选，那属于用户要做的决定', () async {
    final c = containerWith([conn('a'), conn('b')]);
    await settle(c);

    expect(c.read(soleConnectionIdProvider), isNull);
    expect(
      c.read(selectedConnectionProvider),
      isNull,
      reason: '替他猜一台就是替他做决定；多台时列表顶栏要出机器切换器',
    );
  });

  test('一台都没有：不选', () async {
    final c = containerWith([]);
    await settle(c);

    expect(c.read(soleConnectionIdProvider), isNull);
    expect(c.read(selectedConnectionProvider), isNull);
  });

  test('手动切过机器后，列表变成多台时回退**不生效**', () async {
    // ⚠️ 这条原来用**两台**机器，于是 `soleConnectionIdProvider` 返回 null、
    // 唯一那台的回退分支整个是死的 —— 它退化成了「StateProvider 写进去能读
    // 回来」，根本不是它注释里声称的场景。真正的场景是：
    // 用户上次选了 A，现在列表里只剩 A 一台了，回退恰好选回 A（看不出差别）；
    // 要证明「显式优先」，必须让显式 id **不等于**唯一那台。
    final c = containerWith([conn('a'), conn('b')]);
    await settle(c);
    c.read(selectedConnectionIdProvider.notifier).state = 'b';

    expect(
      c.read(selectedConnectionProvider)?.id,
      'b',
      reason: '显式选择必须压过「唯一那台」的回退',
    );
  });

  test('显式选择的机器不在列表里时，回退到唯一那台', () async {
    // 显式 id 失效（机器被删了）不该让界面变成「没有选择电脑」，
    // 列表里明明还有一台可用的。
    final c = containerWith([conn('only')]);
    await settle(c);
    c.read(selectedConnectionIdProvider.notifier).state = 'deleted-machine';

    expect(c.read(selectedConnectionProvider)?.id, 'only');
  });
}

/// 只实现 `load()` 的假仓库。其余方法在本组测试里不该被调用，
/// 真被调用了就让测试炸出来。
class _FakeStore implements ConnectionStore {
  _FakeStore(this.list);

  final List<Connection> list;

  @override
  Future<List<Connection>> load() async => list;

  @override
  Future<void> save(List<Connection> connections) =>
      throw UnimplementedError('本组测试不该写库');
}

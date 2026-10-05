import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/data/store/connection_store.dart';

/// `Connection` 的值相等性。
///
/// 这不是「纯洁癖」。`dshConnectionProvider` 是 `Provider.family`，family
/// 用 `==` 定位条目。`Connection` 没有 `==` 时用的是**对象身份**，而连接
/// 列表每次重新加载都造出全新实例 —— 于是每次加载都重建 `DshConnection`、
/// 重连一次 WebSocket。
///
/// 实测症状：App 内部日志里连续出现多个 `link 发起连接 #N` / `★ 就绪 #N`
/// 且 clientId 各不相同，界面永远停在「正在连接电脑…」。**只有真机上看得见**
/// —— widget 测试不加载 SharedPreferences，碰不到这个循环。
void main() {
  Connection c({
    String id = 'c1',
    String name = '我的电脑',
    String url = 'http://192.168.20.2:23090/qx-gate?token=abc',
    String? token = 'abc',
    int lastUsedAt = 0,
  }) =>
      Connection(
        id: id,
        name: name,
        url: url,
        addedAt: 0,
        lastUsedAt: lastUsedAt,
        qxToken: token,
      );

  group('相同内容 → 相等（family 键稳定，不重连）', () {
    test('重新加载造出的新实例与旧实例相等', () {
      // 这正是 SharedPreferences 每次 load() 的产物。
      final fromDisk1 = c();
      final fromDisk2 = c();
      expect(identical(fromDisk1, fromDisk2), isFalse, reason: '必须是两个对象');
      expect(fromDisk1, equals(fromDisk2), reason: '但必须相等，否则 family 重建');
      expect(fromDisk1.hashCode, fromDisk2.hashCode);
    });

    test('toJson → fromJson 往返后仍然相等', () {
      final a = c();
      final b = Connection.fromJson(a.toJson());
      expect(a, equals(b));
      expect(a.hashCode, b.hashCode);
    });

    test('展示用的字段变了**不**影响相等（不该为此重连）', () {
      // 改个昵称、点开一次刷新一下 lastUsedAt，都不该把 socket 拆了。
      expect(c(), equals(c(name: '书房那台')));
      expect(c(), equals(c(lastUsedAt: 999999)));
    });
  });

  group('影响连接的内容变了 → 不相等（必须重建）', () {
    test('换机器（id 不同）', () {
      expect(c(), isNot(equals(c(id: 'c2'))));
    });

    test('改地址（url 不同）', () {
      expect(c(), isNot(equals(c(url: 'http://10.0.0.5:23090/qx-gate?token=abc'))));
    });

    test('同一台机器重新配对：id 不变但 token 变了', () {
      // ⚠️ `copyWith` 保留 id，所以**只有**把 qxToken 算进相等性，
      // 重新配对后连接才会用上新凭据。若只比 id，这里会漏。
      final before = c(token: 'old-token');
      final after = before.copyWith(qxToken: 'new-token');
      expect(after.id, before.id, reason: '前提：id 确实没变');
      expect(before, isNot(equals(after)), reason: 'token 变了必须重建');
    });

    test('token 从有变成无', () {
      expect(c(token: 'abc'), isNot(equals(c(token: null))));
    });
  });

  test('放进 Set / Map 不会重复', () {
    // family 之外的另一处依赖：集合去重也用 ==。
    final set = {c(), c(), c(id: 'c2')};
    expect(set.length, 2);
  });
}

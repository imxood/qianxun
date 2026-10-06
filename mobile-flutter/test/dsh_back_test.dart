import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/data/store/connection_store.dart';
import 'package:qianxun_mobile/features/dsh/presentation/dsh_credential.dart';
import 'package:qianxun_mobile/features/dsh/presentation/webview_host_state.dart';

void main() {
  group('凭据解析 —— 真机才暴露的那个坑', () {
    // 实测模拟器上的真实存储（RN 客户端写的）：
    //   { "url": "http://192.168.20.2:23090/qx-gate?token=<64hex>",
    //     "qxToken": null }
    //
    // 症状：设备列表显示「在线」（探活走 GatewayClient，它自己会从 url
    // 提 token），但 DSH 页说「这条连接没有凭据」。
    // 根因：WebView 宿主只读 `connection.qxToken`，而旧数据里它是 null。

    test('从 url 的 ?token= 提取（64 位 hex）', () {
      const hex =
          '746f7635ddef778bcfeb8cf0a38e6f0720b8d7d0517a49053d1ae22b9d76f60c';
      final url = Uri.parse('http://192.168.20.2:23090/qx-gate?token=$hex');
      expect(resolveCredential(url, null), hex);
    });

    test('qxToken 字段优先于 url（旧数据两者都在时用字段）', () {
      const fromField = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
      final url = Uri.parse('http://192.168.20.2:23090/qx-gate?token=bbbb');
      expect(resolveCredential(url, fromField), fromField);
    });

    test('?code= 是一次性兑换码，提不出 token → 报「重新配对」', () {
      // ⚠️ 不能把 code 当 token 用 —— 它必须真的去 /qx-gate 兑换一次。
      // 拿它当 cookie 发出去会得到 401，且用户完全看不懂为什么。
      final url = Uri.parse('http://192.168.20.2:23090/qx-gate?code=abc123');
      expect(resolveCredential(url, null), isNull);
    });

    test('两处都没有 → null', () {
      final url = Uri.parse('http://192.168.20.2:23090/');
      expect(resolveCredential(url, null), isNull);
      expect(resolveCredential(url, ''), isNull);
    });

    test('太短的 token 视为无效', () {
      // 1 位不是 token，硬用会得到一个毫无意义的 401。
      final url = Uri.parse('http://192.168.20.2:23090/qx-gate?token=x');
      expect(resolveCredential(url, null), isNull);
    });

    test('**只从 url 提，不改 url 本身**', () {
      // 提取后若把它写回 url，带凭据的 URL 会进日志、进路由、
      // 进而进浏览历史。token 只该进 cookie。
      const hex =
          '746f7635ddef778bcfeb8cf0a38e6f0720b8d7d0517a49053d1ae22b9d76f60c';
      final url = Uri.parse('http://192.168.20.2:23090/qx-gate?token=$hex');
      final token = resolveCredential(url, null);
      expect(token, isNotNull);

      // 原 url 未被改写（它仍是入参，不可变）
      expect(url.toString(), contains('token=$hex'));

      // 而提供给 WebView 的必须**没有凭据**。
      // ⚠️ stripCredential 只清 query/fragment，**保留 path** ——
      //    这是刻意的：万一将来有需要带路径的地址形态，清掉 path
      //    会把地址悄悄改成站点根。现在调用方另外做 `resolve('/')`。
      final stripped = stripCredential(url);
      expect(stripped.query, isEmpty);
      expect(stripped.toString(), isNot(contains(hex)));
      expect(stripped.resolve('/').toString(), 'http://192.168.20.2:23090/');
    });
  });

  group('Connection 值相等仍含凭据', () {
    test('同机器重新配对（token 变了）→ 视为不同连接', () {
      // 没有这条，重新配对后 WebView 会继续用旧 cookie，
      // 表现为「配对成功了但还是 401」。
      const a = Connection(
        id: 'c1',
        name: 'PC',
        url: 'http://192.168.20.2:23090/qx-gate?token=old',
        addedAt: 1,
        qxToken: 'old',
      );
      final b = a.copyWith(qxToken: 'new');
      expect(a, isNot(equals(b)));
    });
  });

  group('decideBackAction 三级递进', () {
    // 顺序不可调换（10-shell-bridge.md §4）：
    //   1. WebView 有历史 → 后退
    //   2. 在 DSH 页 → 回设备列表
    //   3. 在设备列表根页 → 退出
    test('有 WebView 历史时**优先**后退，不回设备列表', () {
      final action = decideBackAction(
        webCanGoBack: true,
        routeCanPop: true,
        onDeviceListRoot: false,
      );
      expect(action, DshBackAction.webHistory);
    });

    test('无 WebView 历史但路由可 pop → 回设备列表', () {
      final action = decideBackAction(
        webCanGoBack: false,
        routeCanPop: true,
        onDeviceListRoot: false,
      );
      expect(action, DshBackAction.deviceList);
    });

    test('到设备列表根页（路由不可 pop）→ 退出', () {
      final action = decideBackAction(
        webCanGoBack: false,
        routeCanPop: false,
        onDeviceListRoot: true,
      );
      expect(action, DshBackAction.exit);
    });

    test('⚠️ 有历史时**绝不**退出 —— 即使已在根页', () {
      // 场景：用户在 DSH 里打开了帮助页 / 外部链接返回后还有历史，
      // 而 Flutter 栈恰好只剩一层。这时按返回应当退历史，不是退出 App。
      final action = decideBackAction(
        webCanGoBack: true,
        routeCanPop: false,
        onDeviceListRoot: true,
      );
      expect(action, DshBackAction.webHistory,
          reason: '有历史就退历史，退出是最不该发生的结果');
    });

    test('⚠️ 路由不可 pop 但不在根页 → 仍回设备列表，不退出', () {
      // onDeviceListRoot 为 false 时不该退出 —— 退出是不可逆的。
      final action = decideBackAction(
        webCanGoBack: false,
        routeCanPop: false,
        onDeviceListRoot: false,
      );
      expect(action, DshBackAction.deviceList);
    });
  });

  group('WebViewHostState 状态文案', () {
    test('ready 时不显示状态条', () {
      const state = WebViewHostState(phase: DshHostPhase.ready);
      expect(state.statusText, isNull);
      expect(state.isBusy, isFalse);
    });

    test('customLayerMissing 与 failed 是**两件事**，文案要不同', () {
      // 这两种白屏的排查方向完全相反：
      //   missing → 页面加载了，但 custom.js 没执行（看 mobile-access 目录）
      //   failed  → 页面根本没加载（看网络 / 清单 / 电脑是否在跑）
      const missing = WebViewHostState(phase: DshHostPhase.customLayerMissing);
      const failed = WebViewHostState(phase: DshHostPhase.failed, detail: '超时');

      expect(missing.statusText, contains('custom.js'));
      expect(missing.statusText, isNot(contains('加载失败')));
      expect(failed.statusText, contains('加载失败'));
      expect(failed.statusText, contains('超时'));
    });

    test('loading / probing 都是 busy', () {
      expect(
        const WebViewHostState(phase: DshHostPhase.loading).isBusy,
        isTrue,
      );
      expect(
        const WebViewHostState(phase: DshHostPhase.probingCustomLayer).isBusy,
        isTrue,
      );
    });

    test('statusText **不泄漏凭据**', () {
      // detail 会被 UI 与日志读。若它含 token，就是一次泄漏。
      const state = WebViewHostState(
        phase: DshHostPhase.failed,
        detail: '连接 192.168.20.2:23090 超时',
      );
      expect(state.statusText, isNot(contains('qx_token')));
    });
  });
}

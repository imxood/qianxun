import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/features/dsh/presentation/webview_host_state.dart';

void main() {
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

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../data/store/connection_store.dart';
import '../features/connections/application/connections_controller.dart';
import '../features/connections/presentation/connections_screen.dart';
import '../features/dsh/presentation/dsh_page.dart';
import '../features/logs/presentation/logs_screen.dart';
import 'theme/qx_theme.dart';

final _router = GoRouter(
  // 方案 B：**根路由是设备列表**，不是会话列表。
  //
  // 05 §5.2 曾把冷启动落在会话列表，理由是「用户 95% 的冷启动是想看 agent」。
  // 那是遥控器方案下的判断。方案 B 之后 DSH 页面由 WebView 承载
  // （`/dsh/:id`），而 WebView 里的会话列表是 DSH 自己的 ——
  // 我们在 Flutter 侧再画一个会话列表就成了**两套 UI**。
  //
  // 所以：
  //   /            设备列表（壳）
  //   /dsh/:id     DSH 页面（WebView 承载，独立页面，带返回）
  //
  // 选设备 → push 进 DSH；返回 → 回设备列表。
  initialLocation: '/',
  routes: [
    GoRoute(
      path: '/',
      builder: (context, state) => ConnectionsScreen(
        onOpen: (conn) {
          HapticFeedback.selectionClick();
          final container = ProviderScope.containerOf(context, listen: false);
          container.read(selectedConnectionIdProvider.notifier).state = conn.id;
          container.read(connectionsControllerProvider.notifier).touch(conn.id);
          // ⚠️ 必须用 `push` 而不是 `go`：`go` 替换整条路由栈，从 DSH 页
          // 按返回就没有可 pop 的页，Android 直接杀进程。`push` 保留栈。
          context.push('/dsh/${Uri.encodeComponent(conn.id)}');
        },
      ),
    ),
    GoRoute(
      path: '/dsh/:connectionId',
      builder: (context, state) {
        final id = state.pathParameters['connectionId'] ?? '';
        final container = ProviderScope.containerOf(context, listen: false);
        final conn = _findConnection(container, id);
        if (conn == null) {
          // 找不到这条连接：回设备列表而不是显示空页 ——
          // 空页会让用户以为「坏了」，而实际问题只是「这条已被删除」。
          return const _MissingConnection();
        }
        return DshPage(connection: conn);
      },
    ),
    GoRoute(
      path: '/logs',
      builder: (context, state) => const LogsScreen(),
    ),
  ],
);

/// 按 id 找连接。找不到返回 null。
///
/// 不用 `firstWhere(orElse:)` 塞一个空 `Connection` —— 那会让 DSH 页
/// 拿一个空 url 去加载，表现为「白屏 + 网络错误」，而真实原因是
/// 「这条连接已经不在了」。两者的排查方向完全不同。
Connection? _findConnection(ProviderContainer container, String id) {
  final state = container.read(connectionsControllerProvider);
  if (state is! ConnectionsReady) return null;
  for (final conn in state.list) {
    if (conn.id == id) return conn;
  }
  return null;
}

/// 连接已不在（被删或 id 变了）。给一个能回到列表的页面，而不是白屏。
class _MissingConnection extends StatelessWidget {
  const _MissingConnection();

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('找不到这台电脑')),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text('这条连接已经不在了。'),
              const SizedBox(height: 16),
              FilledButton(
                onPressed: () => context.go('/'),
                child: const Text('回到设备列表'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 应用根。
class QxApp extends StatelessWidget {
  const QxApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp.router(
      title: '千寻',
      debugShowCheckedModeBanner: false,
      routerConfig: _router,
      theme: QxTheme.light(),
      darkTheme: QxTheme.dark(),
      // 深色优先：这是一块常在暗处被掏出来的屏幕，亮色留给强光环境。
      themeMode: ThemeMode.dark,
      builder: (context, child) {
        // ⚠️ 方案 B 之后**不再**保活 `connectionRootProvider`。
        //
        // 那是遥控器方案留下的：当时待答审批由 Flutter 侧的
        // `interactionController` 管，所以必须全局保活（否则用户停在
        // 任何界面审批都会消失，见 05 §0.1 的 P0）。
        //
        // 现在 DSH 页面由 WebView 承载，审批由页面里的 `custom.js` 直接
        // 读 `projections.userQuestions` —— 那是**服务端数据**，不是
        // Flutter 侧的本地状态，页面刷新就会重新拉。没有本地状态会丢，
        // 也就没有保活的必要。
        //
        // 尊重系统的字号缩放。
        //
        // 上限 0.9–2.0（不是原来的 0.9–1.3）。04 裁决表明确：夹取是为了
        // 让布局不溢出，**不是为了压住无障碍**；把上限卡在 1.3 等于告诉
        // 「需要大字号的用户」这个 App 不为你服务。
        //
        // ⚠️ 注意：这只影响**壳**的 UI（设备列表、日志页、DSH 页的顶栏）。
        // WebView 里的 DSH 页面由 `custom.css` 的 `font-size: 16px`
        // 控制，那条是 iOS 缩放的硬底线，两者不冲突。
        final media = MediaQuery.of(context);
        final clamped = media.textScaler.clamp(
          minScaleFactor: 0.9,
          maxScaleFactor: 2.0,
        );
        return MediaQuery(
          data: media.copyWith(textScaler: clamped),
          child: child ?? const SizedBox.shrink(),
        );
      },
    );
  }
}

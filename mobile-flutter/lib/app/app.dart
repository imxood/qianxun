import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../features/connections/application/connections_controller.dart';
import '../features/connections/presentation/connections_screen.dart';
import '../features/logs/presentation/logs_screen.dart';
import '../features/sessions/presentation/sessions_screen.dart';
import '../features/workspace/application/interaction_controller.dart';
import '../features/workspace/presentation/session_screen.dart';
import 'theme/qx_theme.dart';

final _router = GoRouter(
  initialLocation: '/',
  routes: [
    GoRoute(
      path: '/',
      builder: (context, state) => ConnectionsScreen(
        onOpen: (conn) {
          HapticFeedback.selectionClick();
          // 选中连接是全局状态：会话列表、交互控制器都靠它定位到这条
          // DSH 连接。
          //
          // ⚠️ 必须用 `push` 而不是 `go`：`go` 会**替换**整条路由栈，
          // 从工作台按返回时就没有可 pop 的页面，Android 直接把 App 杀掉
          // ——表现就是「一按返回就退出程序」。`push` 保留栈，返回才正常。
          final container = ProviderScope.containerOf(context, listen: false);
          container.read(selectedConnectionIdProvider.notifier).state = conn.id;
          container.read(connectionsControllerProvider.notifier).touch(conn.id);
          context.push('/workspace');
        },
      ),
    ),
    GoRoute(
      path: '/logs',
      builder: (context, state) => const LogsScreen(),
    ),
    GoRoute(
      path: '/workspace',
      builder: (context, state) => SessionsScreen(
        onOpenSession: (sessionId) => context.push('/workspace/$sessionId'),
      ),
    ),
    GoRoute(
      path: '/workspace/:sessionId',
      builder: (context, state) {
        final id = state.pathParameters['sessionId'] ?? '';
        final title = state.uri.queryParameters['title'];
        return SessionScreen(sessionId: id, title: title);
      },
    ),
  ],
);

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
        // ⚠️ 先保活连接级控制器，再做别的。
        //
        // 待答审批是阻塞状态，用户停在哪个界面都不该让它消失。
        // 放在这里（而不是某个 screen 里）是因为 `MaterialApp.builder`
        // 是整个 widget 树里唯一一定会被构建的位置。
        ProviderScope.containerOf(
          context,
          listen: false,
        ).read(connectionRootProvider);

        // 尊重系统的字号缩放，但**封顶**——工具界面里 2 倍字号会把
        // 每一行都撑成两行，转录会变得没法读。
        final media = MediaQuery.of(context);
        final clamped = media.textScaler.clamp(
          minScaleFactor: 0.9,
          maxScaleFactor: 1.3,
        );
        return MediaQuery(
          data: media.copyWith(textScaler: clamped),
          child: child ?? const SizedBox.shrink(),
        );
      },
    );
  }
}

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
  // 05 §5.2：冷启动**必须**落会话列表，不是连接页。
  // 用户 95% 的冷启动是想看 agent 在干什么，不是想连电脑。原实现把
  // `/connections` 当根，等于每次开 App 都先逼用户做一次「选机器」的
  // 例行动作，才能看到他真正要看的东西。
  initialLocation: '/workspace',
  routes: [
    // 连接管理降级为会话列表里的一个入口（顶栏机器名可点）。
    GoRoute(
      path: '/connections',
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
        final container = ProviderScope.containerOf(context, listen: false);
        container.read(connectionRootProvider);
        // 冷启动自动选中唯一那台电脑（05 §5.2）**不需要**在这里做任何事：
        // 它是 `soleConnectionIdProvider` 的纯派生，`selectedConnectionProvider`
        // 读得到。曾经的写法是在这里 `read(autoSelect…)` 去写状态 ——
        // 那会在 provider 初始化期间改另一个 provider，Riverpod 直接断言禁止。

        // 尊重系统的字号缩放。
        //
        // 上限 0.9–2.0（不是原来的 0.9–1.3）。04 裁决表明确：夹取是为了
        // 让布局不溢出，**不是为了压住无障碍**；把上限卡在 1.3 等于告诉
        // 「需要大字号的用户」这个 App 不为你服务。05 §9.5 第 4 条的验收项
        // 正是「正文文字缩放 2.0 时零溢出」。
        //
        // 等宽**数据层**（`ident` / `metric` / `code`）另在 `QxType` 各自
        // 封顶 1.0x：数字放大会撑破列对齐，那才是 02 保留封顶的理由。
        // 正文与状态角标跟着系统走。
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

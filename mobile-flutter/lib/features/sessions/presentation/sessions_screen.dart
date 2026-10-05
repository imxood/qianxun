import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../../app/theme/qx_test_ids.dart';
import '../../../app/theme/qx_theme.dart';
import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../shared/widgets/primitives.dart';
import '../../connections/application/connections_controller.dart';
import '../../workspace/presentation/pending_interaction_bar.dart';
import '../application/session_list_controller.dart';
import '../application/session_presentation.dart';
import 'link_status_bar.dart';
import 'session_tile.dart';

/// 会话列表（DSH 工作台首页）。
///
/// 这是 App 的**主界面**。05 §5.2：冷启动**必须**落在这里——用户 95% 的
/// 冷启动是想看 agent，不是想连电脑。只有一台电脑就直接进列表，多台才
/// 需要切换器，一台都没有才是配对引导。
///
/// 结构（05 §4.2）：分隔线列表 + 66dp 定高行 + 出血信号轨。
/// 05 §4.1 批评的现状是「一屏 5 张一模一样的圆角卡片」，本屏用
/// 分隔线而不是卡片，每一条省下的 12dp 都是一整行会话。
class SessionsScreen extends ConsumerWidget {
  const SessionsScreen({super.key, required this.onOpenSession});

  final void Function(String sessionId) onOpenSession;

  /// 列表的横向内边距。
  ///
  /// ⚠️ 横向**必须为 0**：信号轨要出血到屏幕左缘（05 §3.1），一格左内边距
  /// 就会让屏幕左边那条连续的线断开，签名元素退化成「卡片左边一个小色块」。
  /// 内容的缩进由行内的 [SessionTile] 负责。
  static const listPadding = EdgeInsets.only(
    top: QxSpace.xs,
    bottom: QxSpace.huge * 2,
  );

  /// 轨是否真的出血：横向内边距为 0。
  ///
  /// 单独暴露成 getter 是为了让测试能断**真实值**。写成
  /// `expect(true, isTrue)` 那种断言永远绿，等于没写。
  static bool get sessionsListHasNoLeftInset => listPadding.left == 0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final conn = ref.watch(selectedConnectionProvider);
    final link = conn == null ? null : ref.watch(dshConnectionProvider(conn));
    final state = ref.watch(sessionListControllerProvider);
    // 呈现态（含排序）。走 provider 而不是在这里现算：本地 waterfall
    // 一到，这条会话必须**立刻**置顶，而那需要一次声明式重建。
    final rows = ref.watch(sessionPresentationsProvider);

    return Scaffold(
      appBar: AppBar(
        titleSpacing: QxSpace.l,
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              conn?.name ?? '工作台',
              style: QxType.title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            if (conn != null)
              Text(
                conn.displayHost,
                style: QxType.ident.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                  fontSize: 11,
                ),
              ),
          ],
        ),
        actions: [
          IconButton(
            onPressed: () => ref.invalidate(sessionListControllerProvider),
            icon: const Icon(Icons.refresh, size: 20),
            tooltip: '刷新',
          ),
          // 05 §5.2：连接管理降级成「一个入口」。根路由已经是会话列表，
          // 所以它必须从这里进，否则就彻底没有去路了。
          IconButton(
            onPressed: () => context.push('/connections'),
            icon: const Icon(Icons.dns_outlined, size: 20),
            tooltip: '连接管理',
          ),
          const SizedBox(width: QxSpace.xs),
        ],
      ),
      // 列表本身的锚点：自动化脚本用它区分「列表页」与其它页。
      backgroundColor: theme.colorScheme.surface,
      body: Semantics(
        identifier: QxTestIds.sessionsList,
        container: true,
        explicitChildNodes: true,
        child: Column(
          children: [
            LinkStatusBar(link: link, onRetry: () => link?.retryNow()),
            Expanded(child: _body(context, ref, state, rows)),
            const PendingInteractionBar(),
          ],
        ),
      ),
      floatingActionButton: state is SessionListReady
          ? FloatingActionButton.extended(
              onPressed: () => _createSession(context, ref),
              backgroundColor: theme.qx.accent,
              foregroundColor: theme.qx.onAccent,
              icon: const Icon(Icons.add_comment_outlined, size: 20),
              label: Semantics(
                identifier: QxTestIds.newSessionButton,
                child: const Text('新会话'),
              ),
            )
          : null,
    );
  }

  Widget _body(
    BuildContext context,
    WidgetRef ref,
    SessionListState state,
    List<SessionPresentation> rows,
  ) {
    return switch (state) {
      SessionListLoading() => const Center(
          child: SizedBox(
            width: 22,
            height: 22,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      SessionListError(:final message, :final reason) =>
        _BlockedBody(reason: reason, message: message),
      SessionListReady(:final error) => rows.isEmpty
          ? const EmptyHero(
              title: '还没有会话',
              hint: '在这台电脑上开始一段对话，它会实时出现在这里。',
              icon: Icons.forum_outlined,
            )
          : RefreshIndicator(
              onRefresh: ref.read(sessionListControllerProvider.notifier).load,
              child: ListView.separated(
                // 轨要**出血到屏幕左缘**（05 §3.1），所以横向不留边距；
                // 内容缩进交给行内的 SignalSpine gutter。
                padding: listPadding,
                itemCount: rows.length + (error == null ? 0 : 1),
                separatorBuilder: (_, __) => const Divider(height: 1),
                itemBuilder: (context, index) {
                  if (error != null && index == 0) {
                    return InlineNotice(text: error, isError: false);
                  }
                  final item = rows[index - (error == null ? 0 : 1)];
                  return SessionTile(
                    item: item,
                    onTap: () => onOpenSession(item.session.sessionId),
                  );
                },
              ),
            ),
    };
  }

  Future<void> _createSession(BuildContext context, WidgetRef ref) async {
    final messenger = ScaffoldMessenger.of(context);
    final result = await ref.read(sessionListControllerProvider.notifier).create();
    result.map(
      (sessionId) {
        if (sessionId.isNotEmpty) {
          // 新建后直接进去，省一次点击。
          //
          // ⚠️ 这里**不能**再 pop。列表现在是根路由（05 §5.2），pop 出去
          // 就是退出 App；而 `onOpenSession` 自己会 push 会话页，pop 会把
          // 刚推上去的那一页弹掉，白跳一场。原先的 `maybePop()` 之所以看着
          // 合理，是因为那时列表是从连接页 push 上来的。
          onOpenSession(sessionId);
        }
        return sessionId;
      },
      (error) {
        messenger.showSnackBar(SnackBar(content: Text(error.message)));
        return '';
      },
    );
  }
}

/// 「出不去」的四种空态。
///
/// ⚠️ 四种各有各的**下一步**，配错了就是在骗用户：
///
/// | 原因               | 下一步 | 错配的后果                                |
/// |--------------------|--------|-------------------------------------------|
/// | `noMachines`       | 去配对 | 说「连不上」→ 反复点重试，永远出不去      |
/// | `needsRepair`      | 重新配对 | 同上                                     |
/// | `needsMachineChoice` | 去选  | 说「连不上」→ 没坏，重试无用              |
/// | `unreachable`      | 重试    | 说「去配对」→ 把还能用的配置清了          |
///
/// 第一条是**首次体验**，最不能搞砸。它曾经显示成「连不上这台电脑」——
/// 全新安装的用户一台机器都没有，被告知连不上。**这是靠模拟器跑出来
/// 才看见的**，静态测试与 widget 测试都碰不到。
class _BlockedBody extends StatelessWidget {
  const _BlockedBody({required this.reason, required this.message});

  final ListBlockReason reason;
  final String message;

  @override
  Widget build(BuildContext context) {
    final title = switch (reason) {
      ListBlockReason.noMachines => '还没有电脑',
      ListBlockReason.needsRepair => '需要重新配对',
      ListBlockReason.needsMachineChoice => '选一台电脑',
      ListBlockReason.unreachable => '连不上这台电脑',
    };
    final icon = switch (reason) {
      ListBlockReason.noMachines => Icons.devices_other,
      ListBlockReason.needsRepair => Icons.link_off,
      ListBlockReason.needsMachineChoice => Icons.dns_outlined,
      ListBlockReason.unreachable => Icons.cloud_off_outlined,
    };
    final action = switch (reason) {
      ListBlockReason.noMachines => ('去配对', Icons.add_link),
      ListBlockReason.needsRepair => ('去配对', Icons.link_off),
      ListBlockReason.needsMachineChoice => ('去选择', Icons.dns_outlined),
      ListBlockReason.unreachable => ('重试', Icons.refresh),
    };

    return EmptyHero(
      title: title,
      hint: message,
      icon: icon,
      action: OutlinedButton.icon(
        onPressed: () {
          if (reason == ListBlockReason.unreachable) {
            ProviderScope.containerOf(context, listen: false)
                .read(sessionListControllerProvider.notifier)
                .load();
          } else {
            context.push('/connections');
          }
        },
        icon: Icon(action.$2, size: 18),
        label: Text(action.$1),
      ),
    );
  }
}

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

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
      body: Column(
        children: [
          LinkStatusBar(link: link, onRetry: () => link?.retryNow()),
          Expanded(child: _body(context, ref, state, rows)),
          const PendingInteractionBar(),
        ],
      ),
      floatingActionButton: state is SessionListReady
          ? FloatingActionButton.extended(
              onPressed: () => _createSession(context, ref),
              icon: const Icon(Icons.add_comment_outlined, size: 20),
              label: const Text('新会话'),
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
      SessionListError(
        :final message,
        :final needsPairing,
        :final needsMachineChoice,
      ) =>
        EmptyHero(
          // ⚠️ 三种空态各有各的下一步，混起来就是在骗用户：
          //   需要重新配对 → 去重新扫码
          //   还没选机器   → 去**选**（重试没用，什么都没坏）
          //   连不上       → 去**重试**
          title: switch ((needsPairing, needsMachineChoice)) {
            (true, _) => '需要重新配对',
            (_, true) => '选一台电脑',
            _ => '连不上这台电脑',
          },
          hint: message,
          icon: needsPairing
              ? Icons.link_off
              : needsMachineChoice
                  ? Icons.dns_outlined
                  : Icons.cloud_off_outlined,
          action: (needsPairing || needsMachineChoice)
              ? OutlinedButton.icon(
                  onPressed: () => context.push('/connections'),
                  icon: const Icon(Icons.dns_outlined, size: 18),
                  label: Text(needsPairing ? '去配对' : '去选择'),
                )
              : OutlinedButton.icon(
                  onPressed: () => ref.read(sessionListControllerProvider.notifier).load(),
                  icon: const Icon(Icons.refresh, size: 18),
                  label: const Text('重试'),
                ),
        ),
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

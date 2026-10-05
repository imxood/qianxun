import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../data/models/content.dart';
import '../../../data/models/session.dart';
import '../../../shared/widgets/primitives.dart';
import '../../connections/application/connections_controller.dart';
import '../application/session_list_controller.dart';
import 'link_status_bar.dart';
import '../../workspace/presentation/pending_interaction_bar.dart';
import '../../../app/theme/qx_theme.dart';
import '../../../../app/theme/qx_semantic.dart';

/// 会话列表（DSH 工作台首页）。
///
/// 这是 App 的**主界面**。它取代了原来那个把 DSH 页面塞进 WebView 的
/// 做法：列表、会话详情、审批、提问全部是原生控件，因此可以真正适配
/// 手机的宽度、字号与手势，而不是把桌面页面缩小。
class SessionsScreen extends ConsumerWidget {
  const SessionsScreen({super.key, required this.onOpenSession});

  final void Function(String sessionId) onOpenSession;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final conn = ref.watch(selectedConnectionProvider);
    final link = conn == null ? null : ref.watch(dshConnectionProvider(conn));
    final state = ref.watch(sessionListControllerProvider);

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
          const SizedBox(width: QxSpace.xs),
        ],
      ),
      body: Column(
        children: [
          LinkStatusBar(link: link, onRetry: () => link?.retryNow()),
          if (state case SessionListReady(items: final items) when items.isNotEmpty)
            _SearchField(
              onChanged: ref.read(sessionListControllerProvider.notifier).search,
            ),
          Expanded(child: _body(context, ref, state)),
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

  Widget _body(BuildContext context, WidgetRef ref, SessionListState state) {
    return switch (state) {
      SessionListLoading() => const Center(
          child: SizedBox(
            width: 22,
            height: 22,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      SessionListError(:final message, :final needsPairing) => EmptyHero(
          title: needsPairing ? '需要重新配对' : '连不上这台电脑',
          hint: message,
          icon: needsPairing ? Icons.link_off : Icons.cloud_off_outlined,
          action: needsPairing
              ? null
              : OutlinedButton.icon(
                  onPressed: () => ref.read(sessionListControllerProvider.notifier).load(),
                  icon: const Icon(Icons.refresh, size: 18),
                  label: const Text('重试'),
                ),
        ),
      SessionListReady(:final items, :final error, :final query) => items.isEmpty
          ? EmptyHero(
              title: query.isEmpty ? '还没有会话' : '没有匹配的会话',
              hint: query.isEmpty ? '在这台电脑上开始一段对话，它会实时出现在这里。' : '换个关键词试试。',
              icon: Icons.forum_outlined,
            )
          : RefreshIndicator(
              onRefresh: ref.read(sessionListControllerProvider.notifier).load,
              child: ListView.separated(
                padding: const EdgeInsets.fromLTRB(
                  QxSpace.l,
                  QxSpace.xs,
                  QxSpace.l,
                  QxSpace.huge * 2,
                ),
                itemCount: items.length + (error == null ? 0 : 1),
                separatorBuilder: (_, __) => const SizedBox(height: QxSpace.s),
                itemBuilder: (context, index) {
                  if (error != null && index == 0) {
                    return InlineNotice(text: error, isError: false);
                  }
                  final item = items[index - (error == null ? 0 : 1)];
                  return SessionRow(
                    session: item,
                    onTap: () => onOpenSession(item.sessionId),
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
          Navigator.of(context).maybePop();
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

/// 会话搜索框。
class _SearchField extends StatefulWidget {
  const _SearchField({required this.onChanged});
  final ValueChanged<String> onChanged;

  @override
  State<_SearchField> createState() => _SearchFieldState();
}

class _SearchFieldState extends State<_SearchField> {
  final _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(QxSpace.l, 0, QxSpace.l, QxSpace.s),
      child: TextField(
        controller: _controller,
        style: QxType.body,
        onChanged: widget.onChanged,
        decoration: InputDecoration(
          hintText: '搜索对话内容',
          prefixIcon: const Icon(Icons.search, size: 20),
          suffixIcon: _controller.text.isEmpty
              ? null
              : IconButton(
                  icon: const Icon(Icons.close, size: 18),
                  onPressed: () {
                    _controller.clear();
                    widget.onChanged('');
                    setState(() {});
                  },
                ),
          isDense: true,
          contentPadding: const EdgeInsets.symmetric(vertical: QxSpace.m),
        ),
        onTap: () => setState(() {}),
      ),
    );
  }
}

/// 一行会话。
///
/// 行内信息的选择逻辑：**状态 > 时间 > 上下文**。
/// 状态用色环 + 文字，标题用一行截断，副行给 cwd / 轮次 / token。
/// 这些数字对「我的 agent 现在在干什么」是决定性的，比任何花哨的
/// 预览文本都有用。
class SessionRow extends StatelessWidget {
  const SessionRow({super.key, required this.session, required this.onTap});

  final SessionSummary session;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // 状态判定与取色**只能**来自 [QxSessionState]，不许在这里另写一套词表：
    // 早期版本这里手搓了「运行中 / 空闲 / 已断开」三选一，跟 05 §2 的五态词表
    // 各说各话，同一屏会出现两套词。Batch 3 重做本列表时这一整段会被
    // SessionTile 取代，但在那之前也不许再分叉。
    //
    // 目前只用得到三态：running / detached / done。等你与失败要等
    // `userQuestions.active` 与 waterfall 接进数据源（Batch 3）。
    final state = switch (session) {
      _ when session.running => QxSessionState.running,
      _ when session.isDetached => QxSessionState.detached,
      _ => QxSessionState.done,
    };
    final accent = state.colorOf(theme.qx);
    final stateLabel = state.label;

    final stats = session.stats;
    final usage = session.tokenUsage;
    final prompt = session.lastPrompt;

    return Semantics(
      button: true,
      label: '${session.displayTitle}，$stateLabel',
      child: Material(
        color: theme.colorScheme.surface,
        borderRadius: QxRadius.cardAll,
        child: InkWell(
          onTap: () {
            HapticFeedback.selectionClick();
            onTap();
          },
          borderRadius: QxRadius.cardAll,
          child: Container(
            decoration: BoxDecoration(
              borderRadius: QxRadius.cardAll,
              border: Border.all(
                color:
                    session.running ? accent.withValues(alpha: 0.42) : theme.qx.hairline,
              ),
            ),
            padding: const EdgeInsets.all(QxSpace.l),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Padding(
                      padding: const EdgeInsets.only(top: 5),
                      child: StateDot(state: state, animate: session.running),
                    ),
                    const SizedBox(width: QxSpace.m),
                    Expanded(
                      child: Text(
                        session.displayTitle,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: QxType.body.copyWith(
                          color: theme.colorScheme.onSurface,
                          height: 1.3,
                        ),
                      ),
                    ),
                    if (session.isSubagent)
                      Padding(
                        padding: const EdgeInsets.only(left: QxSpace.s),
                        child: Icon(
                          Icons.account_tree_outlined,
                          size: 14,
                          color: theme.colorScheme.onSurfaceVariant,
                        ),
                      ),
                  ],
                ),
                // 现成预览：DSH 自己给的 turnOutline，比客户端猜的准。
                if (prompt != null) ...[
                  const SizedBox(height: QxSpace.s),
                  Padding(
                    padding: const EdgeInsets.only(left: QxSpace.l + QxSpace.xs),
                    child: Text(
                      prompt,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: QxType.caption.copyWith(
                        color: theme.colorScheme.onSurface.withValues(alpha: 0.72),
                        height: 1.45,
                      ),
                    ),
                  ),
                ],
                const SizedBox(height: QxSpace.s),
                Padding(
                  padding: const EdgeInsets.only(left: QxSpace.l + QxSpace.xs),
                  child: _MetaLine(
                    session: session,
                    state: state,
                    stats: stats,
                    usage: usage,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _MetaLine extends StatelessWidget {
  const _MetaLine({
    required this.session,
    required this.state,
    required this.stats,
    required this.usage,
  });

  final SessionSummary session;

  /// 状态。**词与色都从它派生**（`state.label` / `state.colorOf`），
  /// 传两个平行字段进来就等于允许两者对不上。
  final QxSessionState state;
  final SessionStats? stats;
  final TokenUsage? usage;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final dim = theme.colorScheme.onSurfaceVariant;
    final parts = <(String, Color)>[
      // 状态**带颜色**地放在最前面：一眼扫过去就知道哪些在跑。
      // 取色走 [QxSessionState.colorOf]，不自己重推三元 —— 同一份映射
      // 写两遍，等于给「新增第 4/5 态」留一个必炸的漏改点。
      (state.label, state.colorOf(context.qx)),
    ];

    if (session.metaCwd != null) parts.add((session.metaCwd!, dim));
    if (stats != null && stats!.turns > 0) parts.add(('${stats!.turns} 轮', dim));
    final ctx = session.contextTokens;
    if (ctx != null && ctx > 0) parts.add(('${formatTokens(ctx)} ctx', dim));
    if (usage != null && usage!.outputTokens > 0) {
      parts.add(('↑${formatTokens(usage!.outputTokens)}', dim));
    }
    parts.add((relTime(session.updatedAt), dim.withValues(alpha: 0.7)));

    return Wrap(
      spacing: 6,
      runSpacing: 2,
      children: [
        for (var i = 0; i < parts.length; i++) ...[
          if (i > 0) Text('·', style: QxType.caption.copyWith(color: dim)),
          Text(
            parts[i].$1,
            style: QxType.caption.copyWith(
              color: parts[i].$2,
              fontWeight: i == 0 ? FontWeight.w700 : FontWeight.w400,
            ),
          ),
        ],
      ],
    );
  }
}

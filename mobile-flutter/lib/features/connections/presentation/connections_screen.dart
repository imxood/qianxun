import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../data/dsh/gateway_client.dart';
import '../../../data/store/connection_store.dart';
import '../../../shared/widgets/primitives.dart';
import '../application/connections_controller.dart';
import '../../../app/theme/qx_theme.dart';
import '../../../../app/theme/qx_semantic.dart';

/// 连接列表。
///
/// 这是「我的电脑」货架。设计目标：用户在地铁上掏出来，**1 秒内**知道
/// 哪台电脑现在能用。因此每行都带一个状态点 + 状态**文字** + 主机名，
/// 而不是让人去数色块。
class ConnectionsScreen extends ConsumerWidget {
  const ConnectionsScreen({super.key, required this.onOpen});

  final void Function(Connection conn) onOpen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(connectionsControllerProvider);
    final controller = ref.read(connectionsControllerProvider.notifier);

    return Scaffold(
      body: SafeArea(
        bottom: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Header(
              onAdd: () => showAddConnectionSheet(context, ref),
            ),
            Expanded(
              child: switch (state) {
                ConnectionsLoading() => const Center(
                    child: SizedBox(
                      width: 22,
                      height: 22,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  ),
                ConnectionsReady(:final list, :final notice, :final noticeIsError) => list.isEmpty
                    ? EmptyHero(
                        title: '还没有连接任何电脑',
                        hint: '在电脑端打开「设置 → 远程访问」，\n'
                            '把配对二维码扫进来就能远程操控这台机器。',
                        icon: Icons.qr_code_scanner_outlined,
                        action: FilledButton.icon(
                          onPressed: () => showAddConnectionSheet(context, ref),
                          icon: const Icon(Icons.add, size: 18),
                          label: const Text('添加电脑'),
                        ),
                      )
                    : RefreshIndicator(
                        onRefresh: controller.refreshStatuses,
                        child: ListView(
                          padding: const EdgeInsets.fromLTRB(
                            QxSpace.l,
                            0,
                            QxSpace.l,
                            QxSpace.huge,
                          ),
                          children: [
                            const SizedBox(height: QxSpace.xs),
                            _ConnectionGroup(
                              connections: list,
                              onOpen: (c) {
                                controller.touch(c.id);
                                onOpen(c);
                              },
                            ),
                            if (notice != null) ...[
                              const SizedBox(height: QxSpace.l),
                              InlineNotice(text: notice, isError: noticeIsError),
                            ],
                            const SizedBox(height: QxSpace.xl),
                            Center(
                              child: Text(
                                '网关是明文 HTTP，密文性由 EasyTier 隧道提供。',
                                style: QxType.micro.copyWith(
                                  color: Theme.of(context)
                                      .colorScheme
                                      .onSurfaceVariant
                                      .withValues(alpha: 0.6),
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
              },
            ),
          ],
        ),
      ),
    );
  }
}

class _Header extends StatelessWidget {
  const _Header({required this.onAdd});
  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        QxSpace.l,
        QxSpace.l,
        QxSpace.l,
        QxSpace.m,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text('连接', style: QxType.display),
                const SizedBox(height: QxSpace.xxs),
                Text(
                  '远程操控你自己的机器',
                  style: QxType.caption.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
          _IconAction(
            icon: Icons.receipt_long_outlined,
            tooltip: '运行日志',
            onPressed: () {
              HapticFeedback.selectionClick();
              context.push('/logs');
            },
          ),
          const SizedBox(width: QxSpace.s),
          _IconAction(
            icon: Icons.add,
            tooltip: '添加连接',
            onPressed: onAdd,
          ),
        ],
      ),
    );
  }
}

/// 圆形图标按钮。
class _IconAction extends StatelessWidget {
  const _IconAction({
    required this.icon,
    required this.onPressed,
    this.tooltip,
  });

  final IconData icon;
  final VoidCallback onPressed;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final button = Material(
      color: context.qx.surfaceRaised,
      borderRadius: BorderRadius.circular(22),
      child: InkWell(
        onTap: () {
          HapticFeedback.selectionClick();
          onPressed();
        },
        customBorder: const CircleBorder(),
        child: SizedBox(
          width: 44,
          height: 44,
          child: Icon(icon, size: 21, color: theme.colorScheme.onSurface),
        ),
      ),
    );
    return tooltip == null ? button : Tooltip(message: tooltip!, child: button);
  }
}

/// 连接分组卡片。
///
/// 用一个共享容器 + 分隔线，而不是每行一张卡：这是一个**列表**，不是
/// 一组互相独立的物体。逐行加卡会让长列表变成搓衣板。
class _ConnectionGroup extends StatelessWidget {
  const _ConnectionGroup({required this.connections, required this.onOpen});

  final List<Connection> connections;
  final void Function(Connection conn) onOpen;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return DecoratedBox(
      decoration: theme.groupBox,
      child: ClipRRect(
        borderRadius: QxRadius.cardAll,
        child: Column(
          children: [
            for (var i = 0; i < connections.length; i++) ...[
              if (i > 0) Divider(height: 1, color: context.qx.hairline),
              _ConnectionRow(
                conn: connections[i],
                onOpen: () => onOpen(connections[i]),
                onMenu: () => _showRowMenu(context, connections[i]),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _ConnectionRow extends ConsumerWidget {
  const _ConnectionRow({
    required this.conn,
    required this.onOpen,
    required this.onMenu,
  });

  final Connection conn;
  final VoidCallback onOpen;
  final VoidCallback onMenu;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final probe = ref.watch(connectionsControllerProvider.notifier).probeOf(conn.id);

    // 连接态与会话态是**两套词表**：连接只分「通不通」，不管 agent 在干嘛。
    // 所以这里用「在线 / DSH 未运行 / 离线」而不是会话的五态。
    final (connColor, statusText, pulsing) = switch (probe) {
      ProbeOk(:final info) => (
          info.dshReady ? context.qx.running : context.qx.waiting,
          info.dshReady ? '在线' : 'DSH 未启动',
          info.dshReady,
        ),
      ProbeUnreachable() => (context.qx.detached, '离线', false),
      ProbeUpstreamDown() => (context.qx.waiting, 'DSH 未运行', false),
      ProbeUnauthorized() => (context.qx.failed, '凭据失效', false),
      ProbeBadUrl() => (context.qx.failed, '链接无效', false),
      null => (theme.colorScheme.onSurfaceVariant, '检测中…', false),
    };

    return Semantics(
      button: true,
      label: '${conn.name}，$statusText，${conn.displayHost}',
      child: InkWell(
        onTap: onOpen,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(
            QxSpace.l,
            QxSpace.m,
            QxSpace.s,
            QxSpace.m,
          ),
          child: Row(
            children: [
              StateDot.colored(color: connColor, animate: pulsing),
              const SizedBox(width: QxSpace.m),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      conn.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: QxType.body.copyWith(color: theme.colorScheme.onSurface),
                    ),
                    const SizedBox(height: 3),
                    Row(
                      children: [
                        Flexible(
                          child: Text(
                            conn.displayHost,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: QxType.ident.copyWith(
                              color: theme.colorScheme.onSurfaceVariant,
                            ),
                          ),
                        ),
                        Text(
                          '  ·  ',
                          style: QxType.caption.copyWith(
                            color: theme.colorScheme.onSurfaceVariant.withValues(alpha: 0.5),
                          ),
                        ),
                        // 状态用**文字**表达，颜色只是辅助——色觉障碍用户
                        // 必须能读到同样的信息。
                        Text(
                          statusText,
                          style: QxType.caption.copyWith(color: connColor),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
              IconButton(
                onPressed: onMenu,
                icon: const Icon(Icons.more_horiz, size: 20),
                color: theme.colorScheme.onSurfaceVariant,
                tooltip: '更多操作',
                visualDensity: VisualDensity.compact,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 行内操作菜单。重命名 / 更新链接 / 删除。
Future<void> _showRowMenu(
  BuildContext context,
  Connection conn,
) {
  final controller = ProviderScope.containerOf(context, listen: false)
      .read(connectionsControllerProvider.notifier);
  return showModalBottomSheet<void>(
    context: context,
    builder: (sheetContext) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              QxSpace.xl,
              0,
              QxSpace.xl,
              QxSpace.m,
            ),
            child: Column(
              children: [
                Text(
                  conn.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: QxType.body.copyWith(
                    color: Theme.of(sheetContext).colorScheme.onSurface,
                  ),
                ),
                const SizedBox(height: 2),
                Text(
                  conn.displayHost,
                  style: QxType.ident.copyWith(
                    color: Theme.of(sheetContext).colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
          const Divider(height: 1),
          _SheetAction(
            label: '重命名',
            icon: Icons.drive_file_rename_outline,
            onTap: () {
              Navigator.pop(sheetContext);
              _promptRename(context, controller, conn);
            },
          ),
          _SheetAction(
            label: '更新配对链接',
            icon: Icons.link,
            onTap: () {
              Navigator.pop(sheetContext);
              _promptUpdateUrl(context, controller, conn);
            },
          ),
          _SheetAction(
            label: '删除连接',
            icon: Icons.delete_outline,
            destructive: true,
            onTap: () {
              Navigator.pop(sheetContext);
              controller.remove(conn.id);
            },
          ),
          const SizedBox(height: QxSpace.s),
        ],
      ),
    ),
  );
}

class _SheetAction extends StatelessWidget {
  const _SheetAction({
    required this.label,
    required this.icon,
    required this.onTap,
    this.destructive = false,
  });

  final String label;
  final IconData icon;
  final VoidCallback onTap;
  final bool destructive;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final color = destructive ? theme.colorScheme.error : theme.colorScheme.onSurface;
    return ListTile(
      onTap: () {
        HapticFeedback.selectionClick();
        onTap();
      },
      leading: Icon(icon, size: 20, color: color),
      title: Text(label, style: QxType.body.copyWith(color: color)),
      contentPadding: const EdgeInsets.symmetric(horizontal: QxSpace.xl),
      shape: const RoundedRectangleBorder(),
    );
  }
}

/// 添加连接的底部弹层。
Future<void> showAddConnectionSheet(BuildContext context, WidgetRef ref) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    builder: (_) => const _AddConnectionSheet(),
  );
}

class _AddConnectionSheet extends ConsumerStatefulWidget {
  const _AddConnectionSheet();

  @override
  ConsumerState<_AddConnectionSheet> createState() => _AddConnectionSheetState();
}

class _AddConnectionSheetState extends ConsumerState<_AddConnectionSheet> {
  final _url = TextEditingController();
  final _name = TextEditingController();
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _url.dispose();
    _name.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    final result = await ref
        .read(connectionsControllerProvider.notifier)
        .add(_url.text, manualName: _name.text);
    if (!mounted) return;
    setState(() {
      _busy = false;
      if (result.isOk) {
        Navigator.pop(context);
      } else {
        _error = result.error;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final inset = MediaQuery.of(context).viewInsets.bottom;

    return Padding(
      padding: EdgeInsets.fromLTRB(
        QxSpace.l,
        0,
        QxSpace.l,
        QxSpace.l + inset,
      ),
      child: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            const Text('添加电脑', style: QxType.title),
            const SizedBox(height: QxSpace.xs),
            Text(
              '在电脑端「设置 → 远程访问」里复制配对链接，或直接扫那个二维码。',
              style: QxType.caption.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
            const SizedBox(height: QxSpace.xl),
            TextField(
              controller: _url,
              autofocus: true,
              minLines: 1,
              maxLines: 3,
              keyboardType: TextInputType.url,
              autocorrect: false,
              enableSuggestions: false,
              style: QxType.code,
              decoration: const InputDecoration(
                hintText: 'http://192.168.1.9:23090/qx-gate?code=…',
                prefixIcon: Icon(Icons.link, size: 20),
              ),
              onChanged: (_) {
                if (_error != null) setState(() => _error = null);
              },
            ),
            const SizedBox(height: QxSpace.m),
            TextField(
              controller: _name,
              style: QxType.body,
              textInputAction: TextInputAction.done,
              decoration: const InputDecoration(
                hintText: '名称（留空则自动用电脑名）',
                prefixIcon: Icon(Icons.badge_outlined, size: 20),
              ),
              onSubmitted: (_) => _submit(),
            ),
            if (_error != null) ...[
              const SizedBox(height: QxSpace.m),
              InlineNotice(text: _error!, isError: true),
            ],
            const SizedBox(height: QxSpace.xl),
            FilledButton(
              onPressed: _busy ? null : _submit,
              child: _busy
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Text('连接'),
            ),
            const SizedBox(height: QxSpace.s),
          ],
        ),
      ),
    );
  }
}

Future<void> _promptRename(
  BuildContext context,
  ConnectionsController connections,
  Connection conn,
) {
  final field = TextEditingController(text: conn.name);
  return showDialog<void>(
    context: context,
    builder: (dialogContext) => AlertDialog(
      title: const Text('重命名'),
      content: TextField(
        controller: field,
        autofocus: true,
        maxLength: 30,
        style: QxType.body,
        decoration: const InputDecoration(hintText: '名称'),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(dialogContext),
          child: const Text('取消'),
        ),
        TextButton(
          onPressed: () {
            connections.rename(conn.id, field.text);
            Navigator.pop(dialogContext);
          },
          child: const Text('保存'),
        ),
      ],
    ),
  );
}

Future<void> _promptUpdateUrl(
  BuildContext context,
  ConnectionsController connections,
  Connection conn,
) {
  final field = TextEditingController(text: conn.url);
  String? error;
  return showDialog<void>(
    context: context,
    builder: (dialogContext) => StatefulBuilder(
      builder: (ctx, setState) => AlertDialog(
        title: const Text('更新配对链接'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              controller: field,
              autofocus: true,
              minLines: 2,
              maxLines: 4,
              keyboardType: TextInputType.url,
              autocorrect: false,
              style: QxType.code,
              decoration: const InputDecoration(hintText: 'http://…/qx-gate?code=…'),
            ),
            if (error != null) ...[
              const SizedBox(height: QxSpace.m),
              InlineNotice(text: error!, isError: true),
            ],
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(dialogContext),
            child: const Text('取消'),
          ),
          TextButton(
            onPressed: () {
              final result = connections.updateUrl(conn.id, field.text);
              if (result.isOk) {
                Navigator.pop(dialogContext);
              } else {
                setState(() => error = result.error);
              }
            },
            child: const Text('保存'),
          ),
        ],
      ),
    ),
  );
}

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:share_plus/share_plus.dart';

import '../../../app/theme/qx_type.dart';
import '../../../core/qx_log.dart';
import '../../../../app/theme/qx_semantic.dart';
import '../../../app/theme/qx_tokens.dart';

/// 日志页。
///
/// 存在的理由：这个客户端最常见的故障表现是**转圈**或**显示离线**——
/// 屏幕上没有任何信息能说明卡在哪一步。传输层的问题（cookie 没带上、
/// WS 升级被拒、mux 帧不认识、超时）恰恰是「看界面看不出来、只能看日志」
/// 那一类。所以这个页面不是给用户「看日志玩」的，是**唯一的自查手段**。
///
/// 三个动作：筛选级别、清空、分享（导出成临时 txt 走系统分享）。
class LogsScreen extends ConsumerStatefulWidget {
  const LogsScreen({super.key});

  @override
  ConsumerState<LogsScreen> createState() => _LogsScreenState();
}

class _LogsScreenState extends ConsumerState<LogsScreen> {
  /// null = 全部。
  QxLevel? _minLevel;
  final bool _autoScroll = true;
  final _scroll = ScrollController();
  String _query = '';

  @override
  void initState() {
    super.initState();
    // 新条目到达时贴底——除非用户正在往上翻找东西。
    qxlog.stream.listen((_) {
      if (!mounted || !_autoScroll) return;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (_scroll.hasClients) {
          _scroll.jumpTo(_scroll.position.maxScrollExtent);
        }
      });
    });
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final all = qxlog.entries.reversed.toList(growable: false);
    final q = _query.trim().toLowerCase();
    final rows = all.where((e) {
      if (_minLevel != null && e.level.severity < _minLevel!.severity) return false;
      if (q.isEmpty) return true;
      return e.message.toLowerCase().contains(q) ||
          e.tag.toLowerCase().contains(q) ||
          e.level.label.toLowerCase().contains(q);
    }).toList(growable: false);

    return Scaffold(
      appBar: AppBar(
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text('运行日志', style: QxType.title),
            Text(
              '${all.length} 条${rows.length == all.length ? '' : ' · 筛出 ${rows.length}'}',
              style: QxType.ident.copyWith(
                fontSize: 11,
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ],
        ),
        actions: [
          IconButton(
            tooltip: '复制到剪贴板',
            icon: const Icon(Icons.copy_all_outlined, size: 20),
            onPressed: rows.isEmpty ? null : () => _copy(rows),
          ),
          IconButton(
            tooltip: '分享日志文件',
            icon: const Icon(Icons.ios_share, size: 20),
            onPressed: rows.isEmpty ? null : () => _share(rows.length),
          ),
          PopupMenuButton<String>(
            tooltip: '更多',
            icon: const Icon(Icons.more_horiz, size: 20),
            onSelected: _onMenu,
            itemBuilder: (_) => [
              const PopupMenuItem(
                value: 'clear',
                child: ListTile(
                  dense: true,
                  contentPadding: EdgeInsets.zero,
                  leading: Icon(Icons.delete_outline, size: 18),
                  title: Text('清空日志'),
                ),
              ),
            ],
          ),
        ],
      ),
      body: Column(
        children: [
          _FilterBar(
            minLevel: _minLevel,
            onLevel: (l) => setState(() => _minLevel = l),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(
              QxSpace.l,
              QxSpace.xs,
              QxSpace.l,
              QxSpace.s,
            ),
            child: TextField(
              style: QxType.code.copyWith(fontSize: 12),
              onChanged: (v) => setState(() => _query = v),
              decoration: const InputDecoration(
                hintText: '过滤（关键字 / 标签 / 级别）',
                isDense: true,
                prefixIcon: Icon(Icons.search, size: 18),
              ),
            ),
          ),
          Expanded(
            child: rows.isEmpty
                ? const _EmptyLog()
                : Scrollbar(
                    controller: _scroll,
                    child: ListView.separated(
                      controller: _scroll,
                      padding: const EdgeInsets.fromLTRB(
                        QxSpace.l,
                        0,
                        QxSpace.l,
                        QxSpace.huge,
                      ),
                      // 行高固定，builder + 默认 cacheExtent 保证长日志滚动流畅。
                      itemCount: rows.length,
                      separatorBuilder: (_, __) => const SizedBox(height: 1),
                      itemBuilder: (context, i) => _LogRow(entry: rows[i]),
                    ),
                  ),
          ),
        ],
      ),
    );
  }

  void _onMenu(String value) {
    if (value != 'clear') return;
    showDialog<void>(
      context: context,
      builder: (d) => AlertDialog(
        title: const Text('清空日志？'),
        content: const Text('内存与磁盘上的日志都会被删除，且不可恢复。'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(d),
            child: const Text('取消'),
          ),
          TextButton(
            onPressed: () {
              qxlog.clear();
              Navigator.pop(d);
              setState(() {});
            },
            child: const Text('清空'),
          ),
        ],
      ),
    );
  }

  Future<void> _share(int count) async {
    final messenger = ScaffoldMessenger.of(context);
    try {
      final file = await qxlog.exportTxt();
      // ⚠️ **正文和文件必须都带**。
      //
      // 只给文件：分享到「聊天 / 笔记 / 邮件正文」这类只收文本的目标时，
      // 对方拿到的是空的——曾经就因此把一句摘要当成日志发出去。
      // 只给正文：超过 Binder 事务上限（约 1MB）会直接崩。
      //
      // 所以两边都给，正文按上限截断，完整内容在附件里。
      final text = qxlog.render(maxEntries: 400);
      await Share.shareXFiles(
        [XFile(file.path, mimeType: 'text/plain')],
        subject: '千寻移动端运行日志',
        text: text,
      );
      qxlog.info('logs', '已导出并分享：${file.path}（$count 条，正文 ${text.length} 字符）');
    } catch (e, st) {
      qxlog.error('logs', '分享失败', e, st);
      messenger.showSnackBar(SnackBar(content: Text('导出失败：$e')));
    }
  }

  Future<void> _copy(List<QxLogEntry> rows) async {
    final text = rows.map((e) => e.format()).join('\n');
    await Clipboard.setData(ClipboardData(text: text));
    if (mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('已复制 ${rows.length} 条到剪贴板')));
    }
  }
}

/// 级别筛选。
class _FilterBar extends StatelessWidget {
  const _FilterBar({required this.minLevel, required this.onLevel});

  final QxLevel? minLevel;
  final ValueChanged<QxLevel?> onLevel;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 44,
      child: ListView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: QxSpace.l),
        children: [
          _Chip(
            label: '全部',
            selected: minLevel == null,
            onTap: () => onLevel(null),
          ),
          for (final l in QxLevel.values)
            _Chip(
              label: l.label,
              color: _levelColor(context, l),
              selected: minLevel == l,
              onTap: () => onLevel(minLevel == l ? null : l),
            ),
        ],
      ),
    );
  }
}

class _Chip extends StatelessWidget {
  const _Chip({
    required this.label,
    required this.selected,
    required this.onTap,
    this.color,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final c = color ?? theme.colorScheme.primary;
    return Padding(
      padding: const EdgeInsets.only(right: QxSpace.s, top: QxSpace.xs),
      child: Material(
        color: selected ? c.withValues(alpha: 0.16) : Colors.transparent,
        borderRadius: BorderRadius.circular(6),
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(6),
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: QxSpace.m),
            alignment: Alignment.center,
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(6),
              border: Border.all(
                color: selected ? c.withValues(alpha: 0.5) : context.qx.hairline,
              ),
            ),
            child: Text(
              label,
              style: QxType.overline.copyWith(
                color: selected ? c : theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _LogRow extends StatelessWidget {
  const _LogRow({required this.entry});
  final QxLogEntry entry;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final c = _levelColor(context, entry.level);
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 3, horizontal: QxSpace.s),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Container(
              width: 3,
              height: 10,
              color: c,
            ),
          ),
          const SizedBox(width: QxSpace.s),
          Text(
            _hhmm(entry.at),
            style: QxType.ident.copyWith(
              fontSize: 10.5,
              color: theme.colorScheme.onSurfaceVariant.withValues(alpha: 0.6),
            ),
          ),
          const SizedBox(width: QxSpace.s),
          SizedBox(
            width: 46,
            child: Text(
              entry.level.label,
              style: QxType.overline.copyWith(color: c, fontSize: 9.5),
            ),
          ),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // 用 `Text.rich` 而不是裸 `RichText`：前者是 `Text` 的一种
                // 构造，`textSpan` 字段有值，所以 finders 与无障碍工具都能
                // 读到内容；裸 `RichText` 的内容对它们是隐形的。
                Text.rich(
                  TextSpan(
                    children: [
                      TextSpan(
                        text: '${entry.tag}  ',
                        style: QxType.ident.copyWith(
                          fontSize: 11,
                          color: theme.colorScheme.primary.withValues(alpha: 0.75),
                        ),
                      ),
                      TextSpan(
                        text: entry.message,
                        style: QxType.code.copyWith(
                          fontSize: 11.5,
                          color: theme.colorScheme.onSurface.withValues(alpha: 0.9),
                        ),
                      ),
                    ],
                  ),
                ),
                if (entry.error != null)
                  Padding(
                    padding: const EdgeInsets.only(top: 2, left: QxSpace.s),
                    child: Text(
                      entry.error!,
                      maxLines: 6,
                      overflow: TextOverflow.ellipsis,
                      style: QxType.code.copyWith(
                        fontSize: 10.5,
                        color: context.qx.failed,
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  static String _hhmm(DateTime t) =>
      '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}:'
      '${t.second.toString().padLeft(2, '0')}';
}

class _EmptyLog extends StatelessWidget {
  const _EmptyLog();

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(QxSpace.xxxl),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.receipt_long_outlined,
              size: 28,
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
            const SizedBox(height: QxSpace.m),
            Text(
              '还没有日志',
              style: QxType.body.copyWith(
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: QxSpace.xs),
            Text(
              '级别筛选为 INFO 及以上。\n连接出问题时回到这里点分享，把文件发出去。',
              textAlign: TextAlign.center,
              style: QxType.caption.copyWith(
                color: Theme.of(context).colorScheme.onSurfaceVariant.withValues(alpha: 0.7),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 日志级别 → 颜色。
///
/// 规格：`02-visual-system.md` §7 的 `LogRow` 行——
/// 非颜色通道是**级别字母**（W/I/E），颜色规则是「**仅 warn 及以上用状态色**」。
///
/// 所以这是一条**严重度阶梯**，不是第二套会话词表：
/// trace/debug/info 全部走中性文字三级，warn 才升到琥珀，error 升到红。
/// 借 `waiting` / `failed` 的色相是有意的——严重度用琥珀与红是通用惯例，
/// 且日志与会话列表是两个屏幕，不会同屏误读。真正要避免的是**把绿色给 INFO**：
/// 绿色在本 App 里专属「有一个 agent 正在跑」，给一条普通日志会让整列
/// 读起来像一堆会话。
///
/// ⚠️ `info` 原来取 `q.onAccent`。`onAccent` 是**配对色**，只在 accent 底上成立，
/// 单独用只有 1.14:1 —— 级别竖条和选中态筛选 chip 的文字都等于隐形，
/// 用户看不出当前选了哪个级别筛选。这类 bug 对比度脚本抓不到，
/// 因为脚本只按「onAccent vs accent」验（6.60 / 5.13，通过）。
/// 教训：配对色不能当独立色用。已加进 `check_contrast.py` 的闸门。
Color _levelColor(BuildContext context, QxLevel l) {
  final q = context.qx;
  return switch (l) {
    QxLevel.trace => q.textTertiary,
    QxLevel.debug => q.textSecondary,
    QxLevel.info => q.textPrimary,
    QxLevel.warn => q.waiting,
    QxLevel.error => q.failed,
  };
}

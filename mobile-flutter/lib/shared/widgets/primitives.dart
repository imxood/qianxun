import 'package:flutter/material.dart';

import '../../app/theme/qx_tokens.dart';
import '../../app/theme/qx_type.dart';

/// 状态点。
///
/// 这是整个 App 信息密度最高的 8 像素。它必须能在一米外被认出，并且
/// **永远 accompanied by 文字**——纯色点在色觉障碍用户眼里毫无信息，
/// 所以本 App 的每个状态点旁边都有一行状态文字，不靠颜色单独表意。
class StateDot extends StatelessWidget {
  const StateDot({
    super.key,
    required this.color,
    this.size = 8,
    this.pulsing = false,
  });

  final Color color;
  final double size;

  /// 运行态的呼吸动画。
  final bool pulsing;

  @override
  Widget build(BuildContext context) {
    if (!pulsing) {
      return Container(
        width: size,
        height: size,
        decoration: BoxDecoration(color: color, shape: BoxShape.circle),
      );
    }
    return _PulsingDot(color: color, size: size);
  }
}

/// 呼吸点。用一个极克制的扩张环表达「活着」，而不是转圈加载器——
/// 转圈表达「在等」，呼吸表达「在跑」，语义不同。
class _PulsingDot extends StatefulWidget {
  const _PulsingDot({required this.color, required this.size});
  final Color color;
  final double size;

  @override
  State<_PulsingDot> createState() => _PulsingDotState();
}

class _PulsingDotState extends State<_PulsingDot> with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1600),
  )..repeat();

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      child: SizedBox(
        width: widget.size * 2.4,
        height: widget.size * 2.4,
        child: AnimatedBuilder(
          animation: _c,
          builder: (context, _) {
            final t = _c.value;
            // 一个周期：0→1 扩张淡出，然后重来。
            final halo = (1 - t).clamp(0.0, 1.0);
            return Stack(
              alignment: Alignment.center,
              children: [
                Container(
                  width: widget.size * (1 + t * 0.9),
                  height: widget.size * (1 + t * 0.9),
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: widget.color.withValues(alpha: 0.28 * halo),
                  ),
                ),
                Container(
                  width: widget.size,
                  height: widget.size,
                  decoration: BoxDecoration(
                    color: widget.color,
                    shape: BoxShape.circle,
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}

/// 等宽小标签。用于状态、阶段、工具名。
class MonoBadge extends StatelessWidget {
  const MonoBadge({
    super.key,
    required this.label,
    required this.color,
    this.filled = false,
  });

  final String label;
  final Color color;

  /// 实心底（高强调）还是描边（次强调）。
  final bool filled;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      decoration: BoxDecoration(
        color: filled ? color.withValues(alpha: 0.16) : Colors.transparent,
        border: Border.all(color: color.withValues(alpha: 0.42)),
        borderRadius: BorderRadius.circular(4),
      ),
      child: Text(label, style: QxType.badge(color)),
    );
  }
}

/// 分组标题。
class SectionHeader extends StatelessWidget {
  const SectionHeader(this.label, {super.key, this.trailing});

  final String label;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        QxSpace.xs,
        QxSpace.xl,
        QxSpace.xs,
        QxSpace.s,
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              label.toUpperCase(),
              style: QxType.overlineIn(theme.colorScheme.onSurfaceVariant),
            ),
          ),
          if (trailing != null) trailing!,
        ],
      ),
    );
  }
}

/// 空状态。
///
/// 空状态是唯一允许「说一句人话」的地方——它是用户和功能之间的第一次
/// 接触，应该解释**为什么空**以及**下一步做什么**，而不是画个灰图标。
class EmptyHero extends StatelessWidget {
  const EmptyHero({
    super.key,
    required this.title,
    required this.hint,
    this.icon = Icons.dns_outlined,
    this.action,
  });

  final String title;
  final String hint;
  final IconData icon;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(QxSpace.xxxl),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 56,
              height: 56,
              decoration: theme.sunkenBox,
              child: Icon(icon, size: 24, color: theme.colorScheme.onSurfaceVariant),
            ),
            const SizedBox(height: QxSpace.l),
            Text(title,
                style: QxType.body.copyWith(
                  color: theme.colorScheme.onSurface,
                )),
            const SizedBox(height: QxSpace.s),
            Text(
              hint,
              textAlign: TextAlign.center,
              style: QxType.caption.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
            if (action != null) ...[
              const SizedBox(height: QxSpace.xl),
              action!,
            ],
          ],
        ),
      ),
    );
  }
}

/// 一行「等宽值 + 说明」。用于连接详情这类技术信息展示。
class MetaRow extends StatelessWidget {
  const MetaRow({super.key, required this.label, required this.value, this.monospace = true});

  final String label;
  final String value;
  final bool monospace;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: QxSpace.s),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 76,
            child: Text(
              label,
              style: QxType.caption.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
          const SizedBox(width: QxSpace.m),
          Expanded(
            child: SelectableText(
              value,
              style: (monospace ? QxType.ident : QxType.caption)
                  .copyWith(color: theme.colorScheme.onSurface),
            ),
          ),
        ],
      ),
    );
  }
}

/// 一行说明文字。成功用主色，失败用 error。
class InlineNotice extends StatelessWidget {
  const InlineNotice({super.key, required this.text, required this.isError});

  final String text;
  final bool isError;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final color = isError ? theme.colorScheme.error : theme.colorScheme.onSurfaceVariant;
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(QxSpace.m),
      decoration: BoxDecoration(
        color: isError
            ? theme.colorScheme.error.withValues(alpha: 0.10)
            : theme.colorScheme.surfaceContainerLowest.withValues(alpha: 0.6),
        borderRadius: QxRadius.controlAll,
        border: Border.all(
          color: isError
              ? theme.colorScheme.error.withValues(alpha: 0.35)
              : QxColor.hairline(theme.brightness),
        ),
      ),
      child: Text(text, style: QxType.caption.copyWith(color: color)),
    );
  }
}

/// 相对时间。用于「3 分钟前使用」这类。
String relTime(int epochMs) {
  if (epochMs <= 0) return '从未使用';
  final diff = DateTime.now().millisecondsSinceEpoch - epochMs;
  if (diff < 0) return '刚刚';
  const minute = 60000;
  if (diff < minute) return '刚刚';
  if (diff < 60 * minute) return '${diff ~/ minute} 分钟前';
  if (diff < 24 * 60 * minute) return '${diff ~/ (60 * minute)} 小时前';
  if (diff < 30 * 24 * 60 * minute) return '${diff ~/ (24 * 60 * minute)} 天前';
  return '${diff ~/ (30 * 24 * 60 * minute)} 个月前';
}

/// 时长。用于「12s」「3m 20s」。
String formatDuration(Duration d) {
  if (d.inSeconds < 1) return '${d.inMilliseconds}ms';
  if (d.inSeconds < 60) return '${d.inSeconds}s';
  final m = d.inMinutes;
  final s = d.inSeconds % 60;
  if (m < 60) return s == 0 ? '${m}m' : '${m}m ${s}s';
  final h = m ~/ 60;
  return '${h}h ${m % 60}m';
}

/// 紧凑 token 数。1.2k 而不是 1234——列表行宽度有限。
String formatTokens(int tokens) {
  if (tokens < 1000) return '$tokens';
  if (tokens < 1000000) {
    final v = tokens / 1000;
    return '${v < 10 ? v.toStringAsFixed(1) : v.round()}k';
  }
  return '${(tokens / 1000000).toStringAsFixed(1)}M';
}

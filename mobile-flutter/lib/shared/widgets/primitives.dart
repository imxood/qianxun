import 'package:flutter/material.dart';

import '../../app/theme/qx_semantic.dart';
import '../../app/theme/qx_tokens.dart';
import '../../app/theme/qx_type.dart';

/// 状态点。
///
/// **不是**信号轨的替代品。轨用于列表行（可扫、需要排序），
/// 点用于列表行**之外**的场景（弹层、空态、日志行），那里没有「成列扫」
///的场景，一个 8px 的点加文字就够了。
class StateDot extends StatelessWidget {
  /// 会话态。色与中文标签都由 [QxSessionState] 决定。
  const StateDot({
    super.key,
    required this.state,
    this.size = 8,
    this.animate = false,
  })  : _color = null;

  /// 只给颜色。**仅用于连接态**——它有自己的词表
  /// （在线 / DSH 未运行 / 离线 / 凭据失效），与会话的五态不是一回事。
  ///
  /// ⚠️ 点自身**不标 Semantics**（否则整行 + 可见文字 + 点 = 读三遍）。
  /// 「绝不靠颜色单独表意」的责任落在**调用方**：
  /// 用了这个构造器，就必须在同一行渲染出可见的状态文字。
  /// 参见 `connections_screen.dart` 的 `_ConnectionRow`。
  const StateDot.colored({
    super.key,
    required Color color,
    this.size = 8,
    this.animate = false,
  })  : state = null,
        _color = color;

  final QxSessionState? state;
  final Color? _color;
  final double size;

  /// 运行态呼吸。**默认关**：默认态下树里不能有无限动画，
  /// 否则 `pumpAndSettle` 会超时。
  final bool animate;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    // 两个构造器都保证有 label，这里只是为了让类型收窄。
    final color = state?.colorOf(q) ?? _color ?? q.detached;
    final glyph = state?.glyph ?? Icons.circle;

    // ⚠️ 这里**不标 Semantics**：整行的 InkWell 已经带上了状态文字，
    // 点自己再标一次会让 TalkBack 把同一个词读三遍（三次停顿），
    // 用户会以为卡住了。`excludeSemantics` 之外的重复标注是噪音。
    if (!animate) {
      return ExcludeSemantics(
        child: SizedBox(
          width: size,
          height: size,
          // 静态态也**必须有字形**。纯色点对色觉障碍用户与灰度截图
          // 零信息；字形 + 旁边的中文词构成两条独立通道。
          child: Icon(glyph, size: size, color: color),
        ),
      );
    }
    return _BreathingDot(color: color, size: size, glyph: glyph);
  }
}

/// 呼吸点：核心 + 一圈缓慢扩张淡出的光晕。
///
/// 表达「它在跑」而不是「在等」。等状态用静态点 + 虚线轨，
/// 因为「等」是静止的状态，用呼吸会误导。
class _BreathingDot extends StatefulWidget {
  const _BreathingDot({
    required this.color,
    required this.size,
    required this.glyph,
  });

  final Color color;
  final double size;
  final IconData glyph;

  @override
  State<_BreathingDot> createState() => _BreathingDotState();
}

class _BreathingDotState extends State<_BreathingDot>
    with SingleTickerProviderStateMixin {
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
    return ExcludeSemantics(
      child: RepaintBoundary(
        child: SizedBox(
          width: widget.size * 2.6,
          height: widget.size * 2.6,
          child: AnimatedBuilder(
            animation: _c,
            builder: (context, _) {
              final t = _c.value;
              final fade = (1 - t).clamp(0.0, 1.0);
              return Stack(
                alignment: Alignment.center,
                children: [
                  Container(
                    width: widget.size * (1 + t * 0.9),
                    height: widget.size * (1 + t * 0.9),
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      color: widget.color.withValues(alpha: 0.26 * fade),
                    ),
                  ),
                  Icon(widget.glyph, size: widget.size * 1.6, color: widget.color),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}

/// 状态标签。
///
/// 状态词**必须**以可见中文出现——纯色点在色觉障碍用户与灰度截图下
/// 毫无信息。这是整个 App 不可协商的无障碍底线。
class StateBadge extends StatelessWidget {
  const StateBadge({super.key, required this.state, this.dense = false});

  final QxSessionState state;
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    final color = state.colorOf(q);
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: dense ? 4 : 6,
        vertical: dense ? 1 : 2,
      ),
      decoration: BoxDecoration(
        // ⚠️ 不要给状态词加半透明底色。状态色本身在 canvas 上就贴着
        // 4.5:1 的下限，再压一层 14% 的同色底会把它拖到 4:0 左右，
        // 亮色模式五个状态全部不达标。描边可以弱化，字色必须是原色。
        borderRadius: QxRadius.chipAll,
        border: Border.all(color: color.withValues(alpha: 0.45)),
      ),
      child: Text(state.label, style: QxType.badge(color)),
    );
  }
}

/// 徽标。等宽 + 小字 + 描边。
///
/// 用在**非状态**的场合（工具名、字节数、模式名）。
/// 状态本身请用 [StateBadge]，它带中文标签，无障碍更稳。
class MonoBadge extends StatelessWidget {
  const MonoBadge({
    super.key,
    required this.label,
    required this.color,
    this.filled = false,
  });

  final String label;
  final Color color;
  final bool filled;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      decoration: BoxDecoration(
        color: filled ? color.withValues(alpha: 0.16) : Colors.transparent,
        borderRadius: QxRadius.chipAll,
        border: Border.all(color: color.withValues(alpha: 0.42)),
      ),
      child: Text(label, style: QxType.badge(color)),
    );
  }
}

/// 一行「标签 : 值」。值默认等宽。
///
/// 这里的 `:` 分隔是参考客户端 `Title :: value` 排版装置的轻量版：
/// 元数据多起来之后，没有分隔符的「标题 值」会糊成一团。
class MetaRow extends StatelessWidget {
  const MetaRow({
    super.key,
    required this.label,
    required this.value,
    this.monospace = true,
  });

  final String label;
  final String value;
  final bool monospace;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    // 等宽**数据层**锁回 1.0x（04 裁决表）：这一行是「标签 : 值」的
    // 对齐装置，值放大后列就散了。全局上限已经放宽到 2.0（正文要跟着
    // 系统走），保护落在这一层。
    final valueText = SelectableText(
      value,
      style: (monospace ? QxType.ident : QxType.caption)
          .copyWith(color: q.textPrimary),
    );
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: QxSpace.s),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 72,
            child: Text(
              label,
              style: QxType.caption.copyWith(color: q.textSecondary),
            ),
          ),
          const SizedBox(width: QxSpace.m),
          Expanded(
            child: monospace ? QxType.dataScaleLocked(child: valueText) : valueText,
          ),
        ],
      ),
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
              style: QxType.overline.copyWith(color: context.qx.textSecondary),
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
/// 空状态是唯一允许「说一句人话」的地方——它是用户与功能之间的第一次
/// 接触，应该解释**为什么空**以及**下一步做什么**，而不是画个灰图标。
class EmptyHero extends StatelessWidget {
  const EmptyHero({
    super.key,
    required this.title,
    required this.hint,
    this.icon = Icons.inbox_outlined,
    this.action,
  });

  final String title;
  final String hint;
  final IconData icon;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(QxSpace.xxxl),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 56,
              height: 56,
              decoration: BoxDecoration(
                color: q.surfaceSunken,
                borderRadius: QxRadius.blockAll,
                border: Border.all(color: q.hairline),
              ),
              child: Icon(icon, size: 24, color: q.textSecondary),
            ),
            const SizedBox(height: QxSpace.l),
            Text(title, style: QxType.body.copyWith(color: q.textPrimary)),
            const SizedBox(height: QxSpace.s),
            Text(
              hint,
              textAlign: TextAlign.center,
              style: QxType.caption.copyWith(color: q.textSecondary, height: 1.5),
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

/// 一行提示。成功用次要色，失败用 error 色。
class InlineNotice extends StatelessWidget {
  const InlineNotice({super.key, required this.text, required this.isError});

  final String text;
  final bool isError;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    final color = isError ? q.failed : q.textSecondary;
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(QxSpace.m),
      decoration: BoxDecoration(
        color: (isError ? q.failed : q.surfaceSunken).withValues(alpha: isError ? 0.10 : 1),
        borderRadius: QxRadius.controlAll,
        border: Border.all(
          color: isError ? q.failed.withValues(alpha: 0.35) : q.hairline,
        ),
      ),
      child: Text(text, style: QxType.caption.copyWith(color: color, height: 1.45)),
    );
  }
}

/// 相对时间。
String relTime(int epochMs) {
  if (epochMs <= 0) return '从未用过';
  final diff = DateTime.now().millisecondsSinceEpoch - epochMs;
  if (diff < 0) return '刚刚';
  const minute = 60000;
  if (diff < minute) return '刚刚';
  if (diff < 60 * minute) return '${diff ~/ minute} 分钟前';
  if (diff < 24 * 60 * minute) return '${diff ~/ (60 * minute)} 小时前';
  if (diff < 30 * 24 * 60 * minute) return '${diff ~/ (24 * 60 * minute)} 天前';
  return '${diff ~/ (30 * 24 * 60 * minute)} 个月前';
}

/// 时长。
String formatDuration(Duration d) {
  if (d.inSeconds < 1) return '${d.inMilliseconds}ms';
  if (d.inSeconds < 60) return '${d.inSeconds}s';
  final m = d.inMinutes;
  final s = d.inSeconds % 60;
  if (m < 60) return s == 0 ? '${m}m' : '${m}m ${s}s';
  return '${m ~/ 60}h ${m % 60}m';
}

/// 紧凑 token 数。`1.2k` 而不是 `1234`。
String formatTokens(int tokens) {
  if (tokens < 1000) return '$tokens';
  if (tokens < 1000000) {
    final v = tokens / 1000;
    return '${v < 10 ? v.toStringAsFixed(1) : v.round()}k';
  }
  return '${(tokens / 1000000).toStringAsFixed(1)}M';
}

/// 相对字节。
String formatBytes(int bytes) {
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(0)} KB';
  if (bytes < 1024 * 1024 * 1024) {
    return '${(bytes / 1024 / 1024).toStringAsFixed(1)} MB';
  }
  return '${(bytes / 1024 / 1024 / 1024).toStringAsFixed(1)} GB';
}

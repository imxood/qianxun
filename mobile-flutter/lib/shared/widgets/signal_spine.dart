import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../app/theme/qx_semantic.dart';
import '../../app/theme/qx_tokens.dart';

/// 信号轨（Signal Spine）。
///
/// 本 App 的签名元素。一条 3-4dp 宽的竖轨，**出血到屏幕边**，贯穿每一行
/// 会话、会话详情头、审批入口的最左缘。它的**粗细、明暗、纹理、运动**
/// 四重编码状态。
///
/// 为什么不叫「状态条」也不做成小圆点：
///
/// - **列状**。一屏 8 个会话扫过去是一排长短、明暗、动静不同的竖条，
///   形成一个**可排序的视觉序列**。用户纯靠余光就能把「要我管的」挑
///   出来，一个字都不用读。
/// - **出血**。它连成屏幕上的一条连续的线，像监控面板的信号轨。
///   内缩成「卡片左边一个小色块」就退化成通用 App 了。
///
/// ⚠️ 轨宽分四档，规格见 `docs/design/05-product-design.md` §3.1：
/// 「运行中」4dp、「等你」3dp 是仅有的两档粗轨，「已断开」2dp、
/// 「已完成」1dp 退到背景里。**条目再多也不许降级**（05 §3.2）——
/// 真正该做的是把两档粗轨保住，而不是让等待混进 1dp 那一堆里。
class SignalSpine extends StatefulWidget {
  const SignalSpine({
    super.key,
    required this.state,
    required this.height,
    this.gutter = QxSpace.m,
  });

  final QxSessionState state;

  /// 轨的**视觉高度**，不跟行高等高（05 §3.1：通高 54dp，不满格）。
  ///
  /// ⚠️ 必填，不能给默认值。给成可选会踩一颗雷：`SizedBox(height: null)`
  /// 在 `ListView` 的纵向无界约束下会走到 `SizedBox.expand`，拿到
  /// `Size.infinite`，直接抛 `RenderBox was not laid out … hasSize`。
  /// 而所有现存测试都传了 54，闸门完全看不见这个坑——它只会在
  /// Batch 3 真的把轨接进行里那天炸出来。宁可让调用方明确写高度。
  final double height;

  /// 轨与内容之间的**真实间隔**。这个间隔由外层 Padding 负责，
  /// 轨本身只占 [QxSessionState.railWidth]。
  final double gutter;

  @override
  State<SignalSpine> createState() => _SignalSpineState();
}

class _SignalSpineState extends State<SignalSpine>
    with SingleTickerProviderStateMixin {
  /// **只创建一次**。
  ///
  /// ⚠️ 曾经在这里 dispose+重建它（切状态时），`SingleTickerProviderStateMixin`
  /// 的 `createTicker` 会断言「multiple tickers were created」——
  /// mixin 只在 `dispose()` 里清空 `_ticker`，手工 `controller.dispose()`
  /// 不会。所以：**一个 provider 一个 controller，切换用 repeat/stop 表达。**
  late final AnimationController _flow = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1600),
  );

  @override
  void initState() {
    super.initState();
    _applyMotion();
  }

  @override
  void didUpdateWidget(covariant SignalSpine old) {
    super.didUpdateWidget(old);
    if (old.state != widget.state) _applyMotion();
  }

  /// 只有运行中有运动。静止的态用呼吸会误导。
  void _applyMotion() {
    if (widget.state.railAnimated) {
      if (!_flow.isAnimating) _flow.repeat();
    } else {
      _flow
        ..stop()
        ..value = 0;
    }
  }

  @override
  void dispose() {
    _flow.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    final width = widget.state.railWidth;

    return Semantics(
      // 3px 的轨对 TalkBack 零信息，必须给它标签。
      label: widget.state.label,
      excludeSemantics: true,
      child: SizedBox(
        width: width + widget.gutter,
        height: widget.height,
        child: Align(
          // ⚠️ 必须 Align + 显式 SizedBox(width)。
          //
          // 直接 `Container(color: …)` 没有 child 时会**撑满约束**：
          // 在 ListView 里子节点拿到的是紧的交叉轴约束，轨会变成一条
          // 铺满整行的色带（实测 768dp）——正好是「3-4dp 出血竖轨」的
          // 反面。宽度必须由自己指定，不能指望父级给。
          alignment: Alignment.centerLeft,
          child: SizedBox(
            width: width,
            height: widget.height,
            child: ClipRRect(
              borderRadius: BorderRadius.circular(width),
              child: widget.state.railDashed
                  ? _DashedRail(color: widget.state.railColor(q))
                  // ⚠️ 彗星**只**在真正会动的态上挂。
                  // `_SolidRail` 覆盖所有非虚线态（含 done / failed），若一律
                  // 传 controller，静止时 `value == 0` 仍会画出上半段
                  // 0→0.6 的白渐变：done 轨的上半被抬到 ≈4.8:1，直接废掉
                  // 「完成要退后而不是庆祝」，还暗示了不存在的运动。
                  : _SolidRail(
                      color: widget.state.railColor(q),
                      flow: widget.state.railAnimated ? _flow : null,
                    ),
            ),
          ),
        ),
      ),
    );
  }
}

/// 颜色之外的第二通道：虚线段。灰度截图下这是唯一还能区分「等你」与
/// 「已断开」的线索。
class _DashedRail extends StatelessWidget {
  const _DashedRail({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      // CustomPaint 而不是 Row 拼 widget：3dp 宽、十几段 dash，
      // 用 widget 拼会产生几十个 RenderObject。
      child: CustomPaint(
        painter: _DashesPainter(color: color),
        size: Size.infinite,
      ),
    );
  }
}

class _SolidRail extends StatelessWidget {
  const _SolidRail({required this.color, required this.flow});

  final Color color;

  /// 可空：**没有它就不画彗星**。
  ///
  /// 静止态传 null，而不是传一个停在 0 的 controller —— 后者仍会画出
  /// 上半段白渐变，把 done 轨从「退后」抬成「有动静」。
  final AnimationController? flow;

  @override
  Widget build(BuildContext context) {
    final f = flow;
    return RepaintBoundary(
      child: SizedBox.expand(
        child: Stack(
          fit: StackFit.expand,
          children: [
            ColoredBox(color: color),
            // 流光：一条更亮的彗尾自上而下循环，让「它在跑」在余光里就
            // 能看见，不必读文字。
            if (f != null)
              AnimatedBuilder(
                animation: f,
                builder: (context, _) => CustomPaint(
                  painter: _CometPainter(progress: f.value),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _CometPainter extends CustomPainter {
  _CometPainter({required this.progress});

  final double progress;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.height <= 0 || size.width <= 0) return;
    final tail = size.height * 0.45;
    final y = -tail + progress * (size.height + tail);
    final top = math.max(0.0, y - tail);
    final h = math.min(tail, size.height - top);
    if (h <= 0) return;
    final paint = Paint()
      ..shader = LinearGradient(
        begin: Alignment.topCenter,
        end: Alignment.bottomCenter,
        colors: [Colors.white.withValues(alpha: 0), Colors.white.withValues(alpha: 0.6)],
      ).createShader(Rect.fromLTWH(0, top, size.width, h));
    canvas.drawRect(Rect.fromLTWH(0, top, size.width, h), paint);
  }

  @override
  bool shouldRepaint(covariant _CometPainter old) => old.progress != progress;
}

class _DashesPainter extends CustomPainter {
  _DashesPainter({required this.color});

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0 || size.height <= 0) return;
    final paint = Paint()..color = color;
    const dash = 4.0;
    const gap = 3.0;
    for (var y = 0.0; y < size.height; y += dash + gap) {
      canvas.drawRect(
        Rect.fromLTWH(0, y, size.width, math.min(dash, size.height - y)),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(covariant _DashesPainter old) => old.color != color;
}

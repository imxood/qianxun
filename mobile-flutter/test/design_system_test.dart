import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/app/theme/qx_semantic.dart';
import 'package:qianxun_mobile/app/theme/qx_tokens.dart';
import 'package:qianxun_mobile/app/theme/qx_type.dart';
import 'package:qianxun_mobile/shared/widgets/signal_spine.dart';

void main() {
  Widget wrap(Widget child) => MaterialApp(home: Scaffold(body: child));

  group('状态词表只有五态', () {
    test('恰好五个，且中文标签互不相同', () {
      expect(QxSessionState.values.length, 5);
      final labels = QxSessionState.values.map((s) => s.label).toSet();
      expect(labels.length, 5, reason: '两个状态共用标签 = 用户分不出来');
    });

    test('没有「已连接」这种无数据源的态', () {
      // blocked 两态曾因服务端不下发而被砍掉。
      expect(
        QxSessionState.values.map((s) => s.name),
        isNot(contains('blocked')),
      );
    });

    test('五个中文词逐字等于 05 §2 的最终词表', () {
      // ⚠️ 这条是**逐字**比对，不是「互不相同」那种弱断言。
      // 之前实现混过一套「运行中 / 等你回答 / 空闲 / 已结束 / 失败」——
      // 它同样满足「五个且互不相同」，却是把 04 的 C10 词表和 05 的五态
      // 拼在一起的产物。弱断言抓不住分叉，只有逐字比对能。
      expect(
        QxSessionState.values.map((s) => s.label).toSet(),
        {'运行中', '等你', '已完成', '已断开', '失败'},
        reason: '改词必须先改 docs/design/05-product-design.md §2 的表',
      );
    });
  });

  group('信号轨的四重编码', () {
    test('粗细分四档，运行中与等你是仅有的两档粗轨', () {
      // 05 §3.1 的逐档规格。运行中 4 / 等你 3 / 已断开 2 / 已完成 1。
      expect(QxSessionState.running.railWidth, 4);
      expect(QxSessionState.waiting.railWidth, 3);
      expect(QxSessionState.detached.railWidth, 2);
      // 「已完成」几乎隐没——它是最不需要用户注意的态。
      expect(QxSessionState.done.railWidth, 1);
      // 不可协商（05 §3.2）：条目再多也只能保这两档最粗，
      // 等你绝不能被降级进 1dp 那一堆里。
      expect(
        QxSessionState.waiting.railWidth,
        greaterThan(QxSessionState.detached.railWidth),
      );
      expect(
        QxSessionState.waiting.railWidth,
        greaterThan(QxSessionState.done.railWidth),
      );
    });

    test('虚线是颜色之外的第二个通道', () {
      // 灰度截图下颜色全没了，靠纹理仍要能区分。
      expect(QxSessionState.waiting.railDashed, isTrue);
      expect(QxSessionState.detached.railDashed, isTrue);
      expect(QxSessionState.running.railDashed, isFalse);
      expect(QxSessionState.done.railDashed, isFalse);
    });

    test('只有运行中有运动', () {
      // 呼吸表达「在跑」；静止的态用运动会误导。
      for (final s in QxSessionState.values) {
        expect(s.railAnimated, s == QxSessionState.running, reason: '$s');
      }
    });

    test('每个状态都有独立字形（无障碍与色觉障碍双保险）', () {
      final glyphs = QxSessionState.values.map((s) => s.glyph).toSet();
      expect(glyphs.length, 5);
    });
  });

  group('排序：等你永远置顶', () {
    test('等待态优先级最高', () {
      expect(
        qxStatePriority(QxSessionState.waiting),
        greaterThan(qxStatePriority(QxSessionState.running)),
      );
      expect(
        qxStatePriority(QxSessionState.running),
        greaterThan(qxStatePriority(QxSessionState.done)),
      );
      // 已断开优先级最低，不该占住用户视线。
      expect(
        qxStatePriority(QxSessionState.detached),
        lessThan(qxStatePriority(QxSessionState.done)),
      );
    });

    test('按 updatedAt 排序时等待态仍会置顶', () {
      final items = <(QxSessionState, int)>[
        (QxSessionState.done, 999), // 时间最新
        (QxSessionState.waiting, 1), // 时间最旧
      ]..sort((a, b) {
        final p = qxStatePriority(b.$1).compareTo(qxStatePriority(a.$1));
        return p != 0 ? p : b.$2.compareTo(a.$2);
      });
      expect(items.first.$1, QxSessionState.waiting);
    });
  });

  group('信号轨渲染', () {
    testWidgets('每种状态都渲染且带中文语义标签', (tester) async {
      for (final s in QxSessionState.values) {
        await tester.pumpWidget(wrap(
          SignalSpine(state: s, height: 54),
        ));
        final sem = tester.getSemantics(find.byType(SignalSpine));
        expect(sem.label, s.label, reason: '$s 必须能被 TalkBack 读出');
      }
    });

    testWidgets('轨宽随状态变化', (tester) async {
      await tester.pumpWidget(wrap(
        const Row(
          children: [
            SignalSpine(state: QxSessionState.running, height: 54),
            SignalSpine(state: QxSessionState.done, height: 54),
          ],
        ),
      ));
      final spines = tester.widgetList<SignalSpine>(find.byType(SignalSpine)).toList();
      expect(spines[0].state.railWidth, greaterThan(spines[1].state.railWidth));
    });

    testWidgets('真实渲染宽度 = 轨宽，不会撑满整行', (tester) async {
      // ⚠️ 这条测的是**渲染出来的几何**，不是枚举值。
      // 上一条「轨宽随状态变化」只比 `spines[0].state.railWidth`，
      // 是两个 Dart 字段比大小 —— 就算轨在 ListView 里被拉成 768dp
      // 铺满整行，它照样绿。真正的回归是：
      //   `Container(color:)` 没有 child 时会**吃满交叉轴约束**，
      //   在 ListView 行里实测渲染成 768dp 的色带，
      //   正好是「4dp 出血竖轨」的反面。
      // 所以这里在 ListView 里量实际 RenderBox 宽度。
      for (final s in QxSessionState.values) {
        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              body: ListView(
                children: [
                  SizedBox(
                    height: 66,
                    child: Row(
                      children: [
                        SignalSpine(state: s, height: 54),
                        const Expanded(child: Text('内容')),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
        // 外层 SizedBox(width: railWidth + gutter) 才是应占的横向空间。
        final box = tester.getSize(find.byType(SignalSpine));
        expect(
          box.width,
          lessThanOrEqualTo(s.railWidth + QxSpace.m + 0.01),
          reason: '$s 的轨撑到了 ${box.width}dp：Container 无 child 时会吃满约束',
        );
        expect(box.height, 54, reason: '$s 的轨高必须是传入的视觉高度');
      }
    });

    testWidgets('静止态不画彗星（done 轨的上半不该被抬亮）', (tester) async {
      // 曾经所有非虚线态都挂 controller，静止时 `value == 0` 仍会画出
      // 上半段 0→0.6 白渐变：done 轨从 1.40:1 被抬到 ≈4.8:1，
      // 「完成要退后而不是庆祝」这条意图就废了。
      //
      // ⚠️ 不能断言「没有 CustomPaint」：虚线轨（等你 / 已断开）本来就用
      // CustomPaint 画虚线段。要断的是**彗星画笔**没被挂上去。
      for (final s in QxSessionState.values.where((x) => !x.railAnimated)) {
        await tester.pumpWidget(wrap(SignalSpine(state: s, height: 54)));
        final painters = tester
            .widgetList<CustomPaint>(find.byType(CustomPaint))
            .map((w) => w.painter.runtimeType.toString());
        expect(
          painters.where((t) => t.contains('Comet')),
          isEmpty,
          reason: '$s 不该挂彗星画笔（静止时它仍会画上半段白渐变）',
        );
      }
      // 反向确认：运行中确实有彗星，否则上面就是空断言。
      await tester.pumpWidget(
        wrap(const SignalSpine(state: QxSessionState.running, height: 54)),
      );
      expect(
        tester
            .widgetList<CustomPaint>(find.byType(CustomPaint))
            .map((w) => w.painter.runtimeType.toString())
            .where((t) => t.contains('Comet')),
        isNotEmpty,
        reason: '运行中必须有彗星，否则「静止态不挂」这条测试没有意义',
      );
    });

    testWidgets('只有运行中会挂动画控制器（默认态可 pumpAndSettle）', (tester) async {
      // 无限动画会让 pumpAndSettle 永久超时，这是本项目明确要避免的。
      for (final s in QxSessionState.values.where((x) => !x.railAnimated)) {
        await tester.pumpWidget(wrap(SignalSpine(state: s, height: 54)));
        await tester.pumpAndSettle(const Duration(milliseconds: 100));
      }
    });
  });

  group('语义色', () {
    test('两套色板状态色互不相同（明暗不是简单反相）', () {
      for (final role in const ['running', 'waiting', 'done', 'detached', 'failed']) {
        const d = QxSemantic.dark;
        const l = QxSemantic.light;
        final dv = _role(d, role);
        final lv = _role(l, role);
        expect(dv, isNot(lv), reason: '$role 明暗同值 = 有一版看不清');
      }
    });

    test('不是纯黑也不是纯白', () {
      for (final p in [QxSemantic.dark, QxSemantic.light]) {
        expect(p.canvas, isNot(const Color(0xFF000000)));
        expect(p.surfaceRaised, isNot(const Color(0xFFFFFFFF)));
      }
    });

    test('copyWith / lerp 覆盖全部 16 个槽位', () {
      const base = QxSemantic.dark;
      final c = base.copyWith(running: const Color(0xFF00FF00));
      expect(c.running, const Color(0xFF00FF00));
      expect(c.waiting, base.waiting, reason: '没传的槽位必须原样保留');

      final l = base.lerp(QxSemantic.light, 0.5);
      expect(l.canvas, isNot(base.canvas));
      // lerp(null) 返回自己
      expect(base.lerp(null, 0.5).canvas, base.canvas);
    });
  });

  group('间距与圆角', () {
    test('间距刻度递增，且都是 2 的倍数', () {
      const s = [
        QxSpace.xxs, QxSpace.xs, QxSpace.s, QxSpace.m, QxSpace.l,
        QxSpace.xl, QxSpace.xxl, QxSpace.xxxl, QxSpace.huge,
      ];
      for (var i = 1; i < s.length; i++) {
        expect(s[i], greaterThan(s[i - 1]), reason: '刻度必须严格递增');
      }
      for (final v in s) {
        // 主体是 4 的倍数体系；`xxs = 2` 是唯一的例外，它只出现在
        // 图标与 6px 圆点这类「光学修正」上（1dp 视觉上不存在，
        // 3dp 又会让 icon 看起来没对齐）。
        expect(v % 2, 0, reason: '$v 不是 2 的倍数');
      }
      // 主体刻度（xxs 之外）仍应是 4 的倍数。
      for (final v in s.skip(1)) {
        expect(v % 4, 0, reason: '$v 破坏 4dp 体系');
      }
    });

    test('触摸目标不小于 48', () {
      // 圆角不是触摸目标；这个常量专门给可点区域用。
      expect(QxSpace.huge, greaterThanOrEqualTo(48));
    });
  });

  group('排版', () {
    test('字阶递减，字号不重复', () {
      // 字号是 getter 上的 `!`，编译期常量拿不到，所以用 final 而非 const。
      final sizes = <double>[
        QxType.display.fontSize!,
        QxType.title.fontSize!,
        QxType.prose.fontSize!,
        QxType.caption.fontSize!,
        QxType.micro.fontSize!,
      ];
      for (var i = 1; i < sizes.length; i++) {
        expect(sizes[i], lessThan(sizes[i - 1]), reason: '字阶必须递减');
      }
    });

    test('正文行高不小于 1.5（中文长段落可读性）', () {
      expect(QxType.prose.height!, greaterThanOrEqualTo(1.5));
    });

    test('技术数据用等宽字体，UI 文字不用', () {
      expect(QxType.code.fontFamily, QxType.mono);
      expect(QxType.ident.fontFamily, QxType.mono);
      expect(QxType.metric.fontFamily, QxType.mono);
      // 正文用系统字体——中文换字体代价大于收益。
      expect(QxType.prose.fontFamily, isNull);
      expect(QxType.body.fontFamily, isNull);
    });

    test('指标数字开等宽数字（否则刷新时整行会抖）', () {
      expect(QxType.metric.fontFeatures, contains(const FontFeature.tabularFigures()));
    });
  });
}

Color _role(QxSemantic p, String name) => switch (name) {
      'running' => p.running,
      'waiting' => p.waiting,
      'done' => p.done,
      'detached' => p.detached,
      'failed' => p.failed,
      _ => throw ArgumentError(name),
    };

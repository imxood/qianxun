import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/app/theme/qx_semantic.dart';
import 'package:qianxun_mobile/app/theme/qx_theme.dart';
import 'package:qianxun_mobile/data/models/content.dart';
import 'package:qianxun_mobile/data/models/session.dart';
import 'package:qianxun_mobile/features/sessions/application/session_presentation.dart';
import 'package:qianxun_mobile/features/sessions/presentation/sessions_screen.dart';
import 'package:qianxun_mobile/features/sessions/presentation/session_tile.dart';
import 'package:qianxun_mobile/features/workspace/application/transcript_controller.dart';
import 'package:qianxun_mobile/features/workspace/presentation/transcript_view.dart';
import 'package:qianxun_mobile/shared/widgets/primitives.dart';
import 'package:qianxun_mobile/shared/widgets/signal_spine.dart';

/// 界面测试：不依赖设备与网络，直接把 widget 泵进测试树。
///
/// 这不是「凑合的替代品」——它验证的是**布局在给定宽度下真的不溢出、
/// 状态分支渲染正确**，这些正是真机截图看不出问题的地方。
void main() {
  Widget wrap(Widget child, {Size size = const Size(390, 844)}) {
    return MaterialApp(
      theme: QxTheme.dark(),
      home: Scaffold(
        body: Center(
          child: SizedBox(width: size.width, child: child),
        ),
      ),
    );
  }

  group('基础组件', () {
    testWidgets('MonoBadge 渲染文本与语义色', (tester) async {
      await tester.pumpWidget(wrap(
        const MonoBadge(label: 'RUNNING', color: Color(0xFF3DDC97), filled: true),
      ));
      expect(find.text('RUNNING'), findsOneWidget);
    });

    testWidgets('EmptyHero 给出可执行的下一步', (tester) async {
      var tapped = false;
      await tester.pumpWidget(wrap(
        EmptyHero(
          title: '还没有连接任何电脑',
          hint: '在电脑端打开「设置 → 远程访问」',
          action: FilledButton(
            onPressed: () => tapped = true,
            child: const Text('添加电脑'),
          ),
        ),
      ));
      expect(find.text('还没有连接任何电脑'), findsOneWidget);
      expect(find.text('添加电脑'), findsOneWidget);
      await tester.tap(find.text('添加电脑'));
      expect(tapped, isTrue);
    });

    testWidgets('InlineNotice 在错误态用 error 色', (tester) async {
      await tester.pumpWidget(wrap(
        const InlineNotice(text: '配对失败：凭据已过期', isError: true),
      ));
      expect(find.textContaining('配对失败'), findsOneWidget);
    });

    testWidgets('StateDot 呼吸态用 RepaintBoundary 包裹（隔离重绘）', (tester) async {
      await tester.pumpWidget(wrap(
        const StateDot(
        state: QxSessionState.running,
        animate: true,
      ),
      ));
      expect(find.byType(StateDot), findsOneWidget);
      expect(find.byType(RepaintBoundary), findsWidgets);
    });
  });

  group('会话行', () {
    SessionSummary build({
      String sessionId = 's1',
      bool running = false,
      String? title,
      String? cwd,
      Map<String, Object?>? projections,
      Map<String, Object?>? userQuestions,
    }) {
      // ⚠️ 合并成一个 values：写两个 `'projections':` 键时后者会覆盖前者，
      // 曾经因此让「12 轮 / 48k ctx」三个断言全部落空。
      final values = <String, Object?>{
        if (title != null) 'title': title,
        ...?projections,
      };
      return SessionSummary.fromJson({
        'sessionId': sessionId,
        'updatedAt': DateTime.now().millisecondsSinceEpoch - 60000,
        'running': running,
        'blank': false,
        if (cwd != null) 'cwd': cwd,
        if (userQuestions != null) 'userQuestions': userQuestions,
        'projections': {'asOfSeq': 1, 'values': values},
      });
    }

    /// 走真实的状态推导，不在测试里手挑状态 —— 否则测的是「我给什么
    /// 它渲染什么」，而不是「这份数据该是什么态」。
    SessionTile tile(SessionSummary s) =>
        SessionTile(item: presentSession(s), onTap: () {});

    testWidgets('显示投影里的标题，不显示"未命名会话"', (tester) async {
      await tester.pumpWidget(wrap(tile(build(title: '重构网关鉴权'))));
      expect(find.text('重构网关鉴词'), findsNothing);
      expect(find.text('重构网关鉴权'), findsOneWidget);
    });

    testWidgets('没有标题投影时退回 cwd 末段，且副行不重复它', (tester) async {
      await tester.pumpWidget(wrap(tile(build(cwd: '/home/me/deep/app'))));
      // 「只出现一次」**就是**不重复的证据：标题回退到 cwd 末段 `app`，
      // 元信息行若再显示一遍 `app`，这里会是 findsNWidgets(2) 而不是 1。
      //
      // ⚠️ 原来这条断言写了两遍一模一样的 findsOneWidget，于是
      // 一条名叫「副行不重复它」的测试永远不会失败 —— 名字承诺的
      // 不变量根本没被断言。合成一条，并把理由写清楚。
      expect(find.text('app'), findsOneWidget);
    });

    testWidgets('有标题时元信息行才显示 cwd', (tester) async {
      await tester.pumpWidget(
        wrap(tile(build(title: '重构鉴权', cwd: '/home/me/proj'))),
      );
      expect(find.text('重构鉴权'), findsOneWidget);
      expect(find.text('proj'), findsOneWidget);
    });

    testWidgets('轨出血到屏幕左缘：行内贴行首，且列表**不留左内边距**', (tester) async {
      // 05 §3.1：轨**出血到屏幕边**，不是内缩成「卡片左边一个小色块」。
      //
      // ⚠️ 这里要断**两处**，缺一不可：
      //   1. 行内：轨的 dx 与行的 dx 齐平（`SignalSpine` 不自带缩进）
      //   2. 页面：`ListView` 的 padding 不含 left。旧实现正是
      //      `EdgeInsets.fromLTRB(QxSpace.l, …)`，轨整体右移了 16dp，
      //      屏幕左缘那条连续的线就断了 —— 而只断第 1 条的话它照样绿。
      //    本批把那条 padding 删掉了，所以这里也要钉住「删掉之后别加回来」。
      expect(
        SessionsScreen.sessionsListHasNoLeftInset,
        isTrue,
        reason: '会话列表的 ListView 重新出现了左内边距，轨就不出血了',
      );

      await tester.pumpWidget(
        MaterialApp(
          theme: QxTheme.dark(),
          home: Scaffold(
            body: ListView(
              padding: const EdgeInsets.symmetric(vertical: 8),
              children: [tile(build(title: 'x'))],
            ),
          ),
        ),
      );
      final tileTopLeft = tester.getTopLeft(find.byType(SessionTile));
      final spineLeft = tester.getTopLeft(find.byType(SignalSpine));
      expect(
        spineLeft.dx - tileTopLeft.dx,
        0,
        reason: '轨必须与行的左缘齐平（出血），不能内缩',
      );
    });

    testWidgets('行高固定 66dp，idle 与 running 同高', (tester) async {
      // 05 §4.2。两态行高不同的话，扫一列时行的位置随状态漂移。
      for (final running in [true, false]) {
        await tester.pumpWidget(wrap(tile(build(title: 'x', running: running))));
        expect(
          tester.getSize(find.byType(SessionTile)).height,
          SessionTile.rowHeight,
          reason: 'running=$running 时行高必须是 66',
        );
        expect(find.byType(SignalSpine), findsOneWidget, reason: '轨必须渲染');
      }
    });

    testWidgets('状态中文词在任何字号下都**不被裁切**', (tester) async {
      // 05 §4.4 说「溢出用 …」，但那句话只对**次要信息**成立。
      // 「绝不靠颜色单独表意」要求每态配一个可见中文词；状态词一旦被裁成
      // 「运行…」，轨的语义就只剩颜色，而 1dp 的已完成轨颜色本身就看不清。
      //
      // ⚠️ `find.text('运行中')` 查的是 **widget**，不是字形 —— 就算被裁成
      // 「运行…」，它照样 findsOneWidget。所以这里量的是
      // `RenderParagraph.didExceedMaxLines`。
      for (final scale in [1.0, 1.5, 2.0]) {
        for (final width in [320.0, 360.0, 390.0]) {
          tester.view.physicalSize = Size(width * 3, 844 * 3);
          tester.view.devicePixelRatio = 3.0;
          await tester.pumpWidget(
            MaterialApp(
              theme: QxTheme.dark(),
              home: Scaffold(
                body: MediaQuery(
                  data: MediaQueryData(textScaler: TextScaler.linear(scale)),
                  child: ListView(
                    children: [
                      tile(
                        build(
                          title: '一个相当长的会话标题',
                          cwd: '/home/me/very/deep/project',
                          projections: const {
                            'sessionStats': {'turns': 128},
                            'contextPressure': {'projectedTokens': 48600000},
                          },
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          );
          final label = find.text('已完成');
          expect(label, findsOneWidget);
          // 量「实际渲染宽度」与「无约束下的固有宽度」之差。
          // ⚠️ 不能只看 `didExceedMaxLines`：状态词用的是 `softWrap: false`，
          // 挤不下时是**溢出**而不是省略，那个标志不可靠。
          final text = tester.widget<Text>(label);
          final painter = TextPainter(
            text: TextSpan(text: text.data!, style: text.style),
            textDirection: TextDirection.ltr,
            maxLines: 1,
          )..layout();
          final actual = tester.getSize(label).width;
          expect(
            actual + 0.5,
            greaterThanOrEqualTo(painter.width),
            reason: '${width}dp @ ${scale}x：状态词「已完成」被压缩到 '
                '$actual < 固有 ${painter.width}',
          );
        }
      }
      tester.view.reset();
    });

    testWidgets('第二行是 agent 做的事，不是用户说的话', (tester) async {
      // 05 §4.3 的核心规则，也是本批最贵的一次分歧的落点。
      await tester.pumpWidget(
        wrap(
          tile(
            build(
              title: '部署',
              projections: const {
                'turnOutline': [
                  {
                    'turn': 1,
                    'seq': 3,
                    'prompt': '帮我把测试环境部署一下',
                    'response': '已经跑完 deploy.sh --env test\n日志见下',
                  },
                ],
              },
            ),
          ),
        ),
      );
      // 用户输入的**回声**不许出现在第二行。
      expect(find.text('帮我把测试环境部署一下'), findsNothing);
      // agent 的**动作**要出现，且只取首行。
      expect(find.text('已经跑完 deploy.sh --env test'), findsOneWidget);
    });

    testWidgets('查不到第二行时不占位，也不拿用户输入填', (tester) async {
      await tester.pumpWidget(
        wrap(
          tile(
            build(
              title: '空会话',
              projections: const {
                'turnOutline': [
                  {'turn': 1, 'seq': 1, 'prompt': '只有提问没有回复', 'response': ''},
                ],
              },
            ),
          ),
        ),
      );
      // 05 §4.3：「查不到 → 空（不占位）」。
      expect(find.text('只有提问没有回复'), findsNothing);
      // 行仍在，只是没有第二行。
      expect(find.byType(SessionTile), findsOneWidget);
    });

    testWidgets('运行中的会话带「运行中」中文词与语义标签', (tester) async {
      await tester.pumpWidget(wrap(tile(build(title: '跑批处理', running: true))));
      // 「绝不靠颜色单独表意」：中文词必须真的画出来。
      expect(find.text('运行中'), findsOneWidget);
      // 语义标签里也要有状态 —— 整行读一遍就够，轨与点自己不再标。
      final sem = tester.getSemantics(find.byType(SessionTile));
      expect(sem.label, contains('运行中'));
    });

    testWidgets('元信息行给出轮次/上下文，且**永不换行**', (tester) async {
      // 用 320dp + 长路径：只有这样，换行的实现才**真的**需要换行。
      // 默认宽度下内容本来就挤得下，Wrap 与 Row 渲染结果一模一样，
      // 任何「看渲染结果」的断言都没有判别力（实测：换成 Wrap 仍全绿）。
      await tester.pumpWidget(
        wrap(
          tile(
            build(
              title: '一个相当长的会话标题用来测试单行截断是否正常工作',
              cwd: '/home/me/a-really-long-directory-name-for-testing',
              projections: const {
                'sessionStats': {'turns': 128},
                'contextPressure': {'projectedTokens': 48600000},
              },
            ),
          ),
          size: const Size(320, 844),
        ),
      );
      expect(find.textContaining('128 轮'), findsOneWidget);
      expect(find.textContaining('48.6M ctx'), findsOneWidget);

      // ⚠️ 真正的不变式是**每一格自己 `maxLines == 1`**。
      //
      // 踩过的坑：先以为外层容器（Row+Expanded vs Wrap）才是防换行的开关，
      // 于是去断「行高仍 66dp」和「各格高度一致」。实测两种断法都**没有
      // 判别力** —— 把外层换成 `Wrap` 后测试照样全绿，因为每一格自带
      // `maxLines: 1`，Wrap 拿到的全是单行子节点，根本没机会换行。
      // 真正的开关在 `Text` 上。
      //
      // 排除标题 / 第二行 / 分隔符 `·`：它们要么本来就单行，要么是单字符。
      final offenders = tester
          .widgetList<Text>(find.descendant(
            of: find.byType(SessionTile),
            matching: find.byType(Text),
          ))
          .where((t) => t.data != '·')
          .where((t) => t.maxLines != 1)
          .map((t) => t.data)
          .toList();
      expect(
        offenders,
        isEmpty,
        reason: '05 §4.4「永不换行」：每一格都必须是 maxLines: 1，'
            '否则它会换行把行撑高。违规：$offenders',
      );
    });

    testWidgets('正文 2.0x 字号下零溢出（05 §9.5 第 4 条）', (tester) async {
      // 上限从 1.3 放宽到 2.0 之后，这条从「愿望」变成「必须」。
      // 04 裁决表：夹取是为了让布局不溢出，不是为了压住无障碍。
      //
      // ⚠️ 这里**不能**只断 `takeException() isNull`：行高改用 minHeight
      // 之后，2.0x 下是「长高」而不是「溢出」，RenderFlex 压根不会报错，
      // 那条断言永远不会失败。真正要断的是**文字没被裁**。
      tester.view.physicalSize = const Size(390 * 3, 844 * 3);
      tester.view.devicePixelRatio = 3.0;
      addTearDown(tester.view.reset);

      await tester.pumpWidget(
        MaterialApp(
          theme: QxTheme.dark(),
          home: Scaffold(
            body: MediaQuery(
              data: const MediaQueryData(textScaler: TextScaler.linear(2.0)),
              child: ListView(
                children: [
                  tile(
                    build(
                      title: '一个相当长的会话标题用来测试放大之后会不会撑破',
                      cwd: '/home/me/very/deep/project',
                      projections: const {
                        'sessionStats': {'turns': 128},
                        'contextPressure': {'projectedTokens': 48600000},
                        'turnOutline': [
                          {
                            'turn': 1,
                            'seq': 1,
                            'prompt': 'p',
                            'response': 'Bash ./deploy.sh --env test --force',
                          },
                        ],
                      },
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      expect(tester.takeException(), isNull, reason: '2.0x 字号下不得溢出');
      // 行必须**长高**（而不是裁内容）：默认 66dp 装不下 2.0x 的三行。
      expect(
        tester.getSize(find.byType(SessionTile)).height,
        greaterThan(SessionTile.rowHeight),
        reason: '内容装不下时行应该长高',
      );
      // 状态词是唯一不许被裁的（见上一条测试）。
      final para = tester.renderObject<RenderParagraph>(
        find.descendant(of: find.text('已完成'), matching: find.byType(RichText)).first,
      );
      expect(para.didExceedMaxLines, isFalse);
    });

    testWidgets('idle 会话可以 settle；运行中**故意**不 settle', (tester) async {
      // 05 §3.1 末尾的取舍说明：默认态（idle 等）树里不得有无限动画；
      // 运行中的彗尾 1.6s 循环是**故意**不 settle 的。
      //
      // 这条测试是那道取舍的锁：它一旦被「修好」（即让运行中也 settle），
      // 意味着有人为了测试方便砍掉了彗星 —— 而运动是唯一无法被灰度截图
      // 与色觉障碍读到的通道，砍它等于把签名级编码降级。
      await tester.pumpWidget(wrap(tile(build(title: 'x'))));
      await tester.pumpAndSettle(const Duration(milliseconds: 100));

      await tester.pumpWidget(wrap(tile(build(title: 'x', running: true))));
      await tester.pump(const Duration(milliseconds: 100));
      var settled = true;
      try {
        await tester.pumpAndSettle(const Duration(milliseconds: 100));
      } on FlutterError {
        settled = false;
      }
      expect(
        settled,
        isFalse,
        reason: '运行中必须有循环彗尾（05 §3.1）。若想让它 settle，'
            '说明有人砍了运动通道 —— 那是设计降级，不是修复',
      );
    });

    testWidgets('点击回调触发一次', (tester) async {
      var taps = 0;
      await tester.pumpWidget(
        wrap(SessionTile(item: presentSession(build(title: 'x')), onTap: () => taps++)),
      );
      await tester.tap(find.text('x'));
      expect(taps, 1);
    });
  });

  group('工具卡', () {
    ToolCallData call() => const ToolCallData(
          turn: 1,
          step: 2,
          callId: 'c1',
          name: 'rg',
          arguments: '{"pattern":"qx-gate"}',
        );

    testWidgets('运行中：静态状态点 + 「运行中」角标，**不转圈**', (tester) async {
      await tester.pumpWidget(wrap(
        ToolCallCard(
          row: RowToolCall(
            seq: 1,
            at: DateTime.now(),
            call: call(),
          ),
        ),
      ));
      expect(find.text('rg'), findsOneWidget);
      expect(find.text('运行中'), findsOneWidget);
      // 05 §8.4：默认态不能有无限动画。转圈是唯一不能被灰度截图读到的
      // 通道，而且会让 pumpAndSettle 永久超时。这条断言原来是
      // `findsOneWidget`（在**要求** spinner 存在），把违规锁成了契约。
      expect(find.byType(CircularProgressIndicator), findsNothing);
      // 静态 StateDot（自带字形）+ 文字角标，两条静态通道。
      expect(find.byType(StateDot), findsOneWidget);
      // 反向保证：整棵树能 settle，说明确实没有在跑的动画。
      await tester.pumpAndSettle(const Duration(milliseconds: 100));
    });

    testWidgets('完成：状态点为静态字形，角标为「已完成」', (tester) async {
      await tester.pumpWidget(wrap(
        ToolCallCard(
          row: RowToolCall(
            seq: 1,
            at: DateTime.now(),
            call: call(),
            result: const ToolResultData(
              turn: 1,
              step: 2,
              message: MessageData(
                id: 'm1',
                role: 'tool',
                content: [],
                source: MessageSource(kind: 'tool', callId: 'c1'),
              ),
            ),
          ),
        ),
      ));
      // 角标文案来自 QxSessionState.done.label（05 §2 的规范用词）
      expect(find.text('已完成'), findsOneWidget);
      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(find.byType(StateDot), findsOneWidget);
    });

    testWidgets('失败：错误色角标 + 错误码可见', (tester) async {
      await tester.pumpWidget(wrap(
        ToolCallCard(
          row: RowToolCall(
            seq: 1,
            at: DateTime.now(),
            call: call(),
            result: const ToolResultData(
              turn: 1,
              step: 2,
              message: MessageData(
                id: 'm1',
                role: 'tool',
                content: [],
                source: MessageSource(kind: 'tool', callId: 'c1'),
              ),
            ),
            error: const ToolErrorRef(name: 'rg', code: 'exit-code-1'),
          ),
        ),
      ));
      expect(find.text('失败'), findsOneWidget);
      // 展开前状态就必须是可读的，不能藏在折叠里。
      expect(find.text('exit-code-1'), findsOneWidget);
    });

    testWidgets('参数与输出默认折叠（地铁上不该读 300 行 grep）', (tester) async {
      await tester.pumpWidget(wrap(
        ToolCallCard(
          row: RowToolCall(seq: 1, at: DateTime.now(), call: call()),
        ),
        size: const Size(390, 3000),
      ));
      expect(find.text('ARGUMENTS'), findsNothing);

      // ⚠️ 不能用 pumpAndSettle：运行中的工具卡有一个**永不停止**的
      // CircularProgressIndicator，动画队列永远不会空，pumpAndSettle 会
      // 直接超时。改用固定时长的 pump 推进折叠动画。
      await tester.tap(find.text('rg'));
      await tester.pump(); // 启动折叠
      await tester.pump(const Duration(milliseconds: 400)); // 走完动画
      expect(find.text('ARGUMENTS'), findsOneWidget);
      expect(find.text('OUTPUT'), findsNothing); // 还没有结果
    });

    testWidgets('折叠标题有墨水反馈（表面由 Material 承载）', (tester) async {
      await tester.pumpWidget(wrap(
        ToolCallCard(
          row: RowToolCall(seq: 1, at: DateTime.now(), call: call()),
        ),
        size: const Size(390, 3000),
      ));
      // ListTile 把背景与涟漪画在**最近的 Material 祖先**上。
      // 中间若隔一个带底色的 Container，涟漪就被整个盖住——点下去毫无
      // 反馈。断言：ExpansionTile 的祖先链上第一个 Material 之后，
      // 不允许再出现带底色的 DecoratedBox。
      final tile = find.byType(ExpansionTile);
      final ancestors =
          tester.elementList(find.ancestor(of: tile, matching: find.byType(Material))).toList();
      expect(ancestors, isNotEmpty);

      // 从 ExpansionTile 往上到它所属的 Material 之间，不能夹 DecoratedBox。
      final mtl = ancestors.first;
      final path = tester.widgetList<Material>(find.byType(Material)).where((_) => true).toList();
      expect(path, isNotEmpty);
      // 该 Material 自身带底色（ sunken ），这正是它能接住涟漪的原因。
      expect(mtl.widget, isA<Material>());
      final material = mtl.widget as Material;
      expect(material.color, isNotNull);
      expect(material.clipBehavior, Clip.antiAlias);
    });
  });

  group('待办卡', () {
    testWidgets('渲染全部条目与完成计数', (tester) async {
      await tester.pumpWidget(wrap(
        const TodoCard(
          todos: [
            TodoItem(content: '读协议', status: 'completed'),
            TodoItem(content: '写客户端', status: 'in_progress'),
            TodoItem(content: '真机验收', status: 'pending'),
          ],
        ),
      ));
      expect(find.text('读协议'), findsOneWidget);
      expect(find.text('写客户端'), findsOneWidget);
      expect(find.text('1/3'), findsOneWidget);
    });
  });

  group('转录行', () {
    testWidgets('用户消息右对齐并带主色底', (tester) async {
      await tester.pumpWidget(wrap(
        TranscriptRowBuilder(
          row: RowUserMessage(
            seq: 1,
            at: DateTime.now(),
            message: const MessageData(
              id: 'm1',
              role: 'user',
              content: [ContentText('跑一下测试')],
              source: MessageSource(kind: 'user'),
            ),
          ),
        ),
      ));
      expect(find.text('跑一下测试'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('助手消息：思维链默认折叠，正文可见', (tester) async {
      await tester.pumpWidget(wrap(
        TranscriptRowBuilder(
          row: RowAssistantMessage(
            seq: 2,
            at: DateTime.now(),
            message: const MessageData(
              id: 'm2',
              role: 'assistant',
              content: [
                ContentReasoning('先看看网关的鉴权逻辑…'),
                ContentText('## 结论\n\n配对链接用的是 `?code=`。'),
              ],
              source: MessageSource(kind: 'model', model: 'deepseek'),
            ),
            usage: const TokenUsage(inputTokens: 100, outputTokens: 2500),
          ),
        ),
      ));
      // 思维链标题可见但内容折叠。
      expect(find.text('思维链'), findsOneWidget);
      expect(find.textContaining('先看看网关'), findsNothing);
      // 正文 markdown 必须渲染成结构化元素而不是一坨纯文本。
      expect(find.textContaining('结论'), findsOneWidget);
      expect(find.text('deepseek'), findsOneWidget);
      expect(find.textContaining('↑2.5k'), findsOneWidget);

      await tester.tap(find.text('思维链'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 400));
      expect(find.textContaining('先看看网关'), findsOneWidget);
    });

    testWidgets('轮次分隔显示 TURN 编号', (tester) async {
      await tester.pumpWidget(wrap(
        TranscriptRowBuilder(
          row: RowTurnDivider(seq: 1, at: DateTime.now(), turn: 7),
        ),
      ));
      expect(find.text('TURN 7'), findsOneWidget);
    });

    testWidgets('长 markdown + 代码块不溢出窄屏', (tester) async {
      await tester.pumpWidget(wrap(
        TranscriptRowBuilder(
          row: RowAssistantMessage(
            seq: 1,
            at: DateTime.now(),
            message: const MessageData(
              id: 'm1',
              role: 'assistant',
              content: [
                ContentText(
                  '这里是一段很长的正文，用来验证窄屏下的换行与裁剪。'
                  '它需要足够长才能触发真实的排版计算，而不是被直接省略。\n\n'
                  '- 列表项一\n- 列表项二\n- 列表项三\n\n'
                  '```dart\nfinal x = someVeryLongFunctionName(argument1, argument2);\n```\n\n'
                  '> 引用块也要在窄屏里正确收缩。\n',
                ),
              ],
              source: MessageSource(kind: 'model'),
            ),
          ),
        ),
      ));
      await tester.pump();
      expect(tester.takeException(), isNull);
    });
  });

  group('转录列表', () {
    testWidgets('空态给出「怎么开始」而不是灰图标', (tester) async {
      await tester.pumpWidget(MaterialApp(
        theme: QxTheme.dark(),
        home: const Scaffold(body: TranscriptView(state: TranscriptState(loading: false))),
      ));
      expect(find.text('这段对话还是空的'), findsOneWidget);
      expect(find.textContaining('发一条消息'), findsOneWidget);
    });

    testWidgets('加载态显示进度而非空白', (tester) async {
      await tester.pumpWidget(MaterialApp(
        theme: QxTheme.dark(),
        home: const Scaffold(body: TranscriptView(state: TranscriptState())),
      ));
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
    });

    testWidgets('多行渲染 + 错误提示排在最后', (tester) async {
      await tester.pumpWidget(MaterialApp(
        theme: QxTheme.dark(),
        home: Scaffold(
          body: TranscriptView(
            state: TranscriptState(
              loading: false,
              error: '与电脑的连接中断了',
              rows: [
                RowTurnDivider(seq: 1, at: DateTime.now(), turn: 1),
                RowUserMessage(
                  seq: 2,
                  at: DateTime.now(),
                  message: const MessageData(
                    id: 'm1',
                    role: 'user',
                    content: [ContentText('第一条')],
                    source: MessageSource(kind: 'user'),
                  ),
                ),
                RowNotice(seq: 3, at: DateTime.now(), label: '命令完成', detail: 'cargo test'),
              ],
            ),
          ),
        ),
      ));
      expect(find.text('TURN 1'), findsOneWidget);
      expect(find.text('第一条'), findsOneWidget);
      expect(find.text('命令完成'), findsOneWidget);
      expect(find.text('与电脑的连接中断了'), findsOneWidget);
    });
  });

  group('格式化工具', () {
    test('相对时间', () {
      final now = DateTime.now().millisecondsSinceEpoch;
      expect(relTime(0), '从未用过');
      expect(relTime(now - 10 * 1000), '刚刚');
      expect(relTime(now - 5 * 60000), '5 分钟前');
      expect(relTime(now - 3 * 3600000), '3 小时前');
      expect(relTime(now - 2 * 86400000), '2 天前');
    });

    test('token 紧凑显示', () {
      expect(formatTokens(999), '999');
      expect(formatTokens(1200), '1.2k');
      expect(formatTokens(48000), '48k');
      expect(formatTokens(2500000), '2.5M');
    });

    test('时长', () {
      expect(formatDuration(const Duration(milliseconds: 240)), '240ms');
      expect(formatDuration(const Duration(seconds: 12)), '12s');
      expect(formatDuration(const Duration(minutes: 3, seconds: 20)), '3m 20s');
    });
  });
}

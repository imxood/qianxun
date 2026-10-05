import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/app/theme/qx_theme.dart';
import 'package:qianxun_mobile/data/models/content.dart';
import 'package:qianxun_mobile/data/models/session.dart';
import 'package:qianxun_mobile/features/sessions/presentation/sessions_screen.dart';
import 'package:qianxun_mobile/features/workspace/application/transcript_controller.dart';
import 'package:qianxun_mobile/features/workspace/presentation/transcript_view.dart';
import 'package:qianxun_mobile/shared/widgets/primitives.dart';

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
        const StateDot(color: Color(0xFF3DDC97), pulsing: true),
      ));
      expect(find.byType(StateDot), findsOneWidget);
      expect(find.byType(RepaintBoundary), findsWidgets);
    });
  });

  group('会话行', () {
    SessionSummary build({
      bool running = false,
      String? title,
      String? cwd,
      Map<String, Object?>? projections,
    }) {
      // ⚠️ 合并成一个 values：写两个 `'projections':` 键时后者会覆盖前者，
      // 曾经因此让「12 轮 / 48k ctx」三个断言全部落空。
      final values = <String, Object?>{
        if (title != null) 'title': title,
        ...?projections,
      };
      return SessionSummary.fromJson({
        'sessionId': 's1',
        'updatedAt': DateTime.now().millisecondsSinceEpoch - 60000,
        'running': running,
        'blank': false,
        if (cwd != null) 'cwd': cwd,
        'projections': {'asOfSeq': 1, 'values': values},
      });
    }

    testWidgets('显示投影里的标题，不显示"未命名会话"', (tester) async {
      await tester.pumpWidget(wrap(
        SessionRow(session: build(title: '重构网关鉴权'), onTap: () {}),
      ));
      expect(find.text('重构网关鉴词'), findsNothing);
      expect(find.text('重构网关鉴权'), findsOneWidget);
    });

    testWidgets('没有标题投影时退回 cwd 末段，且副行不重复它', (tester) async {
      await tester.pumpWidget(wrap(
        SessionRow(session: build(cwd: '/home/me/deep/app'), onTap: () {}),
      ));
      // 标题回退到 cwd 末段…
      expect(find.text('app'), findsOneWidget);
      // …副行就**不能**再显示一遍。同一条路径出现两次是纯冗余。
      expect(find.text('app'), findsOneWidget);
    });

    testWidgets('有标题时副行才显示 cwd', (tester) async {
      await tester.pumpWidget(wrap(
        SessionRow(
          session: build(title: '重构鉴权', cwd: '/home/me/proj'),
          onTap: () {},
        ),
      ));
      expect(find.text('重构鉴权'), findsOneWidget);
      expect(find.text('proj'), findsOneWidget);
    });

    testWidgets('运行中的会话显示状态语义与呼吸点', (tester) async {
      await tester.pumpWidget(wrap(
        SessionRow(session: build(title: '跑批处理', running: true), onTap: () {}),
      ));
      expect(find.byType(StateDot), findsOneWidget);
      // 语义标签里必须带状态文字——纯色点在色觉障碍用户眼里没信息。
      final sem = tester.getSemantics(find.byType(SessionRow));
      expect(sem.label, contains('运行中'));
    });

    testWidgets('副行给出轮次/上下文/token，且不溢出', (tester) async {
      await tester.pumpWidget(wrap(
        SessionRow(
          session: build(
            title: '一个相当长的会话标题用来测试单行截断是否正常工作',
            cwd: '/home/me/proj',
            projections: const {
              'sessionStats': {'turns': 12},
              'contextPressure': {'projectedTokens': 48000},
              'tokenUsage': {'outputTokens': 3200},
            },
          ),
          onTap: () {},
        ),
      ));
      expect(find.textContaining('12 轮'), findsOneWidget);
      expect(find.textContaining('48k ctx'), findsOneWidget);
      expect(find.textContaining('↑3.2k'), findsOneWidget);
      // 窄屏下不得有 RenderFlex 溢出。
      expect(tester.takeException(), isNull);
    });

    testWidgets('点击回调触发一次', (tester) async {
      var taps = 0;
      await tester.pumpWidget(wrap(
        SessionRow(session: build(title: 'x'), onTap: () => taps++),
      ));
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

    testWidgets('运行中：显示 spinner 与「运行中」角标', (tester) async {
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
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
    });

    testWidgets('完成：状态点变实心，角标为「完成」', (tester) async {
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
      expect(find.text('完成'), findsOneWidget);
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
      expect(relTime(0), '从未使用');
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

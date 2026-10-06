import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/app/theme/qx_test_ids.dart';
import 'package:qianxun_mobile/data/models/session.dart';
import 'package:qianxun_mobile/features/sessions/application/session_presentation.dart';
import 'package:qianxun_mobile/features/sessions/presentation/session_tile.dart';

/// 自动化测试 id 的契约。
///
/// 这些 id 是 uiautomator2 脚本唯一的锚点。它们的失败模式是**完全无声**
/// 的：id 拼错时脚本只是「找不到元素」，编译期和运行期都不报错。所以必须
/// 在这里钉住。
void main() {
  Widget wrap(Widget child) =>
      MaterialApp(home: Scaffold(body: Center(child: child)));

  group('测试 id 的形状', () {
    test('全部是 snake_case，且带 qx_ 前缀', () {
      // 脚本里手写这些字符串，前缀让它在 uiautomator dump 的一堆
      // resource-id 里一眼可辨（别人的 id 通常是 com.xxx:id/btn_xxx）。
      final bad = QxTestIds.all
          .where((id) => !RegExp(r'^qx_[a-z0-9_]+$').hasMatch(id))
          .toList();
      expect(bad, isEmpty, reason: '不符合 qx_ + snake_case：$bad');
    });

    test('没有重复', () {
      expect(QxTestIds.all.toSet().length, QxTestIds.all.length);
    });

    test('不依赖 widget 类名（重命名 widget 不该弄坏脚本）', () {
      // 约定：id 描述「它在自动化里是什么」，不是「它叫什么类」。
      // SessionTile → sessionTile 这种耦合会让每次重构都变成脚本事故。
      expect(QxTestIds.sessionTile, 'qx_session_tile');
      expect(QxTestIds.sendButton, 'qx_send_button');
    });
  });

  group('id 真的透到了语义树', () {
    testWidgets('会话行带 qx_session_tile', (tester) async {
      // ⚠️ 这条是本文件的核心：Semantics(identifier:) 只有真的挂到树上，
      // uiautomator 才可能看见它。
      //
      // 注意 verify 的是**语义节点上的 identifier**，不是「源码里写了」。
      // 写成后者的话，把 Semantics 整个删掉它照样绿。
      final handle = tester.ensureSemantics();
      await tester.pumpWidget(wrap(SessionTile(
        item: presentSession(
          SessionSummary.fromJson({
            'sessionId': 's1',
            'updatedAt': 0,
            'running': false,
            'blank': false,
            'projections': {
              'asOfSeq': 1,
              'values': {'title': '某条会话'},
            },
          }),
        ),
        onTap: () {},
      )));

      final node = tester.getSemantics(find.byType(SessionTile));
      expect(
        node.getSemanticsData().identifier,
        QxTestIds.sessionTile,
        reason: 'uiautomator2 靠这个 id 定位会话行',
      );
      handle.dispose();
    });

    testWidgets('identifier 不影响朗读内容（无障碍不能因此退化）', (tester) async {
      // 两条独立的诉求：identifier 给自动化，label 给读屏。
      // 加 identifier 很容易顺手把 label 弄丢，那会让 TalkBack 读不出状态。
      final handle = tester.ensureSemantics();
      await tester.pumpWidget(wrap(SessionTile(
        item: presentSession(SessionSummary.fromJson({
          'sessionId': 's1',
          'updatedAt': 0,
          'running': true,
          'blank': false,
          'projections': {
            'asOfSeq': 1,
            'values': {'title': '跑批处理'},
          },
        })),
        onTap: () {},
      )));

      final data = tester.getSemantics(find.byType(SessionTile)).getSemanticsData();
      expect(data.identifier, QxTestIds.sessionTile);
      expect(data.label, contains('跑批处理'));
      expect(data.label, contains('运行中'), reason: '状态词必须能被读屏读出');
      handle.dispose();
    });

    // ⚠️ 这里**故意没有** PendingInteractionBar 的测试。
    // 待答条的 id 属 Batch 5，现在还没接线；而一条「组件能渲染出来」的
    // 测试不验证任何不变量，只会让人以为那一块已经可以自动化了。
    // 哪些 id 还没接线，由下面那组 `unwired` 如实记账。
  });

  group('未被使用的 id 是欠债', () {
    test('qx_sessions_list / qx_new_session_button 已接上', () {
      // 这两个在 Batch 3 的会话列表页上。列表与新建按钮是最先被脚本
      // 定位的两个东西（冷启动冒烟的起点）。
      expect(QxTestIds.all, contains(QxTestIds.sessionsList));
      expect(QxTestIds.all, contains(QxTestIds.newSessionButton));
    });

    test('明确标出哪些 id 还没接线', () {
      // 诚实记账：`all` 里有些 id 现在还没有对应的 Semantics
      // （转录、输入栏属 Batch 4；待答属 Batch 5）。把它们写下来，
      // 免得「以为已经能用」——自动化脚本找不到 id 时最容易这么想。
      const wired = {
        QxTestIds.sessionTile,
        QxTestIds.sessionsList,
        QxTestIds.newSessionButton,
      };
      final unwired = QxTestIds.all.where((id) => !wired.contains(id)).toList();
      expect(
        unwired,
        isNotEmpty,
        reason: '本批只接了会话列表相关的 id；剩下的等 Batch 4/5，'
            '别以为它们现在就能被脚本定位',
      );
    });
  });
}

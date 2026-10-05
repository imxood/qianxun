import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/app/theme/qx_test_ids.dart';
import 'package:qianxun_mobile/app/theme/qx_theme.dart';
import 'package:qianxun_mobile/data/dsh/rpc_envelope.dart';
import 'package:qianxun_mobile/features/workspace/application/composer_controller.dart';
import 'package:qianxun_mobile/features/workspace/presentation/composer_bar.dart';

/// 输入栏（附件 + steer + 发送）。
///
/// 这个文件的存在理由写在 `ComposerBar` 的注释里：旧版输入栏是个裸
/// `TextEditingController`，`ComposerController` 整块逻辑**没有任何 UI
/// 调用方** —— 于是「支持图片与文件的发送」在界面上不可达。
///
/// 所以这里测的不是「某个 widget 长什么样」，而是**产品承诺**：
/// 1. 有挂附件的入口
/// 2. 每个附件的态如实显示（上传中的文件不能让用户以为已经就绪）
/// 3. 附件没就绪时**不许**发送（静默丢数据）
/// 4. agent 在跑时能给 steer 切换
/// 5. 自动化 id 真的挂上去了
void main() {
  Future<void> pumpBar(
    WidgetTester tester, {
    ComposerState state = const ComposerState(),
    bool running = false,
  }) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          composerControllerProvider('s1').overrideWith((ref) {
            final c = _FakeComposer(ref, 's1');
            c.seed(state);
            return c;
          }),        ],
        child: MaterialApp(
          theme: QxTheme.dark(),
          home: Scaffold(
            body: ComposerBar(sessionId: 's1', running: running, onSent: () {}),
          ),
        ),
      ),
    );
    await tester.pump();
  }

  group('附件入口', () {
    testWidgets('有「添加附件」入口，且带自动化 id', (tester) async {
      final handle = tester.ensureSemantics();
      await pumpBar(tester);
      expect(find.byIcon(Icons.add_circle_outline), findsOneWidget);
      final id = tester.getSemantics(find.bySemanticsLabel('添加附件'));
      expect(id.getSemanticsData().identifier, QxTestIds.attachButton);
      handle.dispose();
    });
  });

  group('附件的态必须如实显示', () {
    testWidgets('图片挂上就是「已就绪」（它根本不上传）', (tester) async {
      final handle = tester.ensureSemantics();
      await pumpBar(
        tester,
        state: const ComposerState(
          attachments: [
            PendingAttachment(
              localId: 'a1',
              name: 'shot.png',
              bytes: 2048,
              isImage: true,
              data: 'iVBORw0KGgo=',
              state: AttachState.ready,
            ),
          ],
        ),
      );
      // 05 §4.3 与 probe_upload.py：图片内联 base64，不走上传。
      expect(find.textContaining('已就绪'), findsOneWidget);
      handle.dispose();
    });

    testWidgets('文件上传中必须显示「上传中」，不能装作好了', (tester) async {
      final handle = tester.ensureSemantics();
      await pumpBar(
        tester,
        state: const ComposerState(
          attachments: [
            PendingAttachment(
              localId: 'a1',
              name: 'report.pdf',
              bytes: 4096,
              isImage: false,
              state: AttachState.uploading,
            ),
          ],
        ),
      );
      expect(find.textContaining('上传中'), findsOneWidget);
      expect(find.textContaining('已就绪'), findsNothing);
      handle.dispose();
    });

    testWidgets('上传失败要显示「失败」', (tester) async {
      final handle = tester.ensureSemantics();
      await pumpBar(
        tester,
        state: const ComposerState(
          attachments: [
            PendingAttachment(
              localId: 'a1',
              name: 'report.pdf',
              bytes: 4096,
              isImage: false,
              state: AttachState.failed,
              error: '网络断了',
            ),
          ],
        ),
      );
      expect(find.textContaining('失败'), findsOneWidget);
      handle.dispose();
    });
  });

  group('不许静默丢数据', () {
    test('附件没就绪时 canSend 为 false', () {
      // 05 §4.3 与 ComposerState 的注释：放行 pending 是**静默丢数据** ——
      // send() 只挑 ready 的进 content，pending 会被无声地不带上，
      // 用户以为「图和文字一起发出去了」，实际只到了文字。
      const uploading = PendingAttachment(
        localId: 'a1',
        name: 'big.bin',
        bytes: 10,
        isImage: false,
        state: AttachState.uploading,
      );
      const ready = PendingAttachment(
        localId: 'a1',
        name: 'ok.png',
        bytes: 10,
        isImage: true,
        data: 'x',
        state: AttachState.ready,
      );
      expect(
        const ComposerState(attachments: [uploading]).canSend,
        isFalse,
        reason: '上传中绝不能放行',
      );
      expect(const ComposerState(attachments: [ready]).canSend, isTrue);
      expect(
        const ComposerState(
          attachments: [ready, uploading],
        ).canSend,
        isFalse,
        reason: '只要有一个没就绪就不能发',
      );
    });

    test('失败态的附件也不能发', () {
      const failed = PendingAttachment(
        localId: 'a1',
        name: 'x.bin',
        bytes: 1,
        isImage: false,
        state: AttachState.failed,
      );
      expect(const ComposerState(attachments: [failed]).canSend, isFalse);
    });
  });

  group('steer 切换', () {
    testWidgets('agent 不在跑时不显示（没有可插进去的轮次）', (tester) async {
      await pumpBar(tester, running: false);
      expect(find.byType(Switch), findsNothing);
    });

    testWidgets('agent 在跑时出现，且两种说法都带中文词', (tester) async {
      final handle = tester.ensureSemantics();
      await pumpBar(tester, running: true);
      expect(find.byType(Switch), findsOneWidget);
      // 绝不只靠开关的颜色/位置表意
      expect(find.text('排队等这轮跑完'), findsOneWidget);
      expect(
        tester.getSemantics(find.byType(Switch)).getSemanticsData().label,
        contains('排队'),
      );
      handle.dispose();
    });

    testWidgets('steer 打开后文案换成「插进当前轮次」', (tester) async {
      final handle = tester.ensureSemantics();
      await pumpBar(
        tester,
        running: true,
        state: const ComposerState(steer: true),
      );
      expect(find.text('插进当前轮次'), findsOneWidget);
      expect(
        tester.getSemantics(find.byType(Switch)).getSemanticsData().label,
        contains('插进当前轮次'),
      );
      handle.dispose();
    });
  });

  group('自动化 id', () {
    testWidgets('输入框与发送键都挂了 id', (tester) async {
      final handle = tester.ensureSemantics();
      await pumpBar(tester);
      expect(
        find.bySemanticsIdentifier(QxTestIds.composerInput),
        findsOneWidget,
      );
      final send = tester.getSemantics(
        find.bySemanticsIdentifier(QxTestIds.sendButton),
      );
      expect(send.getSemanticsData().identifier, QxTestIds.sendButton);
      expect(send.getSemanticsData().flagsCollection.isButton, isTrue);
      handle.dispose();
    });

    testWidgets('发送键的语义标签说明它是做什么的', (tester) async {
      // 自动化按 description 定位（Flutter 的文字都在 content-desc 上），
      // 所以标签必须是人能读懂的动作词，不是类名。
      final handle = tester.ensureSemantics();
      await pumpBar(tester);
      final sem = tester
          .getSemantics(find.bySemanticsIdentifier(QxTestIds.sendButton))
          .getSemanticsData();
      expect(sem.label, anyOf('发送', '发送中'));
      handle.dispose();
    });
  });
}

/// 假的输入栏控制器。
///
/// ⚠️ 继承 `ComposerController` 而不是另写一个 `StateNotifier`：
/// `overrideWith` 的签名要求返回**那个类型**。
/// 基类构造函数会 `ref.read(selectedConnectionProvider)` —— 在
/// ProviderScope 里它返回 null，于是 `origin` 退化成空串、`token` 为 null。
/// 这些测试只验 UI 的**呈现**，从不真的发网络请求，所以这样是安全的。
class _FakeComposer extends ComposerController {
  _FakeComposer(super.ref, super.sessionId);

  /// 灌入初始状态。基类构造函数固定用 `const ComposerState()`，
  /// 而 overrideWith 的签名又不允许在构造时传状态，所以只能事后 seed。
  void seed(ComposerState s) => state = s;

  @override
  void setSteer(bool steer) => state = state.copyWith(steer: steer);

  @override
  void clearError() => state = state.copyWith(clearError: true);

  @override
  void removeAttachment(String localId) => state = state.copyWith(
        attachments:
            state.attachments.where((a) => a.localId != localId).toList(),
      );

  @override
  Future<void> attachImage({
    required String name,
    required Uint8List bytes,
  }) async {}

  @override
  Future<void> attachFile(File file) async {}

  @override
  Future<RpcResult<Object?>> send(String text) async =>
      const RpcOk<Object?>(null);
}

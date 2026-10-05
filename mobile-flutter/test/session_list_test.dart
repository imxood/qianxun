import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/app/theme/qx_semantic.dart';
import 'package:qianxun_mobile/data/models/session.dart';
import 'package:qianxun_mobile/features/sessions/application/session_presentation.dart';

/// 会话列表的状态推导与排序。
///
/// 这里测的是 05 §2 / §3.3 的**判定规则本身**，不碰 widget。
/// 理由：状态判定一旦被「顺手加个 if」污染，UI 测试是发现不了的 ——
/// UI 只会忠实地把错误的态渲染出来。
void main() {
  SessionSummary s(
    String id, {
    bool running = false,
    bool? agentAvailable,
    bool awaiting = false,
    String? response,
    int? updatedAt,
  }) {
    return SessionSummary.fromJson({
      'sessionId': id,
      'updatedAt': updatedAt ?? 1000,
      'running': running,
      'blank': false,
      if (agentAvailable != null) 'agentAvailable': agentAvailable,
      if (awaiting) 'userQuestions': {'active': [{'id': 'q1'}]},
      'projections': {
        'asOfSeq': 1,
        'values': {
          if (response != null)
            'turnOutline': [
              {'turn': 1, 'seq': 1, 'prompt': '用户说的话', 'response': response},
            ],
        },
      },
    });
  }

  PendingApproval approvalFor(String sessionId) => PendingApproval(
        eventId: 'e1',
        agentId: sessionId,
        request: const ApprovalRequest(toolName: 'Bash', callId: 'c1'),
      );

  group('五态判定：每一态都要有真实数据源', () {
    test('运行中 ← running', () {
      expect(presentSession(s('a', running: true)).state, QxSessionState.running);
    });

    test('等你 ← userQuestions.active 非空', () {
      expect(presentSession(s('a', awaiting: true)).state, QxSessionState.waiting);
    });

    test('等你 ← 本地 waterfall 里有它的待答', () {
      final p = presentSession(s('a'), localPending: [approvalFor('a')]);
      expect(p.state, QxSessionState.waiting);
    });

    test('已断开 ← agentAvailable 为 false', () {
      final p = presentSession(s('a', agentAvailable: false, running: true));
      expect(p.state, QxSessionState.detached);
    });

    test('已完成 ← 兜底', () {
      expect(presentSession(s('a')).state, QxSessionState.done);
    });

    test('列表里**不出现**失败态：它没有数据源（05 §11）', () {
      // 真实数据源是 `turn/end` 事件的 `reason.kind`，而那是**事件流**里的
      // 东西，`session/list` 的快照里没有轮次事件。
      //
      // ⚠️ 这里曾经用「回复开头是不是 `error:`」来判，那是在猜：一次正常
      // 完成的轮次，agent 完全可以回「Error: ECONNREFUSED 127.0.0.1:5432」
      //（它在告诉你数据库连不上），于是那条会话被标成「失败」并排到所有
      // 健康会话之上。凭空捏造的状态，代价是信任额度。
      // 要让列表也显示失败，需要服务端在 `session/list` 里带「上一轮结果」
      // 投影 —— 那是一次协议变更，得先探针确认。
      for (final r in [
        'error: 编译失败',
        'Error: ECONNREFUSED 127.0.0.1:5432\ndb is down',
        'aborted: 用户中断',
        '一切正常',
      ]) {
        expect(
          presentSession(s('a', response: r)).state,
          isNot(QxSessionState.failed),
          reason: '回复正文不是失败的数据源：「$r」',
        );
      }
    });
  });

  group('判定顺序：顺序本身就是规则', () {
    test('已断开压过一切：agent 进不去时不该说它在等你', () {
      // 投影说「在等人」，但 agent 已经不可用。显示「等你」会让用户
      // 点进去发现根本进不去 —— 05 §2 说的「凭空捏造的状态」。
      final p = presentSession(
        s('a', agentAvailable: false, awaiting: true),
        localPending: [approvalFor('a')],
      );
      expect(p.state, QxSessionState.detached);
    });

    test('实时优先于快照：本地 waterfall 存在时不用投影', () {
      // 04 C10 的残留风险：两路来源可能不一致，判决是实时优先。
      final p = presentSession(s('a'), localPending: [approvalFor('a')]);
      expect(p.state, QxSessionState.waiting);
      // 反向：本地没有、投影说等，才轮到投影。
      expect(presentSession(s('a', awaiting: true)).state, QxSessionState.waiting);
    });

    test('运行中压过失败：正在重试的会话不该显示成失败', () {
      // 失败之后 agent 很可能立刻起下一轮。把失败排在运行中前面会让
      // 用户看到一个显示「失败」、实际正在干活的会话。
      final p = presentSession(s('a', running: true, response: 'error: 炸了'));
      expect(p.state, QxSessionState.running);
    });

    test('待答归属靠 agentId，不串台', () {
      // agentTeam.members[].id 就是 sessionId（已实测）。
      final mine = presentSession(s('a'), localPending: [approvalFor('b')]);
      expect(mine.state, isNot(QxSessionState.waiting));
    });
  });

  group('第二行：agent 最近做的那件事（05 §4.3）', () {
    test('取最近一轮回复的首行，不取整段', () {
      final p = presentSession(s('a', response: '跑了 deploy.sh\n然后看了日志'));
      expect(p.secondLine, '跑了 deploy.sh');
    });

    test('永远不是用户自己说过的话', () {
      // 同一个 turn 里 prompt 是用户输入、response 是 agent 输出。
      // 第二行只能取后者。
      final p = presentSession(s('a', response: '已完成'));
      expect(p.secondLine, isNot(contains('用户说的话')));
    });

    test('待答时用工具名（它现在正卡在这个动作上）', () {
      final p = presentSession(
        s('a'),
        localPending: [approvalFor('a')],
      );
      expect(p.secondLine, 'Bash');
    });

    test('查不到就空，不占位', () {
      // 05 §4.3：「查不到 → 空（不占位）」。
      expect(presentSession(s('a')).secondLine, isNull);
    });
  });

  group('协议容错：userQuestions 两个嵌套层级都要读得到', () {
    // ⚠️ 本批**没有**网关 token，探针跑不了，`userQuestions` 到底挂在
    // 会话项顶层还是 `projections.values` 里未经实测。解析因此两头都读。
    // 这条测试锁住的是「两头都能用」——它不是确认协议形状，而是保证
    // 猜错一层时「等你」态不会整个失效（那是本 App 的第二签名级元素）。
    // 探针一旦确认，把 fallback 删掉的同时删掉这条测试。
    test('顶层 userQuestions 生效', () {
      final m = SessionSummary.fromJson({
        'sessionId': 'a',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'userQuestions': {
          'active': [
            {'id': 'q1'},
          ],
        },
        'projections': {'asOfSeq': 1, 'values': {}},
      });
      expect(m.awaitsUser, isTrue);
    });

    test('投影里的 userQuestions 同样生效', () {
      final m = SessionSummary.fromJson({
        'sessionId': 'a',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'projections': {
          'asOfSeq': 1,
          'values': {
            'userQuestions': {
              'active': [
                {'id': 'q1'},
              ],
            },
          },
        },
      });
      expect(m.awaitsUser, isTrue);
    });

    test('两处都没有时不算在等人（不凭空造状态）', () {
      final m = SessionSummary.fromJson({
        'sessionId': 'a',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'projections': {'asOfSeq': 1, 'values': {}},
      });
      expect(m.awaitsUser, isFalse);
      expect(presentSession(m).state, QxSessionState.done);
    });

    test('active 为空数组时不算在等人', () {
      final m = SessionSummary.fromJson({
        'sessionId': 'a',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'userQuestions': {'active': <Object?>[], 'settled': <Object?>[]},
        'projections': {'asOfSeq': 1, 'values': {}},
      });
      expect(m.awaitsUser, isFalse);
    });

    test('顶层给空对象时**不**去读投影里过期的快照', () {
      // ⚠️ 这条锁的是「按键存在」而不是「map 非空」。用 isNotEmpty 判的话，
      // 顶层 `{}` 会让代码去读投影 —— 而投影里可能还留着上一轮那条**已经
      // 解决掉**的提问，于是误报「等你」，把一条没人管的会话顶到最前面。
      // 误报「等你」的代价比漏报大得多：它会浪费用户唯一一次注意力。
      final m = SessionSummary.fromJson({
        'sessionId': 'a',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'userQuestions': <String, Object?>{},
        'projections': {
          'asOfSeq': 1,
          'values': {
            'userQuestions': {
              'active': [
                {'id': 'stale'},
              ],
            },
          },
        },
      });
      expect(m.awaitsUser, isFalse, reason: '顶层已清空，不该回退到过期投影');
    });
  });

  group('排序：等你永远最前（05 §3.3）', () {    test('等你的会话排到第一位', () {
      final out = sortForList([
        presentSession(s('done1', updatedAt: 9000)),
        presentSession(s('run', running: true, updatedAt: 8000)),
        presentSession(s('wait', awaiting: true, updatedAt: 1)),
      ]);
      expect(out.first.session.sessionId, 'wait');
    });

    test('本地 waterfall 到达后立刻置顶', () {
      // 这是「第二签名级元素」的兑现路径：它不是靠重拉列表，
      // 而是靠 provider 重建时重新推导。
      final before = sortForList([
        presentSession(s('a', updatedAt: 9000)),
        presentSession(s('b', updatedAt: 8000)),
      ]);
      expect(before.first.session.sessionId, 'a');

      final after = sortForList([
        presentSession(s('a', updatedAt: 9000)),
        presentSession(s('b', updatedAt: 8000), localPending: [approvalFor('b')]),
      ]);
      expect(after.first.session.sessionId, 'b');
    });

    test('同优先级内按最近更新倒序', () {
      final out = sortForList([
        presentSession(s('old', updatedAt: 1)),
        presentSession(s('new', updatedAt: 9999)),
        presentSession(s('mid', updatedAt: 500)),
      ]);
      expect(out.map((e) => e.session.sessionId), ['new', 'mid', 'old']);
    });

    test('五态的优先级严格递减', () {
      final order = [
        QxSessionState.waiting,
        QxSessionState.running,
        QxSessionState.failed,
        QxSessionState.done,
        QxSessionState.detached,
      ];
      final scores = order.map(qxStatePriority).toList();
      for (var i = 0; i < scores.length - 1; i++) {
        expect(
          scores[i],
          greaterThan(scores[i + 1]),
          reason: '${order[i]} 必须比 ${order[i + 1]} 更靠前',
        );
      }
    });
  });
}

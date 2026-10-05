import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/data/dsh/dsh_client.dart';
import 'package:qianxun_mobile/data/dsh/pair_url.dart';
import 'package:qianxun_mobile/data/dsh/rpc_envelope.dart';
import 'package:qianxun_mobile/data/models/content.dart';
import 'package:qianxun_mobile/data/models/session.dart';

void main() {  group('配对链接', () {
    test('认当前格式的 ?code=', () {
      final pair = PairUrl.tryParse('http://192.168.1.9:23090/qx-gate?code=${'a' * 32}');
      expect(pair, isNotNull);
      expect(pair!.port, 23090);
      expect(pair.code, 'a' * 32);
      expect(pair.token, isNull);
      // ?code= 是一次性的，不能用于 info 探测。
      expect(pair.infoUri(), isNull);
    });

    test('认旧格式的 ?token=', () {
      final pair = PairUrl.tryParse('http://10.0.0.5:17400/qx-gate?token=${'f' * 64}');
      expect(pair, isNotNull);
      expect(pair!.token, 'f' * 64);
      // 长期 token 可以直接挂 query 探测。
      expect(pair.infoUri()!.queryParameters['token'], 'f' * 64);
    });

    test('拒绝空主机名（桌面端未开启远程访问时的形态）', () {
      expect(PairUrl.tryParse('http://:23090/qx-gate?code=${'a' * 32}'), isNull);
    });

    test('拒绝非网关路径与缺失凭据', () {
      expect(PairUrl.tryParse('http://1.2.3.4:23090/'), isNull);
      expect(PairUrl.tryParse('http://1.2.3.4:23090/qx-gate'), isNull);
      expect(PairUrl.tryParse('https://1.2.3.4:23090/qx-gate?code=${'a' * 32}'), isNull);
      expect(PairUrl.tryParse('随便一段文字'), isNull);
    });

    test('归一化后保留凭据', () {
      final pair = PairUrl.tryParse('  http://192.168.1.9:23090/qx-gate?code=${'b' * 32}  ')!;
      expect(pair.normalized, 'http://192.168.1.9:23090/qx-gate?code=${'b' * 32}');
    });
  });

  group('cookie 解析', () {
    test('从 Set-Cookie 里取出 qx_token', () {
      final headers = [
        'qx_token=abc123; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax',
      ];
      expect(parseQxTokenCookie(headers), 'abc123');
    });

    test('多个 cookie 时取对的那个', () {
      final headers = ['other=1; Path=/', 'qx_token=deadbeef; Path=/; HttpOnly'];
      expect(parseQxTokenCookie(headers), 'deadbeef');
    });

    test('没有 qx_token 时返回 null', () {
      expect(parseQxTokenCookie(['foo=bar; Path=/']), isNull);
      expect(parseQxTokenCookie([]), isNull);
    });
  });

  group('RPC 信封', () {
    test('解析成功结果', () {
      final res = parseServerResponse(jsonDecode(
        '{"type":"server-response","rpcId":"r1","result":{"ok":true,"value":{"sessionId":"s1"}}}',
      ));
      expect(res.isOk, isTrue);
      expect(asMap(res.valueOrNull)['sessionId'], 's1');
    });

    test('ok:true 但没有 value 视为空对象（void 成功）', () {
      final res = parseServerResponse(
        jsonDecode('{"type":"server-response","rpcId":"r1","result":{"ok":true}}'),
      );
      expect(res.isOk, isTrue);
      expect(res.valueOrNull, isEmpty);
    });

    test('ok:false 是业务失败，不是传输失败', () {
      final res = parseServerResponse(jsonDecode(
        '{"type":"server-response","rpcId":"r1","result":{"ok":false,'
        '"error":{"code":"not-found","message":"没有这个会话","details":{}}}}',
      ));
      expect(res.isOk, isFalse);
      expect(res.errorOrNull!.code, 'not-found');
      expect(res.errorOrNull!.message, '没有这个会话');
      // 业务失败不该被当成「重新配对」。
      expect(res.errorOrNull!.needsRepairing, isFalse);
    });

    test('信封类型不符时当作传输层损坏', () {
      final res = parseServerResponse(jsonDecode('{"type":"client-request"}'));
      expect(res.isOk, isFalse);
      expect(res.errorOrNull!.code, RpcErrorCode.internal);
    });

    test('401 归类为需要重新配对', () {
      final err = RpcError.fromStatus(401, '');
      expect(err.needsRepairing, isTrue);
      expect(err.isRetryable, isFalse);
    });

    test('502 归类为可重试的上游问题', () {
      final err = RpcError.fromStatus(502, '');
      expect(err.code, RpcErrorCode.upstream);
      expect(err.isRetryable, isTrue);
      expect(err.needsRepairing, isFalse);
    });

    test('非 JSON body 不炸', () {
      final res = parseServerResponse(tryDecodeJson('<html>502 Bad Gateway</html>'));
      expect(res.isOk, isFalse);
    });
  });

  group('内容块', () {
    test('文本块', () {
      final block = ContentBlock.fromJson({'type': 'text', 'text': '你好'});
      expect(block, isA<ContentText>());
      expect(block.plainText, '你好');
    });

    test('工具调用块的 arguments 保持未解析字符串', () {
      final block = ContentBlock.fromJson({
        'type': 'tool-call',
        'id': 'c1',
        'name': 'read',
        'arguments': '{"path":"a.rs"}',
      });
      expect(block, isA<ContentToolCall>());
      // 线格式就是字符串，不是 map——这是最容易写错的一处。
      expect((block as ContentToolCall).arguments, '{"path":"a.rs"}');
    });

    test('未知块不丢，原样保留', () {
      final block = ContentBlock.fromJson({'type': 'brand-new', 'x': 1});
      expect(block, isA<ContentBlockUnknown>());
      expect((block as ContentBlockUnknown).raw['x'], 1);
    });
  });

  group('tool/result 双格式', () {
    test('v3：从 content 块里读', () {
      final data = ToolResultData.fromJson({
        'turn': 1,
        'step': 2,
        'message': {
          'id': 'm1',
          'role': 'user',
          'source': {'kind': 'tool', 'callId': 'c1'},
          'content': [
            {
              'type': 'tool-result',
              'toolCallId': 'c1',
              'isError': true,
              'content': [
                {'type': 'text', 'text': 'boom'}
              ],
            }
          ],
        },
      });
      final result = data.message.content.single as ContentToolResult;
      expect(result.toolCallId, 'c1');
      expect(result.isError, isTrue);
      expect(result.content.single.plainText, 'boom');
    });

    test('v4：从消息顶层读（0.1.7+）', () {
      final data = ToolResultData.fromJson({
        'turn': 1,
        'step': 2,
        'message': {
          'id': 'm1',
          'role': 'tool',
          'source': {'kind': 'tool', 'callId': 'c2'},
        },
        'toolCallId': 'c2',
        'isError': false,
        'content': [
          {'type': 'text', 'text': 'ok'}
        ],
      });
      final result = data.message.content.single as ContentToolResult;
      expect(result.toolCallId, 'c2');
      expect(result.isError, isFalse);
      expect(result.content.single.plainText, 'ok');
    });
  });

  group('会话摘要', () {
    test('标题只能从投影里取', () {
      final s = SessionSummary.fromJson({
        'sessionId': 's1',
        'updatedAt': 1730000000000,
        'running': true,
        'blank': false,
        'cwd': '/home/me/proj',
        'projections': {
          'asOfSeq': 10,
          'values': {'title': '重构网关鉴权', 'agentPreset': 'reviewer'},
        },
      });
      expect(s.title, '重构网关鉴权');
      expect(s.displayTitle, '重构网关鉴权');
      // agentPreset 顶层缺失时从投影兜底。
      expect(s.agentPreset, 'reviewer');
      expect(s.running, isTrue);
    });

    test('没有标题投影时退回 cwd 末段', () {
      final s = SessionSummary.fromJson({
        'sessionId': 's2',
        'updatedAt': 0,
        'running': false,
        'blank': true,
        'cwd': '/home/me/deep/app',
      });
      expect(s.title, isNull);
      expect(s.displayTitle, 'app');
    });

    test('统计与上下文投影', () {
      final s = SessionSummary.fromJson({
        'sessionId': 's3',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'projections': {
          'asOfSeq': 1,
          'values': {
            'sessionStats': {'turns': 3, 'steps': 7},
            'tokenUsage': {'outputTokens': 1500},
            'contextPressure': {'projectedTokens': 12000},
          },
        },
      });
      expect(s.stats!.turns, 3);
      expect(s.tokenUsage!.outputTokens, 1500);
      expect(s.contextTokens, 12000);
    });
  });

  group('事件折叠', () {
    test('user/message 折成用户行', () {
      final view = SessionEventView.fromWire(SessionWireEvent.fromJson({
        'type': 'user/message',
        'seq': 1,
        'time': 1730000000000,
        'data': {
          'id': 'm1',
          'role': 'user',
          'content': [
            {'type': 'text', 'text': '跑一下测试'}
          ],
          'source': {'kind': 'user'},
        },
      }));
      expect(view, isA<UserMessageEvent>());
      expect((view as UserMessageEvent).message.plainText, '跑一下测试');
    });

    test('session/title 折成通知（未建模但 UI 必须用）', () {
      final view = SessionEventView.fromWire(SessionWireEvent.fromJson({
        'type': 'session/title',
        'seq': 2,
        'time': 0,
        'data': {'title': '新名字'},
      }));
      expect(view, isA<SessionTitleEvent>());
      expect((view as SessionTitleEvent).title, '新名字');
    });

    test('未知事件类型保留，不丢', () {
      final view = SessionEventView.fromWire(SessionWireEvent.fromJson({
        'type': 'some/future-event',
        'seq': 3,
        'time': 0,
        'data': {'x': 1},
      }));
      expect(view, isA<UnknownSessionEvent>());
      expect(view.wire.data['x'], 1);
    });
  });

  group('提问回答', () {
    test('custom 挂在每一条上，空串视为未填', () {
      const withText = QuestionAnswerItem(id: 'q1', selected: ['A'], custom: '我的想法');
      expect(withText.toJson()['custom'], '我的想法');

      const blank = QuestionAnswerItem(id: 'q2', custom: '   ');
      // 空字符串会被宿主当成「没填」，不能发出去。
      expect(blank.toJson().containsKey('custom'), isFalse);
    });

    test('跳过的题目也要应答（空选是合法回答）', () {
      const answers = [
        QuestionAnswerItem(id: 'q1', selected: ['A']),
        QuestionAnswerItem(id: 'q2'),
      ];
      expect(answers.map((a) => a.toJson()['id']), ['q1', 'q2']);
      expect(answers[1].toJson()['selected'], isEmpty);
    });
  });

  group('任务状态', () {
    test('未知状态降级而不是让整页解析失败', () {
      expect(JobStatus.parse('running'), JobStatus.running);
      expect(JobStatus.parse('brand-new'), JobStatus.unknown);
      expect(JobStatus.parse(null), JobStatus.unknown);
      expect(JobStatus.running.isLive, isTrue);
      expect(JobStatus.completed.isLive, isFalse);
    });
  });

  group('RPC 参数信封', () {
    // 以下形状全部**实测**于 qianxun 0.7.0 内置的 harness
    // （见 scripts/probe_rpc.py），不是从参考项目抄的——参考项目锁在
    // harness 0.1.2-alpha.1，载荷是摊平的，与当前部署不兼容。
    test('session/list 用 _request（唯一的例外）', () {
      expect(DshClient.usesUnderscoreRequest('session/list'), isTrue);
      expect(
        DshClient.wrapArgs('session/list', null),
        {'_request': <String, Object?>{}},
      );
    });

    test('其余单参方法用 request', () {
      for (final m in [
        'session/page',
        'session/prompt',
        'session/cancel',
        'session/search',
        'session/create',
        r'$events/result',
        'skills/list',
      ]) {
        expect(
          DshClient.usesUnderscoreRequest(m),
          isFalse,
          reason: '$m 不该用 _request',
        );
        expect(
          DshClient.wrapArgs(m, {'a': 1}),
          {
            'request': {'a': 1},
          },
          reason: '$m 应该把请求体包在 request 里',
        );
      }
    });

    test('无参方法的 args 是空对象', () {
      for (final m in ['session/modelCatalog', 'agentPresets/list']) {
        expect(DshClient.takesNoArgs(m), isTrue);
        expect(DshClient.wrapArgs(m, {'ignored': true}), isEmpty);
      }
    });
  });

  group('会话摘要 · 实测字段', () {
    test('「已断开」= 子会话 且 父会话不在列表里（**不是** agentAvailable）', () {
      // ⚠️ 这条断言改过一次。原先写的是「agentAvailable 决定已断开」，
      // 那是 05 §2 在没探针的情况下写的。实测本部署 393 条会话：
      //   agentAvailable 分布 392 × False / 1 × True
      //   其中非子会话却有 False 的：142 条
      //   子会话且父会话真不在列表里的：0 条
      // 也就是说 agentAvailable 几乎恒为 false，它复述的是「现在没在跑」。
      // 拿它当判据会把 142 条**完全能打开**的顶级会话标成「已断开」。

      // 顶级会话：无论 agentAvailable 是什么，都不是「已断开」。
      final top = SessionSummary.fromJson({
        'sessionId': 'a',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'agentAvailable': false,
      });
      expect(top.isSubagent, isFalse);
      expect(top.isDetachedGiven(const {'a'}), isFalse,
          reason: '顶级会话没有父会话，不可能是孤儿');

      // 子会话，父会话还在列表里 → 仍可进入。
      final childOk = SessionSummary.fromJson({
        'sessionId': 'c',
        'parentSessionId': 'a',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'agentAvailable': false,
      });
      expect(childOk.isSubagent, isTrue);
      expect(childOk.isDetachedGiven(const {'a', 'c'}), isFalse);

      // 子会话，父会话**不在**列表里 → 真·进不去。
      final childOrphan = SessionSummary.fromJson({
        'sessionId': 'd',
        'parentSessionId': 'gone',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'agentAvailable': false,
      });
      expect(childOrphan.isDetachedGiven(const {'a', 'd'}), isTrue);
      expect(childOrphan.running, isFalse);
    });

    test('turnOutline 提供现成预览', () {
      final s = SessionSummary.fromJson({
        'sessionId': 'c',
        'updatedAt': 0,
        'running': false,
        'blank': false,
        'projections': {
          'asOfSeq': 9,
          'values': {
            'title': '部署测试',
            'turnOutline': [
              {'turn': 1, 'seq': 4, 'prompt': '把代码部署上去', 'response': '已完成'},
              {'turn': 2, 'seq': 9, 'prompt': '再看看日志', 'response': '正常'},
            ],
          },
        },
      });
      expect(s.turnOutline.length, 2);
      expect(s.lastPrompt, '再看看日志');
    });

    test('没有 turnOutline 时预览为 null（不留空字符串）', () {
      final s = SessionSummary.fromJson({
        'sessionId': 'd',
        'updatedAt': 0,
        'running': false,
        'blank': true,
      });
      expect(s.lastPrompt, isNull);
    });
  });

  group('turn/end：失败态唯一真实的数据源', () {
    TurnEndEvent ev(Map<String, Object?> data) =>
        SessionEventView.fromWire(
          SessionWireEvent(type: 'turn/end', seq: 1, time: 0, data: data),
        ) as TurnEndEvent;

    test('嵌套形状 data.reason.kind 能读到', () {
      // ⚠️ 原实现把整个 `data` 当 reason 再读 `reason['kind']`，
      // 也就是只认 data['kind']。若实际形状是嵌套的，isError 永远 false，
      // 转录里的失败态从来没生效过。
      final e = ev({
        'turn': 1,
        'reason': {'kind': 'error'},
      });
      expect(e.reasonKind, 'error');
      expect(e.isError, isTrue);
      expect(e.isBadEnding, isTrue);
    });

    test('扁平形状 data.kind 也能读到', () {
      final e = ev({'turn': 1, 'kind': 'aborted'});
      expect(e.reasonKind, 'aborted');
      expect(e.isAborted, isTrue);
      expect(e.isBadEnding, isTrue);
    });

    test('正常收尾不算坏结局', () {
      final e = ev({
        'turn': 1,
        'reason': {'kind': 'completed'},
      });
      expect(e.isBadEnding, isFalse);
      expect(e.isError, isFalse);
    });

    test('没有 reason 时按 completed 处理，不猜', () {
      final e = ev({'turn': 1});
      expect(e.reasonKind, 'completed');
      expect(e.isBadEnding, isFalse);
    });
  });
}

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:qianxun_mobile/data/dsh/attachment_uploader.dart';
import 'package:qianxun_mobile/data/dsh/qx_protocol.dart';

/// 假的服务端。逐条记录收到的请求，好断言线格式。
class _FakeGateway {
  final List<http.Request> requests = [];
  int status = 200;
  String body = '';

  http.Client client() => MockClient((req) async {
        requests.add(req);
        return http.Response(body, status,
            headers: {'content-type': 'application/json'});
      });
}

void main() {
  group('上传路径', () {
    test('sessionId 与 name 走 query，不是 body 也不是 header', () {
      final path = qxUploadPath('sess-123', '截图.png');
      expect(path, startsWith('/api/session/uploadFileBinary?'));
      final q = Uri.parse('http://x$path').queryParameters;
      expect(q['sessionId'], 'sess-123');
      expect(q['name'], '截图.png');
    });

    test('name 为空时不带 name（服务端会默认叫 file）', () {
      final q = Uri.parse('http://x${qxUploadPath('s', null)}').queryParameters;
      expect(q.containsKey('name'), isFalse);
      expect(q['sessionId'], 's');
    });
  });

  group('上传请求的线格式', () {
    test('Content-Type 是 octet-stream，body 是裸字节不是 JSON', () async {
      final fake = _FakeGateway()
        ..body = '{"ok":true,"value":{"receiptId":"r-1",'
            '"file":{"attachmentId":"sha256:aa","name":"a.png","bytes":3}}}';
      final up = AttachmentUploader(
        origin: 'http://h:1',
        qxToken: 'tok',
        client: fake.client(),
      );

      final bytes = Uint8List.fromList([1, 2, 3]);
      final res = await up.upload(sessionId: 's1', bytes: bytes, name: 'a.png');

      expect(res.isOk, isTrue);
      expect(res.valueOrNull!.receiptId, 'r-1');
      expect(res.valueOrNull!.bytes, 3);

      final req = fake.requests.single;
      expect(req.headers['Content-Type'], 'application/octet-stream');
      // 裸字节：**不是** base64、不是 JSON。
      expect(req.bodyBytes, bytes);
      expect(req.body, isNot(contains('args')));
      expect(req.headers['Cookie'], 'qx_token=tok');
    });

    test('响应是裸的 {ok,value}，没有 server-response 信封', () async {
      final fake = _FakeGateway()
        ..body = '{"ok":true,"value":{"receiptId":"r-2","file":{'
            '"attachmentId":"sha256:bb","name":"b.txt","bytes":9}}}';
      final up = AttachmentUploader(
        origin: 'http://h:1',
        qxToken: null,
        client: fake.client(),
      );
      final res = await up.upload(
        sessionId: 's',
        bytes: Uint8List(9),
        name: 'b.txt',
      );
      expect(res.isOk, isTrue);
      expect(res.valueOrNull!.name, 'b.txt');
    });

    test('ok:false 时错误被解析出来', () async {
      final fake = _FakeGateway()
        ..body = '{"ok":false,"error":{"code":"payload-too-large",'
            '"message":"太大了","details":{}}}';
      final up = AttachmentUploader(
        origin: 'http://h:1',
        qxToken: null,
        client: fake.client(),
      );
      final res = await up.upload(
        sessionId: 's',
        bytes: Uint8List.fromList([1]),
        name: 'a',
      );
      expect(res.isOk, isFalse);
      expect(res.errorOrNull!.code, 'payload-too-large');
      expect(res.errorOrNull!.message, '太大了');
    });

    test('HTTP 415 的纯文本错误体被保留（网关用 text/plain 报错）', () async {
      final fake = _FakeGateway()
        ..status = 415
        ..body = 'content type must be application/octet-stream';
      final up = AttachmentUploader(
        origin: 'http://h:1',
        qxToken: null,
        client: fake.client(),
      );
      final res = await up.upload(
        sessionId: 's',
        bytes: Uint8List.fromList([1]),
        name: 'a',
      );
      expect(res.isOk, isFalse);
      expect(res.errorOrNull!.message, contains('application/octet-stream'));
    });

    test('空文件直接拒绝，不发请求', () async {
      final fake = _FakeGateway();
      final up = AttachmentUploader(
        origin: 'http://h:1',
        qxToken: null,
        client: fake.client(),
      );
      final res = await up.upload(
        sessionId: 's',
        bytes: Uint8List(0),
        name: 'a',
      );
      expect(res.isOk, isFalse);
      expect(fake.requests, isEmpty, reason: '空文件不该打到网络上');
    });

    test('200 但响应不是 JSON 时明确报错，不假装成功', () async {
      final fake = _FakeGateway()..body = '<html>gateway</html>';
      final up = AttachmentUploader(
        origin: 'http://h:1',
        qxToken: null,
        client: fake.client(),
      );
      final res = await up.upload(
        sessionId: 's',
        bytes: Uint8List.fromList([1]),
        name: 'a',
      );
      expect(res.isOk, isFalse);
      expect(res.errorOrNull!.message, contains('不是 JSON'));
    });
  });

  group('content part：图片与文件走两条不同的路', () {
    test('文件用 receiptId，不带 attachmentId', () {
      const r = UploadResult(
        receiptId: 'receipt-1',
        attachmentId: 'sha256:aa',
        name: 'a.txt',
        bytes: 3,
      );
      final part = toFileContentPart(r, mediaType: 'text/plain');
      expect(part['type'], 'file');
      expect(part['receiptId'], 'receipt-1');
      expect(part.containsKey('attachmentId'), isFalse,
          reason: 'attachmentId 是持久标识，不是消费凭证');
    });

    test('图片是内联 base64，根本没有 receiptId', () {
      final part = toImageContentPart(
        base64Data: 'AAAA',
        name: 'a.png',
      );
      expect(part['type'], 'image');
      expect(part['mediaType'], 'image/png');
      expect(part['data'], 'AAAA');
      expect(part.containsKey('receiptId'), isFalse,
          reason: '图片不需要先上传换 receiptId');
    });

    test('图片猜不到 MIME 时兜底成 image/png', () {
      final part = toImageContentPart(base64Data: 'x', name: 'weird.zzz');
      expect(part['mediaType'], 'image/png');
    });

    test('日志里的 base64 被隐去', () {
      final shown = debugParts([
        {'type': 'image', 'mediaType': 'image/png', 'data': 'A' * 400},
      ]);
      expect(shown.contains('已隐去'), isTrue);
      expect(shown.length, lessThan(300));
    });
  });

  group('MIME 猜测', () {
    test('常见类型', () {
      expect(guessMediaType('a.png'), 'image/png');
      expect(guessMediaType('a.JPG'), 'image/jpeg');
      expect(guessMediaType('a.pdf'), 'application/pdf');
      expect(guessMediaType('a.zip'), 'application/zip');
    });

    test('猜不到就返回 null，让服务端自己嗅探', () {
      // 猜错比不猜更糟（会把 png 当 text/plain）。
      expect(guessMediaType('a.bin'), isNull);
      expect(guessMediaType('noext'), isNull);
    });
  });
}

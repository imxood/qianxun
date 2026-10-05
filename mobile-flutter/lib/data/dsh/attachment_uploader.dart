import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import 'pair_url.dart';
import 'qx_protocol.dart';
import 'rpc_envelope.dart';

/// 一次上传的结果。
class UploadResult {
  const UploadResult({
    required this.receiptId,
    required this.attachmentId,
    required this.name,
    required this.bytes,
    this.mediaType,
  });

  /// **一次性**消费凭证，送进 `session/prompt` 的 content part。
  final String receiptId;

  /// 持久标识，之后用 `session/attachment` 读回内容。
  final String attachmentId;

  final String name;
  final int bytes;
  final String? mediaType;
}

/// 附件上传。
///
/// 单独一个类而不是塞进 [DshClient]，因为这条路由的线格式与其它所有
/// RPC **都不同**（裸二进制、无信封、无 `client-request`）。塞在一起
/// 迟早有人用错的那条路径去调它。
class AttachmentUploader {
  AttachmentUploader({
    required this.origin,
    required this.qxToken,
    http.Client? client,
  }) : _client = client ?? http.Client();

  final String origin;
  final String? qxToken;
  final http.Client _client;

  void dispose() => _client.close();

  /// 上传 [bytes] 到会话 [sessionId]。
  ///
  /// [name] 必须是**文件名**（`截图.png`），不是路径。服务端只取最后一段，
  /// 传完整路径会让会话里出现一长串目录。
  Future<RpcResult<UploadResult>> upload({
    required String sessionId,
    required Uint8List bytes,
    required String name,
    String? mediaType,
  }) async {
    if (bytes.isEmpty) {
      return const RpcErr<UploadResult>(
        RpcError(code: RpcErrorCode.badRequest, message: '文件是空的'),
      );
    }
    final uri = Uri.parse('$origin${qxUploadPath(sessionId, name)}');
    try {
      final res = await _client.post(
        uri,
        headers: {
          // 不是 multipart，不是 base64，是**裸二进制**。
          'Content-Type': 'application/octet-stream',
          if (qxToken != null) 'Cookie': cookieHeader(qxToken)!,
          'User-Agent': 'QianxunMobile/0.2 (Flutter)',
        },
        // `http` 遇到 `List<int>` 会直接当 body 发，不做 JSON 编码。
        body: bytes,
      );

      if (res.statusCode != 200) {
        // 错误体是纯文本（网关的 text/plain 约定），不是 JSON。
        return RpcErr<UploadResult>(
          RpcError.fromStatus(res.statusCode, res.body),
        );
      }

      // 响应是**裸的** `{ok, value}`，没有 server-response 信封。
      final decoded = tryDecodeJson(res.body);
      if (decoded is! Map) {
        return const RpcErr<UploadResult>(
          RpcError(
            code: RpcErrorCode.internal,
            message: '上传响应不是 JSON，上传可能没成功',
          ),
        );
      }
      final ok = decoded['ok'] == true;
      if (!ok) {
        final err = RpcError.fromJson(asMap(decoded['error']));
        return RpcErr<UploadResult>(err);
      }

      final value = asMap(decoded['value']);
      final file = asMap(value['file']);
      final receiptId = value['receiptId'] as String?;
      if (receiptId == null || receiptId.isEmpty) {
        return const RpcErr<UploadResult>(
          RpcError(code: RpcErrorCode.internal, message: '上传成功但没拿到 receiptId'),
        );
      }
      return RpcOk<UploadResult>(
        UploadResult(
          receiptId: receiptId,
          attachmentId: (file['attachmentId'] as String?) ?? '',
          name: (file['name'] as String?) ?? name,
          bytes: (file['bytes'] as num?)?.toInt() ?? bytes.length,
          mediaType: mediaType,
        ),
      );
    } on SocketException catch (e) {
      return RpcErr<UploadResult>(
        RpcError(
          code: RpcErrorCode.network,
          message: '上传失败：${e.osError?.message ?? e.message}',
        ),
      );
    } on http.ClientException catch (e) {
      return RpcErr<UploadResult>(
        RpcError(code: RpcErrorCode.network, message: '上传失败：${e.message}'),
      );
    }
  }

  /// 从磁盘读文件并上传。
  Future<RpcResult<UploadResult>> uploadFile({
    required String sessionId,
    required File file,
    String? mediaType,
  }) async {
    try {
      final bytes = await file.readAsBytes();
      return await upload(
        sessionId: sessionId,
        bytes: bytes,
        // 只取文件名。传完整路径会让会话里出现一长串目录。
        name: file.uri.pathSegments.isEmpty
            ? 'file'
            : file.uri.pathSegments.last,
        mediaType: mediaType,
      );
    } on FileSystemException catch (e) {
      return RpcErr<UploadResult>(
        RpcError(
          code: RpcErrorCode.badRequest,
          message: '读不了文件：${e.message}',
        ),
      );
    }
  }
}
/// 猜 MIME 类型。
///
/// 只覆盖常见类型，猜不到就返回 null——服务端会自己按内容嗅探，
/// 猜错比不猜更糟（会把 png 当成 text/plain）。
String? guessMediaType(String filename) {
  final ext = filename.contains('.')
      ? filename.split('.').last.toLowerCase()
      : '';
  return switch (ext) {
    'png' => 'image/png',
    'jpg' || 'jpeg' => 'image/jpeg',
    'gif' => 'image/gif',
    'webp' => 'image/webp',
    'bmp' => 'image/bmp',
    'pdf' => 'application/pdf',
    'txt' => 'text/plain',
    'md' => 'text/markdown',
    'log' => 'text/plain',
    'json' => 'application/json',
    'csv' => 'text/csv',
    'zip' => 'application/zip',
    'tar' => 'application/x-tar',
    'gz' => 'application/gzip',
    _ => null,
  };
}

/// 上传结果 → `session/prompt` 的 content part。
///
/// ⚠️ **图片和文件走的是两条完全不同的路**（实测，见 `probe_upload.py`）：
///
/// | | 形状 | 要不要先上传 |
/// |---|---|---|
/// | 图片 | `{"type":"image","mediaType":"image/png","data":"<base64>","name":"…"}` | **不用** |
/// | 文件 | `{"type":"file","receiptId":"<uuid>"}` | 要 |
///
/// 图片是**内联 base64** 送进去的，不需要 `receiptId` 那一跳。
/// 参考 DTO（`PromptContentPart.Image`）也是 `mediaType` + `data` +
/// 可选 `name`，`receiptId` 只属于 `File`。
///
/// 顺带一提：实测同一 `requestId` 无论配同一个还是新的 `receiptId`
/// 都会被接受，所以网络抖动后原样重发是安全的。
Map<String, Object?> toFileContentPart(UploadResult r, {String? mediaType}) => {
      'type': 'file',
      'receiptId': r.receiptId,
      if (mediaType != null || r.mediaType != null) 'mediaType': mediaType ?? r.mediaType,
    };

/// 图片 part（内联 base64，不经过上传）。
Map<String, Object?> toImageContentPart({
  required String base64Data,
  required String name,
  String? mediaType,
}) =>
    {
      'type': 'image',
      'mediaType': mediaType ?? guessMediaType(name) ?? 'image/png',
      'data': base64Data,
      if (name.isNotEmpty) 'name': name,
    };

/// 把 content part 序列化成一行 JSON（仅用于日志，打印前会截断）。
///
/// 图片 part 里的 `data` 是整张图的 base64，**绝不能**打进日志。
String debugParts(List<Map<String, Object?>> parts) => jsonEncode(parts).replaceAll(
      RegExp(r'"data":"[^"]{20,}"'),
      '"data":"…(已隐去)"',
    );

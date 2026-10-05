import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/qx_log.dart';
import '../../../data/dsh/attachment_uploader.dart';
import '../../../data/dsh/rpc_envelope.dart';
import '../../connections/application/connections_controller.dart';

/// 一个待发送或已发送的附件。
///
/// ⚠️ **图片与文件的生命周期不同**（实测，见 `probe_upload.py`）：
///
/// - **图片**：内联 base64 直接进 `session/prompt` 的 content part，
///   **根本不走上传**。所以它从挂上的那一刻起就是 `ready`。
/// - **文件**：必须先 `POST /api/session/uploadFileBinary` 换一个
///   `receiptId`，才能进 content part。
///
/// 早期版本给图片也走了一遍上传，既慢又白搭一个网络往返。
@immutable
class PendingAttachment {
  const PendingAttachment({
    required this.localId,
    required this.name,
    required this.bytes,
    required this.isImage,
    this.path,
    this.data,
    this.receiptId,
    this.state = AttachState.pending,
    this.error,
  });

  /// 本地唯一 id。用微秒时间戳，**不要**用序号：附件是并发处理的，
  /// 序号会撞。
  final String localId;
  final String name;
  final int bytes;
  final bool isImage;

  /// 磁盘路径（从文件选择器来时有）。
  final String? path;

  /// 图片的内联 base64。`isImage == true` 时必填。
  final String? data;

  /// 文件的 `receiptId`。`isImage == false` 且上传完成后才有。
  final String? receiptId;

  final AttachState state;
  final String? error;

  bool get isReady => state == AttachState.ready;

  PendingAttachment copyWith({
    String? receiptId,
    String? data,
    AttachState? state,
    String? error,
  }) =>
      PendingAttachment(
        localId: localId,
        name: name,
        bytes: bytes,
        isImage: isImage,
        path: path,
        data: data ?? this.data,
        receiptId: receiptId ?? this.receiptId,
        state: state ?? this.state,
        error: error,
      );
}

enum AttachState { pending, uploading, ready, failed }

/// 输入栏状态。
class ComposerState {
  const ComposerState({
    this.attachments = const [],
    this.sending = false,
    this.error,
    this.steer = false,
  });

  final List<PendingAttachment> attachments;
  final bool sending;
  final String? error;

  /// `steer` = 插进当前轮次；`false` = 排队等当前轮次跑完。
  final bool steer;

  bool get busy => sending || attachments.any((a) => a.state == AttachState.uploading);

  /// 能否发送。
  ///
  /// ⚠️ 附件必须**全部** `ready` 才算可发。放行 `pending` 是静默丢数据：
  /// `send()` 只挑 `ready` 的进 content，`pending` 会被无声地不带上，
  /// 用户以为「图和文字一起发出去了」，实际只到了文字。
  bool get canSend {
    if (busy) return false;
    return attachments.every((a) => a.state == AttachState.ready);
  }

  ComposerState copyWith({
    List<PendingAttachment>? attachments,
    bool? sending,
    String? error,
    bool? steer,
    bool clearError = false,
  }) =>
      ComposerState(
        attachments: attachments ?? this.attachments,
        sending: sending ?? this.sending,
        error: clearError ? null : (error ?? this.error),
        steer: steer ?? this.steer,
      );
}

/// 新建对话 + 收发消息 + 附件的控制器。
class ComposerController extends StateNotifier<ComposerState> {
  ComposerController(this._ref, this.sessionId)
      : super(const ComposerState()) {
    _uploader = AttachmentUploader(
      origin: _ref.read(selectedConnectionProvider)?.pair?.origin ?? '',
      qxToken: _ref.read(selectedConnectionProvider)?.effectiveToken,
    );
  }

  final Ref _ref;
  final String sessionId;
  late final AttachmentUploader _uploader;

  @override
  void dispose() {
    _uploader.dispose();
    super.dispose();
  }

  void setSteer(bool steer) {
    if (state.steer == steer) return;
    state = state.copyWith(steer: steer);
  }

  void clearError() {
    if (state.error != null) state = state.copyWith(clearError: true);
  }

  void removeAttachment(String localId) {
    state = state.copyWith(
      attachments: state.attachments.where((a) => a.localId != localId).toList(),
    );
  }

  /// 挂一张图片（拍照 / 相册选图）。
  ///
  /// **不上传。** 图片是内联 base64 进消息的，走上传既慢又白搭一跳。
  /// 所以它挂上就是 `ready`。
  Future<void> attachImage({
    required String name,
    required Uint8List bytes,
  }) async {
    _mutate(
      PendingAttachment(
        localId: 'a${DateTime.now().microsecondsSinceEpoch}',
        name: name,
        bytes: bytes.length,
        isImage: true,
        data: base64Encode(bytes),
        state: AttachState.ready,
      ),
    );
  }

  /// 挂一个文件。需要先上传换 `receiptId`。
  Future<void> attachFile(File file) async {
    late PendingAttachment att;
    try {
      final len = await file.length();
      att = PendingAttachment(
        localId: 'a${DateTime.now().microsecondsSinceEpoch}',
        name: file.uri.pathSegments.isEmpty ? 'file' : file.uri.pathSegments.last,
        bytes: len,
        isImage: false,
        path: file.path,
        state: AttachState.pending,
      );
    } on FileSystemException catch (e) {
      state = state.copyWith(error: '读不了这个文件：${e.message}');
      return;
    }
    _mutate(att);
    _replace(
      att.localId,
      att.copyWith(state: AttachState.uploading),
    );
    final res = await _uploader.uploadFile(
      sessionId: sessionId,
      file: file,
      mediaType: guessMediaType(file.path),
    );
    _applyUpload(att, res);
  }

  void _mutate(PendingAttachment att) {
    state = state.copyWith(
      attachments: [...state.attachments, att],
      clearError: true,
    );
  }

  void _replace(String localId, PendingAttachment next) {
    state = state.copyWith(
      attachments: [
        for (final a in state.attachments) if (a.localId == localId) next else a,
      ],
    );
  }

  void _applyUpload(PendingAttachment att, RpcResult<UploadResult> res) {
    if (!mounted) return;
    res.map(
      (r) => _replace(
        att.localId,
        att.copyWith(receiptId: r.receiptId, state: AttachState.ready),
      ),
      (err) {
        qxlog.warn('attach', '上传失败 ${att.name}：${err.message}');
        _replace(
          att.localId,
          att.copyWith(state: AttachState.failed, error: err.message),
        );
      },
    );
  }

  /// 发送。
  ///
  /// ⚠️ **`requestId` 必须跨重试保持不变**：宿主把它持久化在被接受的那条
  /// 消息上，改了就变成两条不同的消息。
  ///
  /// 附件不受这条约束：`receiptId` 是一次性凭证，但**实测**同一
  /// `requestId` 配同一个或新的 `receiptId` 都会被接受，所以失败后原样
  /// 重发是安全的，**不需要**重新上传。
  String? _pendingRequestId;

  String? _pendingText;

  Future<RpcResult<Object?>> send(String text) async {
    final body = text.trim();
    final atts = state.attachments
        .where((a) => a.state == AttachState.ready && a.receiptId != null)
        .toList(growable: false);
    if (body.isEmpty && atts.isEmpty) {
      return const RpcErr<Object?>(
        RpcError(code: RpcErrorCode.badRequest, message: '消息是空的'),
      );
    }
    if (state.busy) {
      return const RpcErr<Object?>(
        RpcError(code: RpcErrorCode.internal, message: '还有附件在上传，稍等一下'),
      );
    }

    final client = _ref.read(dshClientProvider);
    if (client == null) {
      return const RpcErr<Object?>(
        RpcError(code: RpcErrorCode.unauthorized, message: '还没连上电脑'),
      );
    }

    // 同一段文字的重试沿用同一个 requestId。
    final sameAsPending = _pendingText == body;
    if (!sameAsPending) {
      _pendingRequestId = 'm${DateTime.now().microsecondsSinceEpoch.toRadixString(36)}';
      _pendingText = body;
    }
    final requestId = _pendingRequestId!;

    state = state.copyWith(sending: true, clearError: true);
    // 图片内联 base64，文件带 receiptId。**两种形状不能混。**
    final content = <Map<String, Object?>>[
      if (body.isNotEmpty) {'type': 'text', 'text': body},
      for (final a in atts)
        if (a.isImage)
          toImageContentPart(
            base64Data: a.data!,
            name: a.name,
          )
        else
          toFileContentPart(
            UploadResult(
              receiptId: a.receiptId!,
              attachmentId: '',
              name: a.name,
              bytes: a.bytes,
            ),
            mediaType: guessMediaType(a.name),
          ),
    ];
    qxlog.info(
      'attach',
      '发送：文字 ${body.length} 字，附件 ${atts.length} 个（图片 ${atts.where((a) => a.isImage).length}，文件 ${atts.where((a) => !a.isImage).length}）',
    );

    final res = await client.call(
      'session/prompt',
      request: {
        'requestId': requestId,
        'sessionId': sessionId,
        'mode': state.steer ? 'steer' : 'queue',
        'content': content,
      },
    );

    if (!mounted) return res;
    if (res.isOk) {
      // 发送成功：这个 requestId 定下来了，下次发**新**消息要换新的。
      _pendingRequestId = null;
      _pendingText = null;
      // 清附件，但**保留 steer 选择**：用户刚切成 steer 就发一条，
      // 下一条也大概率还是 steer，静默重置回 queue 是恼人的。
      state = ComposerState(steer: state.steer);
    } else {
      // 失败：保留 requestId 与附件，让用户可以原样重发。
      state = state.copyWith(sending: false, error: res.errorOrNull?.message);
    }
    return res;
  }
}

/// 输入栏控制器。按会话 id 分族。
final composerControllerProvider = StateNotifierProvider.autoDispose
    .family<ComposerController, ComposerState, String>((ref, sessionId) {
  return ComposerController(ref, sessionId);
});

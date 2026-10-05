import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../app/theme/qx_semantic.dart';
import '../../../app/theme/qx_test_ids.dart';
import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../shared/widgets/primitives.dart';
import '../application/composer_controller.dart';

/// 输入栏。附件 + 文字 + steer 切换 + 发送。
///
/// ## 为什么整块换掉旧的实现
///
/// 旧版是个裸 `TextEditingController` 直接调 `transcriptController.send()`，
/// 结果 [ComposerController]（附件上传、图片内联、steer、requestId 重试安全）
/// **写好了却没有任何 UI 调用方** —— 也就是「支持图片与文件的发送」这个
/// 需求在界面上完全不可达：用户没有任何办法挂上附件。
///
/// ## 图片为什么不上传（实测，`probe_upload.py`）
///
/// 图片是**内联 base64** 直接进 `session/prompt` 的 content part，
/// 根本不走上传。所以它挂上就是 `ready`，没有进度、没有失败。
/// 文件相反：必须先换 `receiptId` 才能进 content part，所以有
/// `uploading → ready / failed` 三态，界面上必须如实显示。
class ComposerBar extends ConsumerStatefulWidget {
  const ComposerBar({
    super.key,
    required this.sessionId,
    required this.running,
    required this.onSent,
  });

  final String sessionId;

  /// agent 是否在跑。决定要不要显示 steer 切换。
  final bool running;

  /// 发送成功后回调（用来滚到底部）。
  final VoidCallback onSent;

  @override
  ConsumerState<ComposerBar> createState() => _ComposerBarState();
}

class _ComposerBarState extends ConsumerState<ComposerBar> {
  final _text = TextEditingController();

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  Future<void> _pickImage() async {
    final messenger = ScaffoldMessenger.of(context);
    final controller = ref.read(composerControllerProvider(widget.sessionId).notifier);
    try {
      // ⚠️ file_picker 11.x 去掉了 `FilePicker.platform`，改成直接静态调用。
      // 8.x 的 `FilePicker.platform.pickFiles(...)` 在这里编译不过。
      final res = await FilePicker.pickFiles(
        type: FileType.image,
        // 允许多选：手机上选三张图是常见操作。
        allowMultiple: true,
        withData: true,
      );
      if (res == null) return;
      for (final f in res.files) {
        final bytes = f.bytes;
        if (bytes == null) {
          // 拿不到字节就没什么可发的，如实说，不静默跳过。
          messenger.showSnackBar(
            SnackBar(content: Text('读不到「${f.name}」的内容')),
          );
          continue;
        }
        await controller.attachImage(name: f.name, bytes: bytes);
      }
    } on PlatformException catch (e) {
      messenger.showSnackBar(SnackBar(content: Text('选图失败：${e.message}')));
    }
  }

  Future<void> _pickFile() async {
    final messenger = ScaffoldMessenger.of(context);
    final controller = ref.read(composerControllerProvider(widget.sessionId).notifier);
    try {
      final res = await FilePicker.pickFiles(allowMultiple: true);
      if (res == null) return;
      for (final f in res.files) {
        final path = f.path;
        if (path == null) {
          messenger.showSnackBar(
            SnackBar(content: Text('读不到「${f.name}」的路径')),
          );
          continue;
        }
        await controller.attachFile(File(path));
      }
    } on PlatformException catch (e) {
      messenger.showSnackBar(SnackBar(content: Text('选文件失败：${e.message}')));
    }
  }

  Future<void> _send() async {
    final state = ref.read(composerControllerProvider(widget.sessionId));
    if (!state.canSend) return;
    final controller = ref.read(composerControllerProvider(widget.sessionId).notifier);
    final text = _text.text;
    HapticFeedback.selectionClick();
    final res = await controller.send(text);
    if (!mounted) return;
    res.map(
      (_) {
        _text.clear();
        widget.onSent();
        return null;
      },
      (error) {
        // 失败时**不清输入框**：用户要能原样重发。
        // requestId 由 controller 内部保持不变，重发才是同一条消息。
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(error.message)));
        return null;
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    final state = ref.watch(composerControllerProvider(widget.sessionId));
    final controller =
        ref.read(composerControllerProvider(widget.sessionId).notifier);
    final hasText = _text.text.trim().isNotEmpty;
    final canSend = state.canSend && (hasText || state.attachments.isNotEmpty);

    return Container(
      decoration: BoxDecoration(
        color: q.surface,
        border: Border(top: BorderSide(color: q.hairline)),
      ),
      child: SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (state.error != null)
              Padding(
                padding: const EdgeInsets.fromLTRB(
                  QxSpace.m,
                  QxSpace.s,
                  QxSpace.m,
                  0,
                ),
                child: InlineNotice(text: state.error!, isError: true),
              ),
            if (state.attachments.isNotEmpty)
              _AttachmentStrip(
                attachments: state.attachments,
                onRemove: controller.removeAttachment,
              ),
            if (widget.running)
              _SteerToggle(
                steer: state.steer,
                onChanged: controller.setSteer,
              ),
            Padding(
              padding: const EdgeInsets.fromLTRB(
                QxSpace.m,
                QxSpace.s,
                QxSpace.m,
                QxSpace.s,
              ),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [
                  _AttachMenu(
                    onImage: _pickImage,
                    onFile: _pickFile,
                    busy: state.busy,
                  ),
                  const SizedBox(width: QxSpace.xs),
                  Expanded(
                    child: Semantics(
                      identifier: QxTestIds.composerInput,
                      textField: true,
                      child: TextField(
                        controller: _text,
                        style: QxType.body,
                        minLines: 1,
                        maxLines: 6,
                        textInputAction: TextInputAction.newline,
                        keyboardType: TextInputType.multiline,
                        onChanged: (_) => setState(() {}),
                        decoration: const InputDecoration(
                          hintText: '给 agent 发消息…',
                          isDense: true,
                          contentPadding: EdgeInsets.symmetric(
                            horizontal: QxSpace.m,
                            vertical: QxSpace.s + QxSpace.xxs,
                          ),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(width: QxSpace.s),
                  _SendButton(
                    enabled: canSend,
                    busy: state.sending,
                    onPressed: _send,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 「+」菜单：图片 / 文件分开两个入口。
///
/// 刻意不用一个笼统的「添加附件」：图片**不上传**、文件**要上传**，
/// 两者在网络行为上完全不同（见本文件头的说明），让用户在按下之前就知道
/// 选的这个会发生什么。
class _AttachMenu extends StatelessWidget {
  const _AttachMenu({
    required this.onImage,
    required this.onFile,
    required this.busy,
  });

  final VoidCallback onImage;
  final VoidCallback onFile;
  final bool busy;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      identifier: QxTestIds.attachButton,
      button: true,
      label: '添加附件',
      child: PopupMenuButton<String>(
        icon: Icon(
          busy ? Icons.hourglass_top : Icons.add_circle_outline,
          size: 22,
        ),
        tooltip: '添加附件',
        onSelected: (v) => v == 'image' ? onImage() : onFile(),
        itemBuilder: (context) => const [
          PopupMenuItem(
            value: 'image',
            child: ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              leading: Icon(Icons.image_outlined, size: 20),
              title: Text('图片'),
              // 不解释「不上传」这种实现细节，只说结果。
              subtitle: Text('直接随消息发出'),
            ),
          ),
          PopupMenuItem(
            value: 'file',
            child: ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              leading: Icon(Icons.attach_file, size: 20),
              title: Text('文件'),
              subtitle: Text('先上传再发出'),
            ),
          ),
        ],
      ),
    );
  }
}

/// 已挂附件的横条。**每个附件的态必须如实显示** ——
/// 文件在上传时如果只显示一个「已添加」，用户会在它还没传完时按下发送，
/// 然后发现文件没到。
class _AttachmentStrip extends StatelessWidget {
  const _AttachmentStrip({required this.attachments, required this.onRemove});

  final List<PendingAttachment> attachments;
  final void Function(String localId) onRemove;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    return SizedBox(
      // ⚠️ 高度由**触控区 + 卡片内边距**推出来，不是拍脑袋的：
      //   卡片内边距 8 + 行 48（移除按钮的 48dp 命中区，02 §3.4 硬要求）
      //   + 4 + 状态文字 ~16 + 内边距 8 = 84；ListView 还要再加上下 padding
      //   与分隔线余量，取 92。
      //
      // 先后试过 72（溢出 16px）与 84（溢出 4px）——两次都是被 widget 测试
      // 逮到的，不是猜出来的。**不能靠缩小命中区解决**：02 §3.4 要求所有
      // 可点区域 ≥ 48×48dp。
      height: 92,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.fromLTRB(
          QxSpace.m,
          QxSpace.s,
          QxSpace.m,
          0,
        ),
        itemCount: attachments.length,
        separatorBuilder: (_, __) => const SizedBox(width: QxSpace.xs),
        itemBuilder: (context, i) {
          final a = attachments[i];
          return Semantics(
            label: '${a.name}，${_stateWord(a.state)}',
            child: Container(
              width: 132,
              padding: const EdgeInsets.all(QxSpace.s),
              decoration: BoxDecoration(
                color: q.surfaceSunken,
                borderRadius: QxRadius.controlAll,
                border: Border.all(
                  color: a.state == AttachState.failed ? q.failed : q.hairline,
                ),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                // `min` 而不是 `spaceBetween`：卡片高度已经由 48dp 触控区
                // 定死，再用 spaceBetween 去分配剩余空间会在内容略高时溢出。
                mainAxisSize: MainAxisSize.min,
                children: [
                  Row(
                    children: [
                      Icon(
                        a.isImage ? Icons.image_outlined : Icons.insert_drive_file,
                        size: 14,
                        color: q.textSecondary,
                      ),
                      const SizedBox(width: 4),
                      Expanded(
                        child: Text(
                          a.name,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: QxType.micro.copyWith(color: q.textPrimary),
                        ),
                      ),
                      // 图标按钮必须 48dp 命中区，视觉可以小。
                      InkResponse(
                        onTap: () => onRemove(a.localId),
                        radius: 24,
                        child: const SizedBox(
                          width: 48,
                          height: 48,
                          child: Icon(Icons.close, size: 14),
                        ),
                      ),
                    ],
                  ),
                  Text(
                    '${_stateWord(a.state)} · ${formatBytes(a.bytes)}',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: QxType.micro.copyWith(
                      color: a.state == AttachState.failed
                          ? q.failed
                          : q.textTertiary,
                    ),
                  ),
                ],
              ),
            ),
          );
        },
      ),
    );
  }

  static String _stateWord(AttachState s) => switch (s) {
        AttachState.pending => '准备中',
        AttachState.uploading => '上传中',
        AttachState.ready => '已就绪',
        AttachState.failed => '失败',
      };
}

/// steer 切换：插进当前轮次，还是排队。
///
/// 05 §6 的「等你回答」链路里，steer 是手机上最常用的一个动作
/// （agent 跑偏了，你想立刻纠正它，而不是等它跑完）。
class _SteerToggle extends StatelessWidget {
  const _SteerToggle({required this.steer, required this.onChanged});

  final bool steer;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    return Padding(
      padding: const EdgeInsets.fromLTRB(QxSpace.m, QxSpace.s, QxSpace.m, 0),
      child: Row(
        children: [
          // 状态 + 动作两条通道：中文词说明差异，accent 说明选中。
          // 绝不只靠颜色。
          Text(
            steer ? '插进当前轮次' : '排队等这轮跑完',
            style: QxType.micro.copyWith(
              color: steer ? q.accent : q.textTertiary,
              fontWeight: steer ? FontWeight.w700 : FontWeight.w400,
            ),
          ),
          const SizedBox(width: QxSpace.s),
          Semantics(
            // ⚠️ 必须 `container: true` + `excludeSemantics: true`。
            // `Switch` 自己会生成一个语义节点（role=toggled 但 label 为空），
            // 不 exclude 掉它的话，外层这个 label 不会被合并上去，
            // uiautomator2 读到的 `content-desc` 就是空串。
            // 那个「两个断言读到空 label」就是这么来的。
            container: true,
            excludeSemantics: true,
            label: steer ? '当前：插进当前轮次' : '当前：排队',
            toggled: steer,
            child: Switch(
              value: steer,
              onChanged: onChanged,
            ),
          ),
        ],
      ),
    );
  }
}

class _SendButton extends StatelessWidget {
  const _SendButton({
    required this.enabled,
    required this.busy,
    required this.onPressed,
  });

  final bool enabled;
  final bool busy;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final q = context.qx;
    return Semantics(
      identifier: QxTestIds.sendButton,
      button: true,
      enabled: enabled,
      label: busy ? '发送中' : '发送',
      child: AnimatedContainer(
        duration: QxMotion.quick,
        curve: QxMotion.enter,
        width: 44,
        height: 44,
        decoration: BoxDecoration(
          // 可发送时用 accent 填充。**状态色不得进按钮**——按钮是
          // 「可操作」，不是「当前状态」。
          color: enabled ? q.accent : q.surfaceRaised,
          borderRadius: QxRadius.controlAll,
        ),
        child: Material(
          color: Colors.transparent,
          child: InkWell(
            onTap: enabled ? onPressed : null,
            borderRadius: QxRadius.controlAll,
            child: Center(
              child: busy
                  ? SizedBox(
                      width: 17,
                      height: 17,
                      // 与下方箭头同一个 `onAccent`。这里原来硬写
                      // `Colors.white`，在暗色 accent 上只有 2.56:1。
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        color: enabled ? q.onAccent : q.textTertiary,
                      ),
                    )
                  : Icon(
                      Icons.arrow_upward,
                      size: 20,
                      color: enabled ? q.onAccent : q.textTertiary,
                    ),
            ),
          ),
        ),
      ),
    );
  }
}

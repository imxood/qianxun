import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../shared/widgets/primitives.dart';
import '../../connections/application/connections_controller.dart';
import '../../sessions/presentation/link_status_bar.dart';
import '../application/transcript_controller.dart';
import 'pending_interaction_bar.dart';
import 'transcript_view.dart';
import '../../../../app/theme/qx_semantic.dart';

/// 会话详情（原生 DSH 页面）。
class SessionScreen extends ConsumerStatefulWidget {
  const SessionScreen({super.key, required this.sessionId, this.title});

  final String sessionId;
  final String? title;

  @override
  ConsumerState<SessionScreen> createState() => _SessionScreenState();
}

class _SessionScreenState extends ConsumerState<SessionScreen> {
  final _composer = TextEditingController();
  final _scroll = ScrollController();
  String? _pendingRequestId;

  @override
  void dispose() {
    _composer.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    final text = _composer.text.trim();
    if (text.isEmpty) return;
    final controller = ref.read(transcriptControllerProvider(widget.sessionId).notifier);

    // `requestId` 在**发送前**生成一次并保留：同一条消息的重试必须带
    // 同一个 id（宿主把它持久化在消息上），两条不同的消息绝不能共用。
    _pendingRequestId ??= 'm${DateTime.now().microsecondsSinceEpoch.toRadixString(36)}';

    HapticFeedback.selectionClick();
    _composer.clear();
    setState(() {});

    final result = await controller.send(text, requestId: _pendingRequestId);
    if (!mounted) return;
    result.map(
      (_) {
        // 宿主已接受：这条消息的 id 定下来了，下次重试要沿用同一个
        // 新消息才清空。
        setState(() => _pendingRequestId = null);
        _scrollToEnd();
        return null;
      },
      (error) {
        // 失败时**不**清空 id——用户重发同一条内容必须复用它，否则
        // 宿主会把它当成两条不同的消息。
        ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(error.message)));
        return null;
      },
    );
  }

  void _scrollToEnd() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_scroll.hasClients) return;
      _scroll.animateTo(
        _scroll.position.maxScrollExtent,
        duration: QxMotion.normal,
        curve: QxMotion.enter,
      );
    });
  }

  @override
  Widget build(BuildContext context) {

    final state = ref.watch(transcriptControllerProvider(widget.sessionId));
    final controller = ref.read(transcriptControllerProvider(widget.sessionId).notifier);
    final conn = ref.watch(selectedConnectionProvider);
    final link = conn == null ? null : ref.watch(dshConnectionProvider(conn));

    // 新内容到达时贴底，但**不要**在用户正在往上翻的时候把他拽回来——
    // 那是最让人恼火的移动端行为。
    ref.listen(transcriptControllerProvider(widget.sessionId), (prev, next) {
      if (prev?.rows.length != next.rows.length && _nearBottom()) {
        _scrollToEnd();
      }
    });

    return Scaffold(
      appBar: AppBar(
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              widget.title ?? '会话',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: QxType.title,
            ),
            if (state.running)
              Padding(
                padding: const EdgeInsets.only(top: 1),
                child: Row(
                  children: [
                    const StateDot(
                      state: QxSessionState.running,
                      size: 6,
                      animate: true,
                    ),
                    const SizedBox(width: 6),
                    Text(
                      'agent 运行中',
                      style: QxType.micro.copyWith(
                        color: context.qx.running,
                        fontSize: 10.5,
                      ),
                    ),
                  ],
                ),
              ),
          ],
        ),
        actions: [
          if (state.running)
            IconButton(
              onPressed: controller.cancel,
              icon: const Icon(Icons.stop_circle_outlined, size: 22),
              tooltip: '中断当前轮次',
            ),
          IconButton(
            onPressed: controller.reload,
            icon: const Icon(Icons.refresh, size: 20),
            tooltip: '重新加载',
          ),
          const SizedBox(width: QxSpace.xs),
        ],
      ),
      body: Column(
        children: [
          LinkStatusBar(link: link, onRetry: () => link?.retryNow()),
          Expanded(
            child: ListView.builder(
              controller: _scroll,
              padding: const EdgeInsets.fromLTRB(
                QxSpace.l,
                QxSpace.m,
                QxSpace.l,
                QxSpace.xxl,
              ),
              // 预构建几屏：转录行高差异大，向上滚时不该有空白。
              // ignore: deprecated_member_use
              cacheExtent: 1200,
              itemCount: state.rows.length + (state.error != null ? 1 : 0),
              itemBuilder: (context, index) {
                if (state.error != null && index == state.rows.length) {
                  return Padding(
                    padding: const EdgeInsets.only(top: QxSpace.l),
                    child: InlineNotice(text: state.error!, isError: true),
                  );
                }
                return TranscriptRowBuilder(row: state.rows[index]);
              },
            ),
          ),
          const PendingInteractionBar(),
          _Composer(
            controller: _composer,
            sending: state.sending,
            onSend: _send,
            onChanged: () => setState(() {}),
          ),
        ],
      ),
    );
  }

  bool _nearBottom() {
    if (!_scroll.hasClients) return true;
    return _scroll.position.pixels >= _scroll.position.maxScrollExtent - 160;
  }
}

/// 输入栏。
class _Composer extends StatelessWidget {
  const _Composer({
    required this.controller,
    required this.sending,
    required this.onSend,
    required this.onChanged,
  });

  final TextEditingController controller;
  final bool sending;
  final VoidCallback onSend;
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context) {
    final hasText = controller.text.trim().isNotEmpty;

    return Container(
      decoration: BoxDecoration(
        color: context.qx.surface,
        border: Border(top: BorderSide(color: context.qx.hairline)),
      ),
      child: SafeArea(
        top: false,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(
            QxSpace.m,
            QxSpace.s,
            QxSpace.m,
            QxSpace.s,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
              Expanded(
                child: TextField(
                  controller: controller,
                  style: QxType.body,
                  minLines: 1,
                  maxLines: 6,
                  textInputAction: TextInputAction.newline,
                  keyboardType: TextInputType.multiline,
                  onChanged: (_) => onChanged(),
                  onSubmitted: (_) {
                    if (hasText) onSend();
                  },
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
              const SizedBox(width: QxSpace.s),
              _SendButton(
                enabled: hasText && !sending,
                busy: sending,
                onPressed: onSend,
              ),
            ],
          ),
        ),
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
    return AnimatedContainer(
      duration: QxMotion.quick,
      curve: QxMotion.enter,
      width: 44,
      height: 44,
      decoration: BoxDecoration(
        // 可发送时用 accent 填充。**状态色不得进按钮**——按钮是
        // 「可操作」，不是「当前状态」，混进来会让用户以为这个按钮
        // 代表着 agent 的状态。
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
                    // `Colors.white`，在暗色 accent `#12B5A8` 上只有 2.56:1，
                    // 比它替换掉的箭头还看不见——而箭头早就改对了，
                    // spinner 被漏了一次。
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
    );
  }
}

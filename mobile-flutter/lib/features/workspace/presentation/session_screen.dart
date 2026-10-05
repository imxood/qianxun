import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../app/theme/qx_semantic.dart';
import '../../../app/theme/qx_test_ids.dart';
import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../shared/widgets/primitives.dart';
import '../../connections/application/connections_controller.dart';
import '../../sessions/presentation/link_status_bar.dart';
import '../application/transcript_controller.dart';
import 'composer_bar.dart';
import 'pending_interaction_bar.dart';
import 'transcript_view.dart';

/// 会话详情（原生 DSH 页面）。
class SessionScreen extends ConsumerStatefulWidget {
  const SessionScreen({super.key, required this.sessionId, this.title});

  final String sessionId;
  final String? title;

  @override
  ConsumerState<SessionScreen> createState() => _SessionScreenState();
}

class _SessionScreenState extends ConsumerState<SessionScreen> {
  final _scroll = ScrollController();

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
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
            // 锚点：uiautomator2 靠它确认「进到了会话详情页」。
            child: Semantics(
              identifier: QxTestIds.transcript,
              explicitChildNodes: true,
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
          ),
          const PendingInteractionBar(),
          // ⚠️ 附件 + steer 全在这里。旧的裸 TextEditingController 版本
          // 让 `ComposerController` 整块逻辑（包括图片/文件上传）
          // 没有任何 UI 调用方 —— 那等于「支持图片与文件的发送」
          // 在界面上不可达。
          ComposerBar(
            sessionId: widget.sessionId,
            running: state.running,
            onSent: _scrollToEnd,
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

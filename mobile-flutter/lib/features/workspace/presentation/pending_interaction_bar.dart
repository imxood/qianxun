import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../data/models/content.dart';
import '../../../data/models/session.dart';
import '../../../shared/widgets/primitives.dart';
import '../application/interaction_controller.dart';

/// 待答交互提示条。
///
/// 固定贴在底部、压在输入框之上。这是整个 App 里**唯一**允许抢焦点的
/// 东西——因为它是阻塞的：agent 在等一个答案，你不答它就不动。
/// 做成被动 toast 会让用户完全可能错过。
class PendingInteractionBar extends ConsumerWidget {
  const PendingInteractionBar({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(interactionControllerProvider);
    final head = state.head;
    if (head == null) return const SizedBox.shrink();
    final total = state.approvals.length + state.questions.length;
    final theme = Theme.of(context);

    return Material(
      color: QxColor.raised(theme.brightness),
      child: Container(
        decoration: BoxDecoration(
          border: Border(
            top: BorderSide(color: QxColor.hairline(theme.brightness)),
          ),
        ),
        child: InkWell(
          onTap: () => _open(context, head),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              QxSpace.l,
              QxSpace.m,
              QxSpace.l,
              QxSpace.m,
            ),
            child: Row(
              children: [
                _UrgentGlyph(
                  isApproval: head is PendingApproval,
                  color: head is PendingApproval
                      ? QxColor.pending(theme.brightness)
                      : theme.colorScheme.primary,
                ),
                const SizedBox(width: QxSpace.m),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(
                        switch (head) {
                          PendingApproval(:final request) => '需要你批准：${request.toolName}',
                          PendingQuestion(:final request) => request.questions.length == 1
                              ? 'agent 有 1 个问题'
                              : 'agent 有 ${request.questions.length} 个问题',
                        },
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: QxType.body.copyWith(color: theme.colorScheme.onSurface),
                      ),
                      if (total > 1) ...[
                        const SizedBox(height: 2),
                        Text(
                          '共 $total 项等待处理',
                          style: QxType.caption.copyWith(color: theme.colorScheme.onSurfaceVariant),
                        ),
                      ],
                    ],
                  ),
                ),
                const SizedBox(width: QxSpace.s),
                Icon(
                  Icons.chevron_right,
                  size: 20,
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  void _open(BuildContext context, PendingInteraction item) {
    final controller = ProviderScope.containerOf(context, listen: false)
        .read(interactionControllerProvider.notifier);
    switch (item) {
      case PendingApproval a:
        showApprovalSheet(context, controller, a);
      case PendingQuestion q:
        showQuestionSheet(context, controller, q);
    }
  }
}

/// 紧急提示的图形标记。审批是实心方块（阻断），提问是描边（可延后）。
class _UrgentGlyph extends StatelessWidget {
  const _UrgentGlyph({required this.isApproval, required this.color});
  final bool isApproval;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 32,
      height: 32,
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.14),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: color.withValues(alpha: 0.4)),
      ),
      child: Icon(
        isApproval ? Icons.gpp_maybe_outlined : Icons.help_outline,
        size: 17,
        color: color,
      ),
    );
  }
}

/// 审批弹层。
Future<void> showApprovalSheet(
  BuildContext context,
  InteractionController controller,
  PendingApproval item,
) {
  final theme = Theme.of(context);
  final accent = QxColor.pending(theme.brightness);

  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    builder: (sheetContext) => SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          QxSpace.l,
          0,
          QxSpace.l,
          QxSpace.l,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                MonoBadge(label: 'TOOL APPROVAL', color: accent, filled: true),
                const Spacer(),
                Text(
                  'agent ${item.agentId}',
                  style: QxType.ident.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                    fontSize: 11,
                  ),
                ),
              ],
            ),
            const SizedBox(height: QxSpace.l),
            Text(
              'agent 想调用 ${item.request.toolName}',
              style: QxType.title.copyWith(color: theme.colorScheme.onSurface),
            ),
            if (item.request.reason != null) ...[
              const SizedBox(height: QxSpace.m),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(QxSpace.m),
                decoration: theme.sunkenBox,
                child: SelectableText(
                  item.request.reason!,
                  style: QxType.caption.copyWith(color: theme.colorScheme.onSurface),
                ),
              ),
            ],
            if (item.request.callId != null) ...[
              const SizedBox(height: QxSpace.m),
              MetaRow(label: 'callId', value: item.request.callId!),
            ],
            const SizedBox(height: QxSpace.xl),
            FilledButton.icon(
              style: FilledButton.styleFrom(backgroundColor: accent, foregroundColor: Colors.black),
              onPressed: () {
                Navigator.pop(sheetContext);
                controller.answerApproval(item.eventId, 'allowed-once');
              },
              icon: const Icon(Icons.check, size: 18),
              label: const Text('允许这一次'),
            ),
            const SizedBox(height: QxSpace.s),
            OutlinedButton.icon(
              onPressed: () {
                Navigator.pop(sheetContext);
                controller.answerApproval(item.eventId, 'rejected');
              },
              icon: const Icon(Icons.close, size: 18),
              label: const Text('拒绝'),
            ),
            const SizedBox(height: QxSpace.s),
          ],
        ),
      ),
    ),
  );
}

/// 提问弹层。
Future<void> showQuestionSheet(
  BuildContext context,
  InteractionController controller,
  PendingQuestion item,
) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    isDismissible: false,
    enableDrag: false,
    builder: (_) => _QuestionSheet(controller: controller, item: item),
  );
}

class _QuestionSheet extends StatefulWidget {
  const _QuestionSheet({required this.controller, required this.item});

  final InteractionController controller;
  final PendingQuestion item;

  @override
  State<_QuestionSheet> createState() => _QuestionSheetState();
}

class _QuestionSheetState extends State<_QuestionSheet> {
  /// 每个问题的选择与自由文本。
  late final Map<String, Set<String>> _selected = {};
  late final Map<String, TextEditingController> _custom = {};

  @override
  void initState() {
    super.initState();
    for (final q in widget.item.request.questions) {
      _selected[q.id] = <String>{};
      _custom[q.id] = TextEditingController();
    }
  }

  @override
  void dispose() {
    for (final c in _custom.values) {
      c.dispose();
    }
    super.dispose();
  }

  void _submit() {
    // 宿主会逐位校验条数与 id，因此**每条都必须应答**，跳过的也给空选。
    final answers = [
      for (final q in widget.item.request.questions)
        QuestionAnswerItem(
          id: q.id,
          selected: _selected[q.id]!.toList(growable: false),
          custom: _custom[q.id]!.text,
        ),
    ];
    widget.controller.answerQuestion(widget.item.eventId, answers);
    Navigator.pop(context);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final questions = widget.item.request.questions;
    final inset = MediaQuery.of(context).viewInsets.bottom;

    return Padding(
      padding: EdgeInsets.fromLTRB(
        QxSpace.l,
        0,
        QxSpace.l,
        QxSpace.l + inset,
      ),
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.of(context).size.height * 0.82,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                MonoBadge(
                  label: 'AGENT QUESTION',
                  color: theme.colorScheme.primary,
                  filled: true,
                ),
              ],
            ),
            const SizedBox(height: QxSpace.l),
            Flexible(
              child: ListView(
                shrinkWrap: true,
                children: [
                  for (final q in questions) ...[
                    _QuestionCard(
                      question: q,
                      selected: _selected[q.id]!,
                      customController: _custom[q.id]!,
                      onToggle: (label) => setState(() {
                        final multi = q.multiSelect == true;
                        if (multi) {
                          _selected[q.id]!.contains(label)
                              ? _selected[q.id]!.remove(label)
                              : _selected[q.id]!.add(label);
                        } else {
                          _selected[q.id] = {label};
                        }
                      }),
                    ),
                    const SizedBox(height: QxSpace.m),
                  ],
                ],
              ),
            ),
            const SizedBox(height: QxSpace.m),
            FilledButton(onPressed: _submit, child: const Text('提交回答')),
            const SizedBox(height: QxSpace.xs),
            TextButton(
              onPressed: () {
                widget.controller.dismissQuestion(widget.item.eventId);
                Navigator.pop(context);
              },
              child: Text(
                '跳过这些问题',
                style: QxType.action.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
            ),
            const SizedBox(height: QxSpace.s),
          ],
        ),
      ),
    );
  }
}

class _QuestionCard extends StatelessWidget {
  const _QuestionCard({
    required this.question,
    required this.selected,
    required this.customController,
    required this.onToggle,
  });

  final QuestionItem question;
  final Set<String> selected;
  final TextEditingController customController;
  final ValueChanged<String> onToggle;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      padding: const EdgeInsets.all(QxSpace.l),
      decoration: theme.groupBox,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (question.header != null) ...[
            Text(
              question.header!.toUpperCase(),
              style: QxType.overlineIn(theme.colorScheme.onSurfaceVariant),
            ),
            const SizedBox(height: QxSpace.s),
          ],
          // plan-review 的问题正文是 markdown 计划，纯文本展示即可，
          // 但要保留换行——计划通常是多行结构。
          SelectableText(
            question.question,
            style: QxType.prose.copyWith(color: theme.colorScheme.onSurface),
          ),
          if (question.detail != null) ...[
            const SizedBox(height: QxSpace.m),
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(QxSpace.m),
              decoration: theme.sunkenBox,
              child: Text(
                question.detail!,
                style: QxType.caption.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
            ),
          ],
          if (question.options.isNotEmpty) ...[
            const SizedBox(height: QxSpace.m),
            for (final option in question.options)
              _OptionRow(
                label: option.label,
                description: option.description,
                selected: selected.contains(option.label),
                // plan-review：只有「批准」那个选项是正向的，其他都是
                // 拒绝。用颜色与文案区分，避免用户误点。
                isApprove: question.isPlanReview && option.label == question.planReviewApprove,
                onTap: () => onToggle(option.label),
              ),
          ],
          const SizedBox(height: QxSpace.m),
          TextField(
            controller: customController,
            style: QxType.body,
            minLines: 1,
            maxLines: 4,
            decoration: InputDecoration(
              hintText: question.options.isEmpty ? '你的回答…' : '其他（可选）',
              isDense: true,
            ),
          ),
        ],
      ),
    );
  }
}

class _OptionRow extends StatelessWidget {
  const _OptionRow({
    required this.label,
    required this.selected,
    required this.onTap,
    this.description,
    this.isApprove = false,
  });

  final String label;
  final String? description;
  final bool selected;
  final bool isApprove;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final accent = isApprove ? QxColor.running(theme.brightness) : theme.colorScheme.primary;

    return Padding(
      padding: const EdgeInsets.only(bottom: QxSpace.xs),
      child: Material(
        color: selected ? accent.withValues(alpha: 0.12) : QxColor.sunken(theme.brightness),
        borderRadius: QxRadius.controlAll,
        child: InkWell(
          onTap: onTap,
          borderRadius: QxRadius.controlAll,
          child: Container(
            padding: const EdgeInsets.symmetric(
              horizontal: QxSpace.m,
              vertical: QxSpace.m,
            ),
            decoration: BoxDecoration(
              borderRadius: QxRadius.controlAll,
              border: Border.all(
                color: selected ? accent : QxColor.hairline(theme.brightness),
              ),
            ),
            child: Row(
              children: [
                Icon(
                  selected ? Icons.check_circle : Icons.circle_outlined,
                  size: 18,
                  color: selected ? accent : theme.colorScheme.onSurfaceVariant,
                ),
                const SizedBox(width: QxSpace.m),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        label,
                        style: QxType.body.copyWith(color: theme.colorScheme.onSurface),
                      ),
                      if (description != null) ...[
                        const SizedBox(height: 2),
                        Text(
                          description!,
                          style: QxType.caption.copyWith(color: theme.colorScheme.onSurfaceVariant),
                        ),
                      ],
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

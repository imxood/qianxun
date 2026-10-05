import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../data/models/content.dart';
import '../../../data/models/session.dart';
import '../../../shared/widgets/primitives.dart';
import '../application/interaction_controller.dart';
import '../../../app/theme/qx_theme.dart';
import '../../../../app/theme/qx_semantic.dart';

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
      color: context.qx.surfaceRaised,
      child: Container(
        decoration: BoxDecoration(
          border: Border(
            top: BorderSide(color: context.qx.hairline),
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
                      ? context.qx.waiting
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
  // ⚠️ 按钮底必须是 **accent**，不能是 `waiting` 琥珀。
  // 05 §6.2：「允许这一次」accent 填充，且末行明确「状态色不进按钮填充」——
  // 琥珀是「它在等你」这个**状态**的色，拿来当按钮底会让「状态」和
  // 「可点的承诺」混成一个视觉信号，用户会以为批准本身就是状态。
  //
  // 顺带修掉一个真对比度 bug：这里原来写死 `Colors.black`，亮色板下
  // waiting `#996300` 上只有 4.14:1（低于 AA 的 4.5）。accent + onAccent
  // 由 check_contrast.py 守住（实测 5.13:1）。
  final accent = context.qx.accent;
  final onAccent = context.qx.onAccent;

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
                // 05 §8.5：全中文，只有代码 / 路径 / 工具名保留原文。
                MonoBadge(label: '工具审批', color: accent, filled: true),
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
              style: FilledButton.styleFrom(
                backgroundColor: accent,
                foregroundColor: onAccent,
              ),
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
                  label: 'agent 提问',
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
              style: QxType.overline.copyWith(color: theme.colorScheme.onSurfaceVariant),
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
                // 曾经这里对 plan-review 的「批准」选项单独上 `running` 绿，
                // 注释说是「避免用户误点」。已删：它只在**选中之后**改色，
                // 防不了误点，却把会话状态色（运行中）塞进了一个选择菜单，
                // 还违反 05 §6.2「状态色不进按钮填充」。选中与否由勾选图标 +
                // 描边 + 底色三条通道表达，够清楚了。
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
  });

  final String label;
  final String? description;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // 选中与否统一用 accent：它表示「可点 / 已选中」，不表示状态。
    final accent = context.qx.accent;

    return Padding(
      padding: const EdgeInsets.only(bottom: QxSpace.xs),
      child: Material(
        color: selected ? accent.withValues(alpha: 0.12) : context.qx.surfaceSunken,
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
                color: selected ? accent : context.qx.hairline,
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

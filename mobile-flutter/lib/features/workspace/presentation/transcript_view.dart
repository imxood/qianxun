import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_markdown/flutter_markdown.dart';

import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../data/models/content.dart';
import '../../../shared/widgets/primitives.dart';
import '../application/transcript_controller.dart';
import '../../../app/theme/qx_theme.dart';
import '../../../../app/theme/qx_semantic.dart';

/// 转录视图。
///
/// 这一块是「完全定制的 DSH 页面」的核心：不是把桌面页面缩小，而是
/// 按**手机的信息密度**重新排布。核心决定：
///
/// - 用户消息右对齐、带底色——因为「我刚说了什么」需要能被一眼定位。
/// - 模型消息**不**套气泡：agent 的输出往往很长（代码、计划、列表），
///   气泡会把可用宽度再砍掉 24px 并制造大量视觉噪声。
/// - 思维链默认折叠、弱化：它是模型的内心戏，不是结论。
/// - 工具调用渲染成**紧凑的状态卡**，参数与输出默认折叠：agent 跑
///   `rg` 搜 300 个文件时，手机上不该滚过 300 行输出。
class TranscriptView extends StatelessWidget {
  const TranscriptView({super.key, required this.state});

  final TranscriptState state;

  @override
  Widget build(BuildContext context) {
    if (state.loading && state.rows.isEmpty) {
      return const Center(
        child: SizedBox(
          width: 22,
          height: 22,
          child: CircularProgressIndicator(strokeWidth: 2),
        ),
      );
    }
    if (state.rows.isEmpty) {
      return const EmptyHero(
        title: '这段对话还是空的',
        hint: '在下面发一条消息，agent 会在你的电脑上开始工作。',
        icon: Icons.chat_bubble_outline,
      );
    }

    return ListView.builder(
      padding: const EdgeInsets.fromLTRB(
        QxSpace.l,
        QxSpace.m,
        QxSpace.l,
        QxSpace.xxl,
      ),
      // 每行高度差异很大，预构建几屏。
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
        return _RowTile(row: state.rows[index]);
      },
    );
  }
}

class _RowTile extends StatelessWidget {
  const _RowTile({required this.row});
  final TranscriptRow row;

  @override
  Widget build(BuildContext context) {
    return switch (row) {
      RowUserMessage r => _UserBubble(message: r.message, at: r.at),
      RowAssistantMessage r => _AssistantBlock(message: r.message, usage: r.usage),
      RowToolCall r => ToolCallCard(row: r),
      RowTodo r => TodoCard(todos: r.todos),
      RowTurnDivider r => TurnDivider(turn: r.turn),
      RowNotice r => NoticeRow(label: r.label, detail: r.detail),
    };
  }
}

/// 渲染一行转录。
///
/// 抽成顶层函数是为了让会话页能在一个自带的 `ListView` 里复用它——
/// 会话页需要自己持有 `ScrollController` 才能实现「用户上翻时不抢回底部」
/// 这个行为，而 `TranscriptView` 内部已经有一个了。
class TranscriptRowBuilder extends StatelessWidget {
  const TranscriptRowBuilder({super.key, required this.row});

  final TranscriptRow row;

  @override
  Widget build(BuildContext context) => _RowTile(row: row);
}

/// 用户气泡。右对齐 + 主色淡底。
class _UserBubble extends StatelessWidget {
  const _UserBubble({required this.message, required this.at});
  final MessageData message;
  final DateTime at;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: QxSpace.m),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          ConstrainedBox(
            constraints: BoxConstraints(
              maxWidth: MediaQuery.of(context).size.width * 0.82,
            ),
            child: Container(
              padding: const EdgeInsets.symmetric(
                horizontal: QxSpace.m + QxSpace.xxs,
                vertical: QxSpace.s + QxSpace.xxs,
              ),
              decoration: BoxDecoration(
                color: theme.colorScheme.primary.withValues(alpha: 0.14),
                borderRadius: const BorderRadius.only(
                  topLeft: QxRadius.bubble,
                  topRight: Radius.circular(QxSpace.xs),
                  bottomLeft: QxRadius.bubble,
                  bottomRight: QxRadius.bubble,
                ),
              ),
              child: SelectionArea(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    for (final block in message.content)
                      if (block is ContentText)
                        Text(
                          block.text,
                          style: QxType.prose.copyWith(color: theme.colorScheme.onSurface),
                        ),
                  ],
                ),
              ),
            ),
          ),
          const SizedBox(height: QxSpace.xxs),
          Padding(
            padding: const EdgeInsets.only(right: QxSpace.xs),
            child: Text(
              _clock(at),
              style: QxType.micro.copyWith(
                color: theme.colorScheme.onSurfaceVariant.withValues(alpha: 0.55),
                fontSize: 10,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// 助手块。无气泡，全宽。
class _AssistantBlock extends StatelessWidget {
  const _AssistantBlock({required this.message, this.usage});
  final MessageData message;
  final TokenUsage? usage;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final reasoning = message.content.whereType<ContentReasoning>().toList();
    final blocks = message.content
        .where((b) => b is! ContentReasoning && b is! ContentToolCall && b is! ContentToolResult)
        .toList();

    return Padding(
      padding: const EdgeInsets.only(bottom: QxSpace.l),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (reasoning.isNotEmpty) ...[
            _ReasoningBlock(items: reasoning),
            const SizedBox(height: QxSpace.s),
          ],
          if (blocks.isNotEmpty)
            SelectionArea(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (final block in blocks)
                    if (block is ContentText) _MarkdownBody(text: block.text),
                ],
              ),
            ),
          if (message.source.model != null) ...[
            const SizedBox(height: QxSpace.s),
            Row(
              children: [
                Text(
                  message.source.model!,
                  style: QxType.micro.copyWith(
                    color: theme.colorScheme.onSurfaceVariant.withValues(alpha: 0.5),
                    fontSize: 10,
                  ),
                ),
                if (usage != null && usage!.outputTokens > 0) ...[
                  Text(
                    '  ·  ↑${formatTokens(usage!.outputTokens)}',
                    style: QxType.micro.copyWith(
                      color: theme.colorScheme.onSurfaceVariant.withValues(alpha: 0.5),
                      fontSize: 10,
                    ),
                  ),
                ],
              ],
            ),
          ],
        ],
      ),
    );
  }
}

/// 思维链。默认折叠——它是过程，不是结论。
class _ReasoningBlock extends StatelessWidget {
  const _ReasoningBlock({required this.items});
  final List<ContentReasoning> items;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Material(
      // ⚠️ 必须用 Material 承载表面，**不能**用 Container(decoration:)。
      //
      // ListTile 把背景与墨水飞溅画在**最近的 Material 祖先**上。如果中间
      // 隔一个带底色的 Container，飞溅就画在更外层、被这个 Container 整个
      // 盖住——表现是点下去毫无反馈。Flutter 自己在 debug 下会为此断言。
      //
      // 同理也不能只设 ListTileTheme 的 tileColor 为透明：那只是让底色不
      // 遮挡，涟漪仍然画在外层 Material 上。唯一正确的解法是让这个
      // Material 本身成为「最近的 Material 祖先」。
      color: context.qx.surfaceSunken,
      clipBehavior: Clip.antiAlias,
      shape: RoundedRectangleBorder(
        borderRadius: QxRadius.controlAll,
        side: BorderSide(color: context.qx.hairline),
      ),
      child: Theme(
        data: theme.copyWith(dividerColor: Colors.transparent),
        child: ExpansionTile(
          collapsedIconColor: theme.colorScheme.onSurfaceVariant,
          iconColor: theme.colorScheme.onSurfaceVariant,
          tilePadding: const EdgeInsets.symmetric(horizontal: QxSpace.m),
          childrenPadding: const EdgeInsets.fromLTRB(
            QxSpace.m,
            0,
            QxSpace.m,
            QxSpace.m,
          ),
          title: Row(
            children: [
              Icon(
                Icons.psychology_outlined,
                size: 15,
                color: theme.colorScheme.onSurfaceVariant,
              ),
              const SizedBox(width: QxSpace.s),
              Text(
                '思维链',
                style: QxType.caption.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
          children: [
            for (final item in items)
              Padding(
                padding: const EdgeInsets.only(bottom: QxSpace.s),
                child: Text(
                  item.text,
                  style: QxType.caption.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                    height: 1.6,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// 工具执行卡。
///
/// 参数与输出默认折叠，但**状态永远可见**。这是给「agent 现在在干什么」
/// 用的：你在地铁上只想知道它还活着没有、在跑什么，而不是读 300 行
/// grep 结果。
class ToolCallCard extends StatelessWidget {
  const ToolCallCard({super.key, required this.row});
  final RowToolCall row;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final q = theme.qx;
    // 失败有两种来源：显式的 error 引用，或结果块里 isError。
    final failed = row.error != null ||
        (row.result?.message.content.whereType<ContentToolResult>().any((b) => b.isError == true) ??
            false);
    // 工具卡的态只有三种，与会话的五态不是一回事。
    final state = switch ((row.isRunning, failed)) {
      (true, _) => QxSessionState.running,
      (false, true) => QxSessionState.failed,
      _ => QxSessionState.done,
    };
    final accent = state.colorOf(q);

    final output = row.result?.message.content
        .whereType<ContentToolResult>()
        .map((b) => b.content.map((c) => c.plainText).join('\n'))
        .join('\n');

    return Padding(
      padding: const EdgeInsets.only(bottom: QxSpace.s),
      child: Material(
        // 同上：用 Material 承载表面，让涟漪有地方画。
        color: context.qx.surfaceSunken,
        clipBehavior: Clip.antiAlias,
        shape: RoundedRectangleBorder(
          borderRadius: QxRadius.controlAll,
          side: BorderSide(color: context.qx.hairline),
        ),
        child: Theme(
          data: theme.copyWith(dividerColor: Colors.transparent),
          child: ExpansionTile(
            collapsedIconColor: theme.colorScheme.onSurfaceVariant,
            iconColor: theme.colorScheme.onSurfaceVariant,
            tilePadding: const EdgeInsets.symmetric(horizontal: QxSpace.m),
            childrenPadding: const EdgeInsets.fromLTRB(
              QxSpace.m,
              0,
              QxSpace.m,
              QxSpace.m,
            ),
            title: Row(
              children: [
                // 05 §8.4：运行态**不转圈**。
                // 原来这里是 12dp 的 CircularProgressIndicator，它带来两个
                // 代价：默认态的转录里有了一个无限动画（`pumpAndSettle`
                // 必然超时，测试只能绕），以及一个仅靠运动表意的状态 ——
                // 运动是唯一不能被灰度截图与色觉障碍读到的通道。
                // 换成静态 StateDot（自带字形）＋ 右侧已有的「运行中」角标，
                // 两条静态通道，信息一点没少。
                StateDot(state: state),
                const SizedBox(width: QxSpace.s),
                Expanded(
                  child: Text(
                    row.call.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: QxType.ident.copyWith(
                      color: theme.colorScheme.onSurface,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),
                if (row.call.arguments.isNotEmpty && _isLong(row.call.arguments))
                  MonoBadge(
                    label: '${row.call.arguments.length}B',
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                const SizedBox(width: QxSpace.xs),
                MonoBadge(label: state.label, color: accent, filled: true),
              ],
            ),
            // 错误码必须在**展开前**可读。折叠里是过程信息，状态才是结论：
            // 地铁上你只想知道它失败了没有、为什么失败。
            subtitle: row.error == null
                ? null
                : Padding(
                    padding: const EdgeInsets.only(top: 3, left: 20),
                    child: Text(
                      row.error!.code,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: QxType.ident.copyWith(
                        color: q.failed,
                        fontSize: 11,
                      ),
                    ),
                  ),
            children: [
              if (row.call.arguments.isNotEmpty) ...[
                _CodePanel(
                  title: 'arguments',
                  content: _prettyJson(row.call.arguments),
                  dim: true,
                ),
                const SizedBox(height: QxSpace.s),
              ],
              if (output != null && output.isNotEmpty)
                _CodePanel(
                  title: row.error != null ? 'error' : 'output',
                  content: output,
                  dim: row.error != null,
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 等宽代码面板。横向滚动而非换行——命令与路径换行后完全不可读。
class _CodePanel extends StatelessWidget {
  const _CodePanel({
    required this.title,
    required this.content,
    this.dim = false,
  });

  final String title;
  final String content;
  final bool dim;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          title.toUpperCase(),
          style: QxType.overline.copyWith(color: 
            dim ? context.qx.failed : theme.colorScheme.onSurfaceVariant,
          ).copyWith(fontSize: 9.5),
        ),
        const SizedBox(height: QxSpace.xs),
        ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 260),
          child: Container(
            width: double.infinity,
            padding: const EdgeInsets.all(QxSpace.s + QxSpace.xxs),
            decoration: BoxDecoration(
              color: context.qx.surfaceSunken,
              borderRadius: BorderRadius.circular(QxSpace.s),
              border: Border.all(color: context.qx.hairline),
            ),
            child: Scrollbar(
              child: SingleChildScrollView(
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: SelectableText(
                    content,
                    style: QxType.code.copyWith(
                      color: dim
                          ? context.qx.failed
                          : theme.colorScheme.onSurface.withValues(alpha: 0.88),
                      fontSize: 12,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// 待办卡。
class TodoCard extends StatelessWidget {
  const TodoCard({super.key, required this.todos});
  final List<TodoItem> todos;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final done = todos.where((t) => t.isDone).length;

    return Padding(
      padding: const EdgeInsets.only(bottom: QxSpace.s),
      child: Container(
        padding: const EdgeInsets.all(QxSpace.m),
        decoration: theme.groupBox,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.checklist, size: 14, color: theme.colorScheme.onSurfaceVariant),
                const SizedBox(width: QxSpace.s),
                Text(
                  '待办',
                  style: QxType.caption.copyWith(
                    color: theme.colorScheme.onSurface,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                const Spacer(),
                Text(
                  '$done/${todos.length}',
                  style: QxType.ident.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                    fontSize: 11,
                  ),
                ),
              ],
            ),
            const SizedBox(height: QxSpace.s),
            for (final todo in todos)
              Padding(
                padding: const EdgeInsets.only(bottom: 3),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Icon(
                      todo.isDone
                          ? Icons.check_box
                          : todo.isActive
                              ? Icons.pending_outlined
                              : Icons.check_box_outline_blank,
                      size: 14,
                      color: todo.isDone
                          ? context.qx.done
                          : todo.isActive
                              ? context.qx.running
                              : theme.colorScheme.onSurfaceVariant,
                    ),
                    const SizedBox(width: QxSpace.s),
                    Expanded(
                      child: Text(
                        todo.content,
                        style: QxType.caption.copyWith(
                          color: todo.isDone
                              ? theme.colorScheme.onSurfaceVariant
                              : theme.colorScheme.onSurface,
                          decoration: todo.isDone ? TextDecoration.lineThrough : null,
                          decorationColor: theme.colorScheme.onSurfaceVariant,
                        ),
                      ),
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

/// 轮次分隔。
class TurnDivider extends StatelessWidget {
  const TurnDivider({super.key, required this.turn});
  final int turn;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: QxSpace.m),
      child: Row(
        children: [
          Expanded(child: Divider(color: context.qx.hairline)),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: QxSpace.m),
            child: Text(
              'TURN $turn',
              style: QxType.overline.copyWith(
                color: theme.colorScheme.onSurfaceVariant.withValues(alpha: 0.5),
              ),
            ),
          ),
          Expanded(child: Divider(color: context.qx.hairline)),
        ],
      ),
    );
  }
}

/// 轻量通知行（命令、改名等）。
class NoticeRow extends StatelessWidget {
  const NoticeRow({super.key, required this.label, this.detail});
  final String label;
  final String? detail;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: QxSpace.xs),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(top: 5),
            child: Container(
              width: 3,
              height: 12,
              color: theme.colorScheme.onSurfaceVariant.withValues(alpha: 0.35),
            ),
          ),
          const SizedBox(width: QxSpace.s),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  label,
                  style: QxType.caption.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                    fontWeight: FontWeight.w600,
                  ),
                ),
                if (detail != null && detail!.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(top: 2),
                    child: Text(
                      detail!,
                      maxLines: 3,
                      overflow: TextOverflow.ellipsis,
                      style: QxType.code.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                        fontSize: 11.5,
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// Markdown 正文。
///
/// 用了 `flutter_markdown` 但**换掉了整套样式**：默认样式的标题过大、
/// 引用框太重，在手机上会把一段解释顶出三屏。工具类界面里，markdown
/// 的作用是「可读的层级」，不是「漂亮的排版」。
class _MarkdownBody extends StatelessWidget {
  const _MarkdownBody({required this.text});
  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final on = theme.colorScheme.onSurface;
    final dim = theme.colorScheme.onSurfaceVariant;

    return Padding(
      padding: const EdgeInsets.only(bottom: QxSpace.s),
      child: MarkdownBody(
        data: text,
        selectable: false,
        styleSheet: MarkdownStyleSheet(
          p: QxType.prose.copyWith(color: on),
          h1: QxType.title.copyWith(fontSize: 19, color: on, height: 1.3),
          h2: QxType.title.copyWith(fontSize: 17, color: on, height: 1.3),
          h3: QxType.body.copyWith(fontWeight: FontWeight.w700, color: on, height: 1.35),
          h4: QxType.body.copyWith(fontWeight: FontWeight.w700, color: on),
          h5: QxType.body.copyWith(color: dim),
          h6: QxType.caption.copyWith(color: dim),
          em: QxType.prose.copyWith(color: on, fontStyle: FontStyle.italic),
          strong: QxType.prose.copyWith(color: on, fontWeight: FontWeight.w700),
          listBullet: QxType.prose.copyWith(color: on),
          code: QxType.code.copyWith(
            color: on,
            fontSize: 12.5,
            decoration: TextDecoration.underline,
            decorationColor: dim,
          ),
          codeblockDecoration: BoxDecoration(
            color: context.qx.surfaceSunken,
            borderRadius: BorderRadius.circular(QxSpace.s),
            border: Border.all(color: context.qx.hairline),
          ),
          blockquoteDecoration: BoxDecoration(
            color: dim.withValues(alpha: 0.07),
            borderRadius: BorderRadius.circular(QxSpace.s),
            border: Border(
              left: BorderSide(color: theme.colorScheme.primary, width: 2.5),
            ),
          ),
          blockquote: QxType.prose.copyWith(color: dim),
          blockquotePadding: const EdgeInsets.fromLTRB(
            QxSpace.m,
            QxSpace.s,
            QxSpace.m,
            QxSpace.s,
          ),
          horizontalRuleDecoration: BoxDecoration(
            border: Border(top: BorderSide(color: context.qx.hairline)),
          ),
          tableBorder: TableBorder.all(color: context.qx.hairline, width: 1),
          tableHead: QxType.caption.copyWith(fontWeight: FontWeight.w700, color: on),
          tableBody: QxType.caption.copyWith(color: on),
          tableCellsPadding: const EdgeInsets.all(QxSpace.s),
          a: QxType.prose.copyWith(
            color: theme.colorScheme.primary,
            decoration: TextDecoration.underline,
            decorationColor: theme.colorScheme.primary,
          ),
        ),
      ),
    );
  }
}

/// 工具参数的 JSON 美化。失败时原样返回。
String _prettyJson(String raw) {
  final decoded = tryDecodeJsonLoose(raw);
  if (decoded is! Map && decoded is! List) return raw;
  try {
    return const JsonEncoder.withIndent('  ').convert(decoded);
  } catch (_) {
    return raw;
  }
}

/// 参数是否长到值得在折叠标题上标一个体量。
bool _isLong(String s) => s.length > 160;

Object? tryDecodeJsonLoose(String body) {
  if (body.trim().isEmpty) return null;
  try {
    return const JsonDecoder().convert(body);
  } catch (_) {
    return null;
  }
}

String _clock(DateTime t) =>
    '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';

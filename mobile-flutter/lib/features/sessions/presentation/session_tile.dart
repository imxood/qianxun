import 'package:flutter/material.dart';

import '../../../app/theme/qx_semantic.dart';
import '../../../app/theme/qx_test_ids.dart';
import '../../../app/theme/qx_tokens.dart';
import '../../../app/theme/qx_type.dart';
import '../../../shared/widgets/primitives.dart';
import '../../../shared/widgets/signal_spine.dart';
import '../application/session_presentation.dart';

/// 会话行。05 §4.2 的最终结构。
///
/// ```
/// ┌─┬──────────────────────────────────────────┐
/// │▌│ 最新代码部署测试环境                        │  ← 轨(出血) + 标题
/// │▌│ Bash  ./deploy.sh --env test                │  ← 第二行: agent 最近做的那件事
/// │▌│ 运行中 · iot_tool_cloud · 7 轮 · 365k ctx   │  ← 固定格, 永不换行
/// └─┴──────────────────────────────────────────┘
/// ```
///
/// **行高 66dp 固定**，idle 与 running 同高（05 §4.2）。这一条不是审美
/// 选择：若两态行高不同，扫一列时行的**位置**就随状态漂移，垂直方向
/// 的定位能力和水平方向的定位能力会互相打架。
///
/// 用分隔线列表而不是一排一模一样的圆角卡片（05 §4.1 的现状批评）：
/// 卡片之间那 12dp 间隙里没有任何信息，纯浪费垂直空间。
class SessionTile extends StatelessWidget {
  const SessionTile({
    super.key,
    required this.item,
    required this.onTap,
  });

  final SessionPresentation item;
  final VoidCallback onTap;

  /// 05 §4.2：轨通高 54dp，不满格。
  static const railHeight = 54.0;

  /// 05 §4.2：行高固定 66dp，一屏 10 行。
  static const rowHeight = 66.0;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final q = context.qx;
    final session = item.session;
    final state = item.state;

    return Semantics(
      button: true,
      // 整行读一遍就够：名称 + 状态。轨与状态点自己**不再标 Semantics**
      // （轨内部已 ExcludeSemantics），否则同一个词会被读三遍。
      label: '${session.displayTitle}，${state.label}',
      // 给原生自动化留的稳定锚点（uiautomator2 读得到 resource-id）。
      // 详见 QxTestIds 的说明。**不影响**上面的朗读内容。
      identifier: QxTestIds.sessionTile,
      child: Material(
        color: theme.colorScheme.surface,
        child: InkWell(
          onTap: onTap,
          // 涟漪只做瞬时填充，不改描边色（02 §3.3）。
          //
          // ⚠️ 用 `minHeight` 而不是 `height`。两条**都是**强制要求：
          //   05 §4.2  行高 66dp 固定，idle 与 running 同高
          //   05 §9.5  正文缩放 2.0 时零溢出
          // 写死 `height` 会在 2.0x 下溢出 52px（实测），写死不设下界
          // 又会让默认态行高随内容漂移、扫不出节奏。所以：**默认态正好
          // 66dp，字号放大时向下生长**。两者的交集就是 minHeight。
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: rowHeight),
            child: Row(
              children: [
                // 轨出血到屏幕左缘。05 §3.1 的签名级元素。
                SignalSpine(state: state, height: railHeight, gutter: QxSpace.s),
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.only(right: QxSpace.l),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      // `min` 而不是默认的 `max`：默认会在无界高度里
                      // 撑满（ListView 行里实测到 600），那就不是行高固定
                      // 而是行高随机。贴合内容 + 外层 minHeight 66 才是
                      // 「默认 66、放大才长高」。
                      mainAxisSize: MainAxisSize.min,
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Text(
                          session.displayTitle,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: QxType.title.copyWith(
                            color: theme.colorScheme.onSurface,
                            fontSize: 15.5,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        // 第二行：agent 最近做的那件事。查不到就**不占位**
                        // （05 §4.3），不要用用户输入的回声填。
                        if (item.secondLine case final line?) ...[
                          const SizedBox(height: 3),
                          Text(
                            line,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: QxType.caption.copyWith(
                              color: q.textSecondary,
                            ),
                          ),
                        ],
                        const SizedBox(height: 3),
                        _MetaCells(item: item),
                      ],
                    ),
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

/// 元信息行。05 §4.4：**固定格，永不换行**。
///
/// 用 `Wrap` 会让长路径把整行撑成两行，行的垂直节奏立刻塌掉——而这一屏
/// 是**被扫**的，扫的前提是每行的基线在同一处。所以每格 `Expanded`，
/// 溢出用 `…`。
class _MetaCells extends StatelessWidget {
  const _MetaCells({required this.item});

  final SessionPresentation item;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final q = context.qx;
    final session = item.session;
    final state = item.state;

    // 状态标签在**最前**且是唯一带色的元信息（05 §4.4）。
    //
    // ⚠️ 它**不参与**宽度均分，给自己的自然宽度。05 §4.4 说溢出用 `…`，
    // 但那句话只对**次要信息**成立——「绝不靠颜色单独表意」要求每态都配
    // 一个可见中文词，一旦状态词自己被省略成「运行…」，整条轨的语义就
    // 只剩颜色，而轨在 1dp（已完成）时颜色本身就看不清。
    // 曾经把 5 格全塞进 Expanded 均分，实测在 390dp 下每格只有 61.8dp，
    // 1.5x 起连「运行中」都被裁掉。
    final statusLabel = Text(
      state.label,
      maxLines: 1,
      softWrap: false,
      style: QxType.caption.copyWith(
        color: state.colorOf(q),
        fontWeight: FontWeight.w700,
      ),
    );

    // 次要信息：cwd 末段、轮次、ctx、相对时间。均可被省略。
    final cells = <Widget>[
      if (session.metaCwd != null) _cell(session.metaCwd!, theme: theme),
      if (session.stats case final s? when s.turns > 0)
        _cell('${s.turns} 轮', theme: theme, monospace: true),
      if (session.contextTokens case final c? when c > 0)
        _cell('${formatTokens(c)} ctx', theme: theme, monospace: true),
      _cell(relTime(session.updatedAt), theme: theme),
    ];

    if (cells.isEmpty) {
      return Row(children: [statusLabel]);
    }

    return Row(
      children: [
        statusLabel,
        for (var i = 0; i < cells.length; i++) ...[
          Text('·', style: QxType.caption.copyWith(color: q.textTertiary)),
          Expanded(child: cells[i]),
        ],
      ],
    );
  }

  Widget _cell(
    String text, {
    required ThemeData theme,
    bool monospace = false,
  }) {
    final label = Text(
      text,
      // ⚠️ `maxLines: 1` **就是**「永不换行」那条不变式的执行点。
      // 断掉它，这一格就会换行、把行撑高；留着它，放不下就出省略号。
      //
      // （别以为外层用 `Row` + `Expanded` 就能防住：实测把外层换成
      //  `Wrap` 渲染结果**完全一样**，因为每一格自己就 `maxLines: 1`。
      //  换句话说，真正的开关在这里，不在容器上。）
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      textAlign: TextAlign.left,
      style: (monospace ? QxType.metric : QxType.caption).copyWith(
        color: theme.colorScheme.onSurfaceVariant,
      ),
    );
    // 数字类单元格属于等宽数据层：放大后列会散（04 裁决表）。
    return monospace ? QxType.dataScaleLocked(child: label) : label;
  }
}

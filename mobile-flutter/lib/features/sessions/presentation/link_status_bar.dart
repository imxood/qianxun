import 'package:flutter/material.dart';

import '../../../app/theme/qx_type.dart';
import '../../../data/dsh/dsh_connection.dart';
import '../../../../app/theme/qx_semantic.dart';
import '../../../app/theme/qx_tokens.dart';

/// 连接状态条。
///
/// 只在**非就绪**时出现。这是刻意的：一个永远占位的「已连接」条会吃掉
/// 40 像素并且让界面永远显得有事发生。连接健康时，用户不需要被告知。
class LinkStatusBar extends StatelessWidget {
  const LinkStatusBar({super.key, required this.link, required this.onRetry});

  final DshConnection? link;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final state = link?.state;
    if (state == null) return const SizedBox.shrink();

    final (message, action) = switch (state) {
      // LinkIdle 正常情况下不可达（provider 一建好就 start() 了）。
      // 它还停在这里说明连接压根没被启动过——给一个明确的重连按钮，
      // 不要再转一个永远停不下来的圈。
      LinkIdle() => ('连接未启动', _Action(label: '连接', onTap: onRetry)),
      LinkConnecting(:final attempt) => (
          attempt == 0 ? '正在连接电脑…' : '正在重连（第 $attempt 次）…',
          null,
        ),
      LinkReconnecting(:final attempt, :final nextRetryIn, :final cause) => (
          '$cause · ${(nextRetryIn.inMilliseconds / 1000).ceil()}s 后第 $attempt 次重试',
          _Action(label: '立即重试', onTap: onRetry),
        ),
      // 凭据被拒：重试无用，只能重新配对，所以不给「重试」而是给指引。
      LinkNeedsPairing(:final message) => (
          message,
          _Action(label: '去重新配对', onTap: onRetry),
        ),
      LinkClosed() => ('已断开', _Action(label: '重新连接', onTap: onRetry)),
      LinkReady() => ('', null),
    };

    if (message.isEmpty) return const SizedBox.shrink();

    final theme = Theme.of(context);
    final isFatal = state is LinkNeedsPairing;
    final color = isFatal ? theme.colorScheme.error : theme.colorScheme.onSurfaceVariant;

    return Container(
      width: double.infinity,
      color: isFatal
          ? theme.colorScheme.error.withValues(alpha: 0.10)
          : context.qx.surfaceSunken,
      padding: const EdgeInsets.fromLTRB(QxSpace.l, QxSpace.s, QxSpace.m, QxSpace.s),
      child: Row(
        children: [
          if (!isFatal)
            SizedBox(
              width: 12,
              height: 12,
              child: CircularProgressIndicator(strokeWidth: 1.6, color: color),
            )
          else
            Icon(Icons.error_outline, size: 14, color: color),
          const SizedBox(width: QxSpace.s),
          Expanded(
            child: Text(
              message,
              style: QxType.caption.copyWith(color: color),
            ),
          ),
          if (action != null)
            TextButton(
              onPressed: action.onTap,
              style: TextButton.styleFrom(
                minimumSize: const Size(0, 32),
                padding: const EdgeInsets.symmetric(horizontal: QxSpace.m),
                textStyle: QxType.caption.copyWith(fontWeight: FontWeight.w600),
              ),
              child: Text(action.label),
            ),
        ],
      ),
    );
  }
}

class _Action {
  const _Action({required this.label, required this.onTap});
  final String label;
  final VoidCallback onTap;
}

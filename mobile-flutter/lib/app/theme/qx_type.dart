import 'package:flutter/material.dart';

import 'qx_tokens.dart';

/// 千寻排版刻度。
///
/// 决策：**中文正文用系统字体，技术数据用等宽字体**。
///
/// 理由：中文没有一款「非系统」的字体能在所有 Android / iOS 机型上
/// 都拿到正确的字重与 hinting——为中文换字体的代价远大于收益，而且
/// 绝大多数中文设备上的 PingFang / 思源黑体本身观感就很好。
///
/// 但工具类 App 的性格来自**数据层**：工具名、命令、退出码、session id、
/// 耗时、行号。这些必须等宽，否则列对不齐、扫视成本极高。所以本 App
/// 把等宽字体作为签名层铺到所有数据位置，UI 文字层保持系统字体。
abstract final class QxType {
  /// 随包分发的等宽字体（见 pubspec.yaml 的 `QxMono`）。
  static const String mono = 'QxMono';

  /// 屏幕大标题。仅用于「连接」这类顶级页面。
  static const TextStyle display = TextStyle(
    fontSize: 32,
    height: 1.18,
    fontWeight: FontWeight.w600,
    letterSpacing: -0.5,
  );

  /// 页面标题。
  static const TextStyle title = TextStyle(
    fontSize: 20,
    height: 1.3,
    fontWeight: FontWeight.w600,
    letterSpacing: -0.2,
  );

  /// 分组标题（小标签、全大写/等宽的那类）。
  static const TextStyle overline = TextStyle(
    fontSize: 11,
    height: 1.2,
    fontWeight: FontWeight.w600,
    letterSpacing: 1.1,
  );

  /// 列表主行。
  static const TextStyle body = TextStyle(
    fontSize: 15.5,
    height: 1.4,
    fontWeight: FontWeight.w500,
  );

  /// 正文 / 消息正文。行高放宽到 1.55，中文长段落可读性必需。
  static const TextStyle prose = TextStyle(
    fontSize: 15.5,
    height: 1.55,
    fontWeight: FontWeight.w400,
  );

  /// 次要说明。
  static const TextStyle caption = TextStyle(
    fontSize: 13,
    height: 1.4,
    fontWeight: FontWeight.w400,
  );

  /// 最小可读字号（状态角标、时间戳）。
  static const TextStyle micro = TextStyle(
    fontSize: 11.5,
    height: 1.25,
    fontWeight: FontWeight.w500,
  );

  /// 工具输出 / 命令 / 代码。正文可读，字号比 UI 略小以容纳长行。
  static const TextStyle code = TextStyle(
    fontFamily: mono,
    fontSize: 13,
    height: 1.5,
    fontWeight: FontWeight.w400,
  );

  /// 代码加粗（强调的命令、关键字）。
  static const TextStyle codeBold = TextStyle(
    fontFamily: mono,
    fontSize: 13,
    height: 1.5,
    fontWeight: FontWeight.w700,
  );

  /// 标识符：session id、job id、token 片段。带一点字距便于逐字核对。
  static const TextStyle ident = TextStyle(
    fontFamily: mono,
    fontSize: 12,
    height: 1.35,
    fontWeight: FontWeight.w500,
    letterSpacing: 0.2,
  );

  /// 主按钮 / 关键动作。
  static const TextStyle action = TextStyle(
    fontSize: 15,
    height: 1.2,
    fontWeight: FontWeight.w600,
    letterSpacing: 0.1,
  );

  /// 把 [QxType.overline] 之类的小标签接上等宽字体。
  ///
  /// 状态角标、阶段名这类「像终端输出」的短文本走这条。
  static const TextStyle monoOverline = TextStyle(
    fontFamily: mono,
    fontSize: 10.5,
    height: 1.2,
    fontWeight: FontWeight.w700,
    letterSpacing: 0.9,
  );

  /// 组装完整 [TextTheme]。
  static TextTheme textTheme(Color onSurface, Color muted) => TextTheme(
        displayLarge: display.copyWith(color: onSurface),
        displaySmall: display.copyWith(color: onSurface),
        headlineLarge: title.copyWith(color: onSurface),
        headlineMedium: title.copyWith(color: onSurface),
        headlineSmall: title.copyWith(color: onSurface),
        titleLarge: body.copyWith(color: onSurface),
        titleMedium: body.copyWith(color: onSurface),
        titleSmall: body.copyWith(color: onSurface),
        bodyLarge: prose.copyWith(color: onSurface),
        bodyMedium: prose.copyWith(color: onSurface),
        bodySmall: caption.copyWith(color: onSurface),
        labelLarge: action.copyWith(color: onSurface),
        labelMedium: micro.copyWith(color: muted),
        labelSmall: micro.copyWith(color: muted),
      );

  /// 导航栏/分组标题上色后的 overline。
  static TextStyle overlineIn(Color color) => overline.copyWith(color: color);

  /// 状态角标样式：等宽 + 全大写语义色。
  static TextStyle badge(Color color) => monoOverline.copyWith(color: color);
}

/// 与令牌同源的「组件外观」快捷构造。
///
/// 这些不是 MD3 的 ThemeData 扩展点（那会锁死可定制性），而是给本项目
/// 高频、造型高度一致的几类控件一套统一样式。
extension QxThemedStyles on ThemeData {
  /// 分组容器：卡片底 + 发丝描边 + 圆角。
  ///
  /// 工具类界面的「卡片」不靠阴影分层（阴影在暗色背景上几乎不可见），
  /// 而是靠**描边 + 极轻的表面提亮**——这是深色工具 UI 的正确分层方式。
  BoxDecoration get groupBox => BoxDecoration(
        color: colorScheme.surface,
        borderRadius: QxRadius.cardAll,
        border: Border.all(color: QxColor.hairline(brightness)),
      );

  /// 凹槽容器：输入框、代码块。
  BoxDecoration get sunkenBox => BoxDecoration(
        color: QxColor.sunken(brightness),
        borderRadius: QxRadius.controlAll,
        border: Border.all(color: QxColor.hairline(brightness)),
      );

  /// 浮起容器：弹层、底部抽屉。
  BoxDecoration get raisedBox => BoxDecoration(
        color: QxColor.raised(brightness),
        borderRadius: QxRadius.cardAll,
        border: Border.all(color: QxColor.hairline(brightness)),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: brightness == Brightness.dark ? 0.5 : 0.12),
            blurRadius: 28,
            offset: const Offset(0, 12),
          ),
        ],
      );
}

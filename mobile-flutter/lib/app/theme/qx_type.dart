import 'package:flutter/material.dart';

/// 排版刻度。
///
/// 决策：**中文正文用系统字体，技术数据用等宽字体**。
///
/// 理由：中文没有一款「非系统」字体能在所有 Android / iOS 机型上
/// 同时拿到正确的字重与 hinting——为中文换字体的代价远大于收益，而且
/// 绝大多数中文设备上的苹方 / 思源黑体本身观感就很好。
///
/// 但工具类 App 的性格来自**数据层**：工具名、命令、退出码、session id、
/// 耗时、行号。这些必须等宽，否则列对不齐、扫视成本极高。所以把等宽
/// 作为签名层铺到所有数据位置，UI 文字层保持系统字体。
abstract final class QxType {
  /// 随包分发的等宽字体（见 pubspec.yaml 的 `QxMono`）。
  static const String mono = 'QxMono';

  /// 屏幕大标题。字阶比参考实现克制：32 而不是 34，中文在同样字号下
  /// 视觉体量更大，再大就挤了。
  static const TextStyle display = TextStyle(
    fontSize: 30,
    height: 1.2,
    fontWeight: FontWeight.w600,
    letterSpacing: -0.4,
  );

  /// 页面标题。
  static const TextStyle title = TextStyle(
    fontSize: 19,
    height: 1.3,
    fontWeight: FontWeight.w600,
    letterSpacing: -0.2,
  );

  /// 分组标题。小标签、等宽、可扫。
  static const TextStyle overline = TextStyle(
    fontFamily: mono,
    fontSize: 10.5,
    height: 1.2,
    fontWeight: FontWeight.w700,
    letterSpacing: 0.9,
  );

  /// 列表主行。
  static const TextStyle body = TextStyle(
    fontSize: 15.5,
    height: 1.35,
    fontWeight: FontWeight.w500,
  );

  /// 消息正文。行高 1.55，中文长段落可读性必需。
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

  /// 工具输出 / 命令 / 代码。字号比 UI 略小以容纳长行。
  static const TextStyle code = TextStyle(
    fontFamily: mono,
    fontSize: 12.5,
    height: 1.5,
    fontWeight: FontWeight.w400,
  );

  /// 标识符：session id、job id、token 片段。带一点字距便于逐字核对。
  static const TextStyle ident = TextStyle(
    fontFamily: mono,
    fontSize: 11.5,
    height: 1.35,
    fontWeight: FontWeight.w500,
    letterSpacing: 0.1,
  );

  /// 指标数字。**开 `tnum`（等宽数字）**：
  /// 不开的话 `365k` 与 `152k` 宽度不同，刷新时整行会左右抖。
  static const TextStyle metric = TextStyle(
    fontFamily: mono,
    fontSize: 12,
    height: 1.3,
    fontWeight: FontWeight.w500,
    fontFeatures: [FontFeature.tabularFigures()],
  );

  /// 主按钮 / 关键动作。
  static const TextStyle action = TextStyle(
    fontSize: 15,
    height: 1.2,
    fontWeight: FontWeight.w600,
    letterSpacing: 0.1,
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

  /// 状态角标：等宽 + 加粗 + 语义色。
  ///
  /// 状态词是**中文**，所以不能用 `toUpperCase`（中文没有大小写），
  /// 这里靠字重与字距建立「像标签」的观感。
  static TextStyle badge(Color color) => overline.copyWith(color: color);
}

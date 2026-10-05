import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'qx_semantic.dart';
import 'qx_tokens.dart';
import 'qx_type.dart';

/// 千寻主题。
///
/// 暗色优先。理由是使用场景：这是一块常在暗处被掏出来的屏幕（通勤、会议、
/// 沙发），深色在 OLED 上更省电也更不刺眼。亮色同样完整支持，不是摆设。
abstract final class QxTheme {
  static ThemeData dark() => _build(Brightness.dark, QxSemantic.dark);
  static ThemeData light() => _build(Brightness.light, QxSemantic.light);

  static ThemeData _build(Brightness brightness, QxSemantic q) {
    final isDark = brightness == Brightness.dark;
    final scheme = ColorScheme.fromSeed(
      seedColor: q.accent,
      brightness: brightness,
    ).copyWith(
      primary: q.accent,
      onPrimary: q.onAccent,
      surface: q.surface,
      onSurface: q.textPrimary,
      // 必须接管：`fromSeed` 会给出一个带青绿倾向的 `onSurfaceVariant`
      // （实测 #BEC9C6 / #3F4947），而它被 Material 组件与 30+ 处代码
      // 直接读取。不接管等于次要文字层绕过设计系统。
      onSurfaceVariant: q.onSurfaceVariant,
      surfaceContainerLowest: q.surfaceSunken,
      surfaceContainerLow: q.canvas,
      surfaceContainer: q.surface,
      surfaceContainerHigh: q.surfaceRaised,
      surfaceContainerHighest: q.surfaceRaised,
      outlineVariant: q.hairline,
      error: q.failed,
    );

    final base = ThemeData(
      useMaterial3: true,
      brightness: brightness,
      colorScheme: scheme,
      textTheme: QxType.textTheme(q.textPrimary, q.textSecondary),
      // 层级靠发丝描边，**不是**阴影。关掉 M3 的色调抬升。
      scaffoldBackgroundColor: q.canvas,
      splashFactory: InkSparkle.splashFactory,
      visualDensity: VisualDensity.standard,
      extensions: [q],
    );

    return base.copyWith(
      appBarTheme: AppBarThemeData(
        backgroundColor: q.canvas,
        surfaceTintColor: Colors.transparent,
        foregroundColor: q.textPrimary,
        elevation: 0,
        scrolledUnderElevation: 0,
        centerTitle: false,
        titleTextStyle: QxType.title.copyWith(color: q.textPrimary),
        systemOverlayStyle:
            isDark ? SystemUiOverlayStyle.light : SystemUiOverlayStyle.dark,
      ),
      dividerTheme: DividerThemeData(color: q.hairline, thickness: 1, space: 1),
      dividerColor: q.hairline,
      cardTheme: CardThemeData(
        color: q.surface,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        margin: EdgeInsets.zero,
        shape: const RoundedRectangleBorder(borderRadius: QxRadius.cardAll),
      ),
      bottomSheetTheme: BottomSheetThemeData(
        backgroundColor: q.surfaceRaised,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        modalElevation: 0,
        showDragHandle: true,
        dragHandleColor: q.textTertiary.withValues(alpha: 0.5),
        dragHandleSize: const Size(36, 4),
        shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: QxRadius.container),
        ),
      ),
      dialogTheme: DialogThemeData(
        backgroundColor: q.surfaceRaised,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        shape: const RoundedRectangleBorder(borderRadius: QxRadius.cardAll),
      ),
      inputDecorationTheme: InputDecorationThemeData(
        filled: true,
        fillColor: q.surfaceSunken,
        hintStyle: QxType.body.copyWith(color: q.textTertiary),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: QxSpace.m,
          vertical: QxSpace.m,
        ),
        border: _inputBorder(q.hairline),
        enabledBorder: _inputBorder(q.hairline),
        focusedBorder: _inputBorder(q.accent, width: 1.6),
        errorBorder: _inputBorder(q.failed),
        focusedErrorBorder: _inputBorder(q.failed, width: 1.6),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          textStyle: QxType.action,
          minimumSize: const Size(0, 48),
          padding: const EdgeInsets.symmetric(horizontal: QxSpace.xl),
          shape: const RoundedRectangleBorder(borderRadius: QxRadius.controlAll),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          textStyle: QxType.action,
          minimumSize: const Size(0, 48),
          foregroundColor: q.textPrimary,
          side: BorderSide(color: q.hairline),
          shape: const RoundedRectangleBorder(borderRadius: QxRadius.controlAll),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(
          textStyle: QxType.action,
          foregroundColor: q.accent,
          minimumSize: const Size(0, 44),
        ),
      ),
      segmentedButtonTheme: SegmentedButtonThemeData(
        style: SegmentedButton.styleFrom(
          textStyle: QxType.caption.copyWith(fontWeight: FontWeight.w600),
          side: BorderSide(color: q.hairline),
          shape: const RoundedRectangleBorder(borderRadius: QxRadius.controlAll),
        ),
      ),
      snackBarTheme: SnackBarThemeData(
        backgroundColor: q.surfaceRaised,
        contentTextStyle: QxType.caption.copyWith(color: q.textPrimary),
        actionTextColor: q.accent,
        behavior: SnackBarBehavior.floating,
        elevation: 0,
        shape: const RoundedRectangleBorder(borderRadius: QxRadius.controlAll),
        insetPadding: const EdgeInsets.all(QxSpace.l),
      ),
      listTileTheme: ListTileThemeData(
        contentPadding: const EdgeInsets.symmetric(horizontal: QxSpace.l),
        minVerticalPadding: QxSpace.m,
        iconColor: q.textSecondary,
        titleTextStyle: QxType.body.copyWith(color: q.textPrimary),
        subtitleTextStyle: QxType.caption.copyWith(color: q.textSecondary),
        shape: const RoundedRectangleBorder(borderRadius: QxRadius.controlAll),
      ),
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: q.surface,
        surfaceTintColor: Colors.transparent,
        indicatorColor: q.accent.withValues(alpha: 0.16),
        elevation: 0,
        height: 64,
        labelBehavior: NavigationDestinationLabelBehavior.alwaysShow,
        labelTextStyle: WidgetStatePropertyAll(
          QxType.micro.copyWith(fontWeight: FontWeight.w600),
        ),
      ),
      sliderTheme: SliderThemeData(
        activeTrackColor: q.accent,
        inactiveTrackColor: q.hairline,
        thumbColor: q.accent,
        trackHeight: 3,
        overlayShape: const RoundSliderOverlayShape(overlayRadius: 16),
      ),
      switchTheme: SwitchThemeData(
        thumbColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.selected) ? scheme.onPrimary : q.textSecondary,
        ),
        trackColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.selected) ? q.accent : q.hairline,
        ),
        trackOutlineColor: const WidgetStatePropertyAll(Colors.transparent),
      ),
      progressIndicatorTheme: ProgressIndicatorThemeData(
        color: q.accent,
        linearTrackColor: q.hairline,
        circularTrackColor: q.hairline,
      ),
      tooltipTheme: TooltipThemeData(
        decoration: BoxDecoration(
          color: q.surfaceRaised,
          borderRadius: QxRadius.controlAll,
          border: Border.all(color: q.hairline),
        ),
        textStyle: QxType.micro.copyWith(color: q.textPrimary),
      ),
      textSelectionTheme: TextSelectionThemeData(
        cursorColor: q.accent,
        selectionColor: q.accent.withValues(alpha: 0.28),
        selectionHandleColor: q.accent,
      ),
      pageTransitionsTheme: const PageTransitionsTheme(
        builders: {
          TargetPlatform.android: PredictiveBackPageTransitionsBuilder(),
          TargetPlatform.iOS: CupertinoPageTransitionsBuilder(),
        },
      ),
    );
  }

  static OutlineInputBorder _inputBorder(Color c, {double width = 1}) =>
      OutlineInputBorder(
        borderRadius: QxRadius.controlAll,
        borderSide: BorderSide(color: c, width: width),
      );
}

/// 组件外观的快捷构造。
///
/// 这些**不是** MD3 的 ThemeData 扩展点（那会锁死可定制性），
/// 而是给本项目高频、造型高度一致的几类表面一套统一样式。
extension QxThemedStyles on ThemeData {
  QxSemantic get qx => extension<QxSemantic>() ?? QxSemantic.dark;

  /// 分组容器：surface + 发丝描边。
  ///
  /// ⚠️ 必须配 `Material` 用，不要用 `Container(decoration:)`。
  /// ListTile 会把墨水飞溅画在**最近的 Material 祖先**上，中间夹一个
  /// 带底色的 Container 会把涟漪整个盖住，表现是「点下去毫无反馈」。
  /// （这个 bug 被 widget 测试抓到过，见 test/ui_test.dart。）
  BoxDecoration get groupBox => BoxDecoration(
        color: qx.surface,
        borderRadius: QxRadius.cardAll,
        border: Border.all(color: qx.hairline),
      );

  /// 凹槽：输入框、代码块。
  BoxDecoration get sunkenBox => BoxDecoration(
        color: qx.surfaceSunken,
        borderRadius: QxRadius.controlAll,
        border: Border.all(color: qx.hairline),
      );

  /// 浮层：抽屉、底部弹层。
  ///
  /// **靠描边，不靠阴影。** OLED 上阴影几乎不可见（实测「已完成」的
  /// 1dp 轨与 surface 只差 1.41:1，本来就是靠描边才能看见），
  /// 加阴影只会引入一个看不见的层级通道，反而让人以为层级在阴影上。
  /// 浮层的「浮起感」靠的是 `surfaceRaised` 与 canvas 的**差**加描边。
  BoxDecoration get raisedBox => BoxDecoration(
        color: qx.surfaceRaised,
        borderRadius: QxRadius.cardAll,
        border: Border.all(color: qx.hairline),
      );
}

import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'qx_tokens.dart';
import 'qx_type.dart';

/// 千寻主题。
///
/// 两套：`dark`（默认）与 `light`，都从同一个青碧种子推导，保证品牌一致。
/// 关闭 MD3 的 surfaceTint 泛色——工具界面需要**精确可控的表面层级**，
/// 而不是被种子色染上一层蓝绿。
abstract final class QxTheme {
  static ThemeData dark() => _build(Brightness.dark);
  static ThemeData light() => _build(Brightness.light);

  static ThemeData _build(Brightness brightness) {
    final isDark = brightness == Brightness.dark;
    final scheme = ColorScheme.fromSeed(
      seedColor: QxColor.seed,
      brightness: brightness,
    ).copyWith(
      surface: QxColor.surface(brightness),
      onSurface: isDark ? const Color(0xFFE8EDF2) : const Color(0xFF10161C),
      surfaceContainerLowest: QxColor.sunken(brightness),
      surfaceContainerLow: QxColor.canvas(brightness),
      surfaceContainer: QxColor.surface(brightness),
      surfaceContainerHigh: QxColor.raised(brightness),
      surfaceContainerHighest: QxColor.raised(brightness),
      outlineVariant: QxColor.hairline(brightness),
    );
    final muted = isDark ? const Color(0xFF8A98A6) : const Color(0xFF5A6672);

    final text = QxType.textTheme(scheme.onSurface, muted);
    final base = ThemeData(
      useMaterial3: true,
      brightness: brightness,
      colorScheme: scheme,
      textTheme: text,
      scaffoldBackgroundColor: QxColor.canvas(brightness),
      splashFactory: InkSparkle.splashFactory,
      visualDensity: VisualDensity.standard,
    );

    return base.copyWith(
      appBarTheme: AppBarThemeData(
        backgroundColor: QxColor.canvas(brightness),
        surfaceTintColor: Colors.transparent,
        foregroundColor: scheme.onSurface,
        elevation: 0,
        scrolledUnderElevation: 0,
        centerTitle: false,
        titleTextStyle: QxType.title.copyWith(color: scheme.onSurface),
        systemOverlayStyle: isDark ? SystemUiOverlayStyle.light : SystemUiOverlayStyle.dark,
      ),
      dividerTheme: DividerThemeData(
        color: QxColor.hairline(brightness),
        thickness: 1,
        space: 1,
      ),
      cardTheme: CardThemeData(
        color: scheme.surface,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        margin: EdgeInsets.zero,
        shape: const RoundedRectangleBorder(borderRadius: QxRadius.cardAll),
      ),
      bottomSheetTheme: BottomSheetThemeData(
        backgroundColor: QxColor.raised(brightness),
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        modalElevation: 0,
        showDragHandle: true,
        dragHandleColor: muted.withValues(alpha: 0.4),
        dragHandleSize: const Size(36, 4),
        shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: QxRadius.sheet),
        ),
      ),
      dialogTheme: DialogThemeData(
        backgroundColor: QxColor.raised(brightness),
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        shape: const RoundedRectangleBorder(borderRadius: QxRadius.cardAll),
      ),
      inputDecorationTheme: InputDecorationThemeData(
        filled: true,
        fillColor: QxColor.sunken(brightness),
        hintStyle: QxType.body.copyWith(color: muted),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: QxSpace.m,
          vertical: QxSpace.m,
        ),
        border: OutlineInputBorder(
          borderRadius: QxRadius.controlAll,
          borderSide: BorderSide(color: QxColor.hairline(brightness)),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: QxRadius.controlAll,
          borderSide: BorderSide(color: QxColor.hairline(brightness)),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: QxRadius.controlAll,
          borderSide: BorderSide(color: scheme.primary, width: 1.6),
        ),
        errorBorder: OutlineInputBorder(
          borderRadius: QxRadius.controlAll,
          borderSide: BorderSide(color: scheme.error),
        ),
        focusedErrorBorder: OutlineInputBorder(
          borderRadius: QxRadius.controlAll,
          borderSide: BorderSide(color: scheme.error, width: 1.6),
        ),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          textStyle: QxType.action,
          minimumSize: const Size(0, 48),
          padding: const EdgeInsets.symmetric(horizontal: QxSpace.xl),
          shape: const RoundedRectangleBorder(
            borderRadius: QxRadius.controlAll,
          ),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          textStyle: QxType.action,
          minimumSize: const Size(0, 48),
          foregroundColor: scheme.onSurface,
          side: BorderSide(color: QxColor.hairline(brightness)),
          shape: const RoundedRectangleBorder(
            borderRadius: QxRadius.controlAll,
          ),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(
          textStyle: QxType.action,
          foregroundColor: scheme.primary,
          minimumSize: const Size(0, 44),
        ),
      ),
      segmentedButtonTheme: SegmentedButtonThemeData(
        style: SegmentedButton.styleFrom(
          textStyle: QxType.caption.copyWith(fontWeight: FontWeight.w600),
          side: BorderSide(color: QxColor.hairline(brightness)),
          shape: const RoundedRectangleBorder(
            borderRadius: QxRadius.controlAll,
          ),
        ),
      ),
      snackBarTheme: SnackBarThemeData(
        backgroundColor: QxColor.raised(brightness),
        contentTextStyle: QxType.caption.copyWith(color: scheme.onSurface),
        actionTextColor: scheme.primary,
        behavior: SnackBarBehavior.floating,
        elevation: 0,
        shape: const RoundedRectangleBorder(
          borderRadius: QxRadius.controlAll,
        ),
        insetPadding: const EdgeInsets.all(QxSpace.l),
      ),
      listTileTheme: ListTileThemeData(
        contentPadding: const EdgeInsets.symmetric(horizontal: QxSpace.l),
        minVerticalPadding: QxSpace.m,
        iconColor: muted,
        titleTextStyle: QxType.body.copyWith(color: scheme.onSurface),
        subtitleTextStyle: QxType.caption.copyWith(color: muted),
        shape: const RoundedRectangleBorder(
          borderRadius: QxRadius.controlAll,
        ),
      ),
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: QxColor.surface(brightness),
        surfaceTintColor: Colors.transparent,
        indicatorColor: scheme.primary.withValues(alpha: 0.16),
        elevation: 0,
        height: 64,
        labelBehavior: NavigationDestinationLabelBehavior.alwaysShow,
        labelTextStyle: WidgetStatePropertyAll(
          QxType.micro.copyWith(fontWeight: FontWeight.w600),
        ),
      ),
      sliderTheme: SliderThemeData(
        activeTrackColor: scheme.primary,
        inactiveTrackColor: QxColor.hairline(brightness),
        thumbColor: scheme.primary,
        trackHeight: 3,
        overlayShape: const RoundSliderOverlayShape(overlayRadius: 16),
      ),
      switchTheme: SwitchThemeData(
        thumbColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.selected) ? scheme.onPrimary : muted,
        ),
        trackColor: WidgetStateProperty.resolveWith(
          (s) => s.contains(WidgetState.selected) ? scheme.primary : QxColor.hairline(brightness),
        ),
        trackOutlineColor: const WidgetStatePropertyAll(Colors.transparent),
      ),
      progressIndicatorTheme: ProgressIndicatorThemeData(
        color: scheme.primary,
        linearTrackColor: QxColor.hairline(brightness),
        circularTrackColor: QxColor.hairline(brightness),
      ),
      tooltipTheme: TooltipThemeData(
        decoration: BoxDecoration(
          color: QxColor.raised(brightness),
          borderRadius: QxRadius.controlAll,
          border: Border.all(color: QxColor.hairline(brightness)),
        ),
        textStyle: QxType.micro.copyWith(color: scheme.onSurface),
      ),
      textSelectionTheme: TextSelectionThemeData(
        cursorColor: scheme.primary,
        selectionColor: scheme.primary.withValues(alpha: 0.28),
        selectionHandleColor: scheme.primary,
      ),
      pageTransitionsTheme: const PageTransitionsTheme(
        builders: {
          TargetPlatform.android: PredictiveBackPageTransitionsBuilder(),
          TargetPlatform.iOS: CupertinoPageTransitionsBuilder(),
        },
      ),
    );
  }
}

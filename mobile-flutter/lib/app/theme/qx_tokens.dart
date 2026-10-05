import 'package:flutter/material.dart';

/// 千寻的设计令牌。
///
/// 定位：这是一台**放在口袋里的开发机控制台**，不是消费级 App。用户是
/// 盯着自己电脑屏幕干活的人，手机屏幕要在强光下、在地铁上、在会议中一眼可读。
///
/// 因此基调是 **暗色优先 + 高对比技术感**：界面安静、信息锐利、状态色有意义。
/// 拒绝「紫渐变 + 大圆角 + 卡通插画」这套通用 AI 审美——那类界面不承载
/// 工具状态，只承载装饰。
///
/// 色彩策略：
/// - 品牌主色取 **青碧（teal）**，与桌面端千寻的冷调一致，且在暗背景上
///   长时间注视不刺眼。
/// - 会话状态用 **四色语义环**：运行=青、等待=琥珀、完成=翠绿、失败=赤。
///   这四个色是整个 App 的信息骨架，只在这里出现，其余一律用中性阶。
/// - 中性阶微微偏冷（hue 210），让青色的强调更干净。
abstract final class QxColor {
  // ── 品牌种子 ──────────────────────────────────────────────────────
  /// Material You 动态取色的回退种子。
  static const Color seed = Color(0xFF12B5A8);

  // ── 状态语义色（暗色）────────────────────────────────────────────
  static const Color runningDark = Color(0xFF3DDC97);
  static const Color runningLight = Color(0xFF0E9F6E);
  static const Color pendingDark = Color(0xFFFFC24B);
  static const Color pendingLight = Color(0xFFB87700);
  static const Color doneDark = Color(0xFF7CC4FF);
  static const Color doneLight = Color(0xFF0B6BCB);
  static const Color failDark = Color(0xFFFF7B72);
  static const Color failLight = Color(0xFFCF222E);

  // ── 状态语义色（亮色）────────────────────────────────────────────
  static const Color offline = Color(0xFF6E7781);

  // ── 中性阶（暗色：背景 → 前景）──────────────────────────────────
  /// App 最底层画布，比纯黑柔和，长时间看不累。
  static const Color canvasDark = Color(0xFF0B0F14);

  /// 卡片 / 分组底。
  static const Color surfaceDark = Color(0xFF131A22);

  /// 卡片之上的浮起层（弹层、抽屉）。
  static const Color raisedDark = Color(0xFF1B242E);

  /// 输入框 / 代码块底。
  static const Color sunkenDark = Color(0xFF080B0F);

  // ── 中性阶（亮色）────────────────────────────────────────────────
  static const Color canvasLight = Color(0xFFF6F7F9);
  static const Color surfaceLight = Color(0xFFFFFFFF);
  static const Color raisedLight = Color(0xFFFFFFFF);
  static const Color sunkenLight = Color(0xFFEDF0F3);

  // ── 描边 ─────────────────────────────────────────────────────────
  static const Color hairlineDark = Color(0x1FFFFFFF);
  static const Color hairlineLight = Color(0x14000000);

  // ── 状态色解析 ───────────────────────────────────────────────────
  /// 按语义取状态色，自动适配明暗。
  static Color running(Brightness b) => b == Brightness.dark ? runningDark : runningLight;
  static Color pending(Brightness b) => b == Brightness.dark ? pendingDark : pendingLight;
  static Color done(Brightness b) => b == Brightness.dark ? doneDark : doneLight;
  static Color fail(Brightness b) => b == Brightness.dark ? failDark : failLight;

  /// 描边色：随明暗切换。
  static Color hairline(Brightness b) => b == Brightness.dark ? hairlineDark : hairlineLight;

  /// 画布色：随明暗切换。
  static Color canvas(Brightness b) => b == Brightness.dark ? canvasDark : canvasLight;
  static Color surface(Brightness b) => b == Brightness.dark ? surfaceDark : surfaceLight;
  static Color raised(Brightness b) => b == Brightness.dark ? raisedDark : raisedLight;
  static Color sunken(Brightness b) => b == Brightness.dark ? sunkenDark : sunkenLight;
}

/// 8dp 间距刻度。MD3 规范要求间距成体系，不允许随手写魔法数字。
abstract final class QxSpace {
  static const double xxs = 2;
  static const double xs = 4;
  static const double s = 8;
  static const double m = 12;
  static const double l = 16;
  static const double xl = 20;
  static const double xxl = 24;
  static const double xxxl = 32;
  static const double huge = 48;
}

/// 圆角刻度。工具类界面用小圆角（克制），只有气泡类元素才放大圆角。
abstract final class QxRadius {
  static const Radius control = Radius.circular(8);
  static const Radius card = Radius.circular(14);
  static const Radius sheet = Radius.circular(22);
  static const Radius bubble = Radius.circular(18);

  static const BorderRadius controlAll = BorderRadius.all(control);
  static const BorderRadius cardAll = BorderRadius.all(card);
  static const BorderRadius sheetAll = BorderRadius.all(sheet);
  static const BorderRadius bubbleAll = BorderRadius.all(bubble);
}

/// 动效刻度。
///
/// 三档就够：150ms 反馈、300ms 状态切换、500ms 大件入场。
/// 弹簧只在「用户刚松手」的场合用——弹簧表达因果，不表达装饰。
abstract final class QxMotion {
  static const Duration instant = Duration(milliseconds: 150);
  static const Duration quick = Duration(milliseconds: 220);
  static const Duration normal = Duration(milliseconds: 300);
  static const Duration slow = Duration(milliseconds: 500);

  /// 状态切换统一用这条曲线：起步快、收尾稳，避免机械的线性。
  static const Curve enter = Curves.easeOutCubic;
  static const Curve exit = Curves.easeInCubic;
  static const Curve emphasized = Cubic(0.2, 0.0, 0.0, 1.0);
}

import 'package:flutter/material.dart';

/// 间距刻度。
///
/// 8dp 体系，但**不是** Material 的 4/8/16/24 四档。工具界面需要
/// 12 与 20 这种「不上不下」的档，否则要么挤要么散。
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

/// 圆角刻度。
///
/// 工具类界面用小圆角：圆角越大界面越「软」，而这个 App 要的是工具感。
/// 唯一的大圆角是气泡与输入栏（22），因为它们是「容器」而不是「条目」。
abstract final class QxRadius {
  static const Radius chip = Radius.circular(6);
  static const Radius control = Radius.circular(8);
  static const Radius block = Radius.circular(12);
  static const Radius card = Radius.circular(14);
  static const Radius bubble = Radius.circular(18);
  static const Radius container = Radius.circular(22);

  static const BorderRadius chipAll = BorderRadius.all(chip);
  static const BorderRadius controlAll = BorderRadius.all(control);
  static const BorderRadius blockAll = BorderRadius.all(block);
  static const BorderRadius cardAll = BorderRadius.all(card);
  static const BorderRadius bubbleAll = BorderRadius.all(bubble);
  static const BorderRadius containerAll = BorderRadius.all(container);
}

/// 动效刻度。
///
/// 三档够用：150 反馈、300 状态切换、500 大件入场。
/// 弹簧只用于「用户刚松手」——弹簧表达因果，不表达装饰。
abstract final class QxMotion {
  static const Duration instant = Duration(milliseconds: 150);
  static const Duration quick = Duration(milliseconds: 220);
  static const Duration normal = Duration(milliseconds: 300);
  static const Duration slow = Duration(milliseconds: 500);

  static const Curve enter = Curves.easeOutCubic;
  static const Curve exit = Curves.easeInCubic;
  static const Curve emphasized = Cubic(0.2, 0.0, 0.0, 1.0);
}

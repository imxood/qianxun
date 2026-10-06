import 'package:flutter/foundation.dart';

/// WebView 宿主的可观测状态。
///
/// 抽出来的理由不是为了「架构好看」，而是**为了让返回键那三级递进
/// 能被测试**。`DshPage._handleBack` 里有 `await controller.canGoBack()`
/// 与 `Navigator.canPop()`，在 widget 测试里都要 mock WebView；
/// 把判定抽成纯函数后，返回顺序就能直接单测。
///
/// ⚠️ 返回顺序**不可调换**（05 §9.5 / 10-shell-bridge.md §4）：
///   1. WebView 有历史 → 后退
///   2. 在 DSH 页 → 回设备列表
///   3. 在设备列表根页 → 退出
/// 换序的后果：用户在 DSH 里翻过 10 次历史后按返回，本该回设备列表，
/// 却被送回 10 步之前 —— 看着像「返回键坏了」。
enum DshBackAction {
  /// 退一步 WebView 历史。
  webHistory,

  /// 回设备列表。
  deviceList,

  /// 交给系统（退出 App）。
  exit,
}

/// 返回键的唯一判定逻辑。纯函数，无副作用，可单测。
///
/// 三个输入对应三件事：
/// - [webCanGoBack]：`controller.canGoBack()` 的结果
/// - [routeCanPop]：Navigator 栈里还有没有可 pop 的页
/// - [onDeviceListRoot]：当前是否已在设备列表根页（此时「退」= 退出）
DshBackAction decideBackAction({
  required bool webCanGoBack,
  required bool routeCanPop,
  required bool onDeviceListRoot,
}) {
  // 1. 先看 WebView 历史。用户明确要求「DSH 是独立页面」，
  //    但独立页面**内部**的导航仍应能后退。
  if (webCanGoBack) return DshBackAction.webHistory;
  // 2. DSH 页 → 回设备列表。
  if (routeCanPop) return DshBackAction.deviceList;
  // 3. 到头了。
  return onDeviceListRoot ? DshBackAction.exit : DshBackAction.deviceList;
}

/// 宿主的当前阶段。给 UI 区分「三种白屏」用。
enum DshHostPhase {
  /// 正在注入 cookie 与加载。
  loading,

  /// 主文档已加载，正在确认 custom.js 是否生效。
  probingCustomLayer,

  /// 页面可用，定制层已确认在（`window.qxMobile` 存在）。
  ready,

  /// 页面加载了，但**定制层没生效** → custom.js 没被执行。
  ///
  /// 这是与「加载失败」完全不同的故障：页面看起来是加载了的，
  /// 但所有 UI 增强都没发生。分开报，否则排查方向会错。
  customLayerMissing,

  /// 主文档加载失败。
  failed,
}

/// 宿主状态快照。
@immutable
class WebViewHostState {
  const WebViewHostState({
    this.phase = DshHostPhase.loading,
    this.detail = '',
  });

  final DshHostPhase phase;

  /// 补充说明。**不含凭据** —— 这一条会被日志与 UI 读。
  final String detail;

  bool get isBusy =>
      phase == DshHostPhase.loading || phase == DshHostPhase.probingCustomLayer;

  /// 顶部状态条该显示什么。null = 不显示（一切正常）。
  String? get statusText => switch (phase) {
        DshHostPhase.loading => '正在连接…',
        DshHostPhase.probingCustomLayer => '正在检测定制层…',
        DshHostPhase.ready => null,
        DshHostPhase.customLayerMissing =>
          '页面已加载，但 custom.js 未生效（window.qxMobile 不存在）',
        DshHostPhase.failed => detail.isEmpty ? '加载失败' : '加载失败：$detail',
      };

  WebViewHostState copyWith({DshHostPhase? phase, String? detail}) =>
      WebViewHostState(
        phase: phase ?? this.phase,
        detail: detail ?? this.detail,
      );

  @override
  bool operator ==(Object other) =>
      other is WebViewHostState &&
      other.phase == phase &&
      other.detail == detail;

  @override
  int get hashCode => Object.hash(phase, detail);
}

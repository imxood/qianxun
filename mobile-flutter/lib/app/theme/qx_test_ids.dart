/// 自动化测试用的稳定标识。
///
/// **为什么需要它。** Flutter 用 Skia 把每个像素画进单个 `FlutterView`，
/// 所以 Android 的 UiAutomator 眼里的原生 App 是一片空白画布——没有
/// `resource-id`、`className` 也没意义。能被原生自动化看见的，只有
/// Flutter **语义树**映射过去的那一层。
///
/// `Semantics(identifier: …)` 是那座桥：Flutter 官方文档写明它在 Android
/// 上会写进 `AccessibilityNodeInfo.setViewIdResourceName`，也就是在
/// `adb shell uiautomator dump` 的 XML 里**显示为 `resource-id`**。
/// 于是 uiautomator2 可以 `d(resourceId=…)` 直接定位。
///
/// **为什么集中在一个文件里。** 散落的字符串字面量一定会出现拼写漂移
/// （`session_tile` / `sessionTile` / `session-tile`），而这种漂移在
/// 运行时完全无声——脚本只是「找不到元素」。集中之后：
///  - 改一处，全仓同步
///  - `test/automation_ids_test.dart` 能断言「每个 id 都真的用上了」
///
/// **命名约定**：`snake_case`，与 Flutter widget 类名无关，只描述它在
/// 自动化里**是什么**，不是它叫什么。这样重命名 widget 不会连带弄坏脚本。
///
/// ⚠️ 这些 id **不面向用户**，Flutter 文档明确说它「not exposed to the
/// users of the app」——不影响无障碍朗读，也不出现在任何 UI 上。
abstract final class QxTestIds {
  // ── 会话列表 ──────────────────────────────────────────────
  static const sessionsList = 'qx_sessions_list';
  static const sessionTile = 'qx_session_tile';
  static const newSessionButton = 'qx_new_session_button';
  static const connectionButton = 'qx_connection_button';

  // ── 会话详情 ──────────────────────────────────────────────
  static const transcript = 'qx_transcript';
  static const composerInput = 'qx_composer_input';
  static const sendButton = 'qx_send_button';
  static const attachButton = 'qx_attach_button';

  // ── 待答链路 ──────────────────────────────────────────────
  static const pendingBar = 'qx_pending_bar';
  static const approveButton = 'qx_approve_button';
  static const rejectButton = 'qx_reject_button';

  // ── 连接 / 日志 ───────────────────────────────────────────
  static const connectionsScreen = 'qx_connections_screen';
  static const logsScreen = 'qx_logs_screen';

  /// 全部 id。给测试遍历用——见 `test/automation_ids_test.dart`。
  static const all = <String>[
    sessionsList,
    sessionTile,
    newSessionButton,
    connectionButton,
    transcript,
    composerInput,
    sendButton,
    attachButton,
    pendingBar,
    approveButton,
    rejectButton,
    connectionsScreen,
    logsScreen,
  ];
}

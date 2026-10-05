import 'package:flutter/material.dart';

/// 领域语义色（`ThemeExtension`）。
///
/// 为什么不塞进 [ColorScheme]：`ColorScheme` 的槽位是 Material 定义的
/// （primary / secondary / surface / error …），而这个 App 的信息骨架是
/// **五个会话状态**，它们不是 Material 角色。硬塞进 `secondary` / `tertiary`
/// 只会让「状态色」和「品牌色」语义打架。
///
/// 也为什么不留在 `qx_tokens.dart` 里用静态常量：那样拿不到 `Brightness`，
/// 亮色模式就得在每个调用点写 `brightness == dark ? a : b`，写错一处就是
/// 深色界面上出现浅色字。放 `ThemeExtension` 里，明暗由主题一次决定。
@immutable
class QxSemantic extends ThemeExtension<QxSemantic> {
  const QxSemantic({
    required this.running,
    required this.waiting,
    required this.done,
    required this.detached,
    required this.failed,
    required this.accent,
    required this.onAccent,
    required this.canvas,
    required this.surface,
    required this.surfaceRaised,
    required this.surfaceSunken,
    required this.hairline,
    required this.textPrimary,
    required this.textSecondary,
    required this.textTertiary,
    required this.onSurfaceVariant,
    required this.railIdle,
  });

  // ── 状态色 ────────────────────────────────────────────────────
  /// 运行中。绿。**这是唯一带运动的状态。**
  final Color running;

  /// 等你。琥珀。轨为虚线。
  final Color waiting;

  /// 已完成（agent 在，但这会儿没在跑）。蓝。
  final Color done;

  /// 已断开（子 agent 的父会话结束）。中性灰，**不用**红色——
  /// 红色是警报，用户不该为「正常的生命周期结束」紧张。
  final Color detached;

  /// 失败。赤。
  final Color failed;

  /// 品牌青。**只用于可操作元素**（发送键、选中态、链接），
  /// 不参与状态表达——状态色只有上面五个。
  final Color accent;

  /// accent 上的前景色。
  ///
  /// 暗色下 accent 是亮青，前景必须是**深**色（#04211F，6.6:1）；
  /// 硬写 `Colors.white` 只有 2.56:1，20px 的箭头在暗色下几乎看不见。
  final Color onAccent;

  // ── 表面层级 ──────────────────────────────────────────────────
  /// 页面最底层。
  final Color canvas;

  /// 卡片 / 行。
  final Color surface;

  /// 浮层：抽屉、底部弹层。
  final Color surfaceRaised;

  /// 凹陷：输入框、代码块。
  final Color surfaceSunken;

  /// 发丝描边。**层级靠它，不靠阴影**——OLED 上阴影几乎不可见。
  final Color hairline;

  // ── 文字三级 ──────────────────────────────────────────────────
  final Color textPrimary;
  final Color textSecondary;
  final Color textTertiary;

  /// Material 的 `onSurfaceVariant` 覆盖值。
  ///
  /// Material 组件（ListTile / 展开项 / 菜单）默认读它，而它是全项目
  /// 第二高频的文字色。不显式接管的话，`ColorScheme.fromSeed` 推出来的
  /// 带青绿值会在 30 多处悄悄生效。
  final Color onSurfaceVariant;


  /// 「已完成」态的轨宽是 1dp，接近隐没。这个颜色就是那条线。
  final Color railIdle;

  /// 暗色板。
  ///
  /// 刻意**不用纯黑纯白**：OLED 上纯黑会让像素边缘发绿，长时间看不舒服；
  /// 纯白在暗环境刺眼。`canvas` 也不是纯黑，抬到 `#0B0E13` 刚好让
  /// `surface` 提亮一档就能被看见。
  static const dark = QxSemantic(
    running: Color(0xFF3DDC97),
    waiting: Color(0xFFFFC24B),
    done: Color(0xFF7CC4FF),
    detached: Color(0xFF737C87),
    failed: Color(0xFFFF7B72),
    accent: Color(0xFF12B5A8),
    onAccent: Color(0xFF04211F),
    canvas: Color(0xFF0B0E13),
    surface: Color(0xFF12161D),
    surfaceRaised: Color(0xFF1A2029),
    surfaceSunken: Color(0xFF070A0E),
    hairline: Color(0x1FFFFFFF),
    textPrimary: Color(0xFFE8EDF2),
    textSecondary: Color(0xFF9AA7B4),
    textTertiary: Color(0xFF72818F),
    // `onSurfaceVariant` 必须**显式接管**：`ColorScheme.fromSeed` 会从种子
    // 推导出带青绿倾向的值（实测 #BEC9C6），而它是全项目第二高频的文字
    // 色（30+ 处调用），不接管就等于整层次要文字绕过了设计系统。
    onSurfaceVariant: Color(0xFF9AA7B4),
    railIdle: Color(0xFF2A323C),
  );

  /// 亮色板。
  ///
  /// 注意 `surfaceRaised` 是 `#FEFFFF` 而不是 `#FFFFFF`，
  /// `textPrimary` 是 `#141A21` 而不是纯黑——亮色下这两处的纯白/纯黑
  /// 会让描边消失或显得刺眼。
  static const light = QxSemantic(
    running: Color(0xFF0B7E57),
    waiting: Color(0xFF996300),
    done: Color(0xFF0B6BCB),
    detached: Color(0xFF656F79),
    failed: Color(0xFFCF222E),
    accent: Color(0xFF0C7B72),
    onAccent: Color(0xFFFFFFFF),
    canvas: Color(0xFFEFF2F5),
    surface: Color(0xFFF7F9FB),
    surfaceRaised: Color(0xFFFEFFFF),
    surfaceSunken: Color(0xFFE7ECF1),
    hairline: Color(0x14000000),
    textPrimary: Color(0xFF141A21),
    textSecondary: Color(0xFF5A6672),
    // 亮色的 tertiary 要压到 4.5:1 以上，**三个底色都得过**：
    // surface（列表）、surfaceSunken（输入框的 hint）、canvas（页面底）。
    // 只按 surface 调色会漏掉后两个 —— 亮色下后两者更浅，对比度更低。
    textTertiary: Color(0xFF626B75),
    onSurfaceVariant: Color(0xFF5A6672),
    railIdle: Color(0xFFC8D0D8),
  );

  @override
  QxSemantic copyWith({
    Color? running,
    Color? waiting,
    Color? done,
    Color? detached,
    Color? failed,
    Color? accent,
    Color? onAccent,
    Color? canvas,
    Color? surface,
    Color? surfaceRaised,
    Color? surfaceSunken,
    Color? hairline,
    Color? textPrimary,
    Color? textSecondary,
    Color? textTertiary,
    Color? onSurfaceVariant,
    Color? railIdle,
  }) =>
      QxSemantic(
        running: running ?? this.running,
        waiting: waiting ?? this.waiting,
        done: done ?? this.done,
        detached: detached ?? this.detached,
        failed: failed ?? this.failed,
        accent: accent ?? this.accent,
        onAccent: onAccent ?? this.onAccent,
        canvas: canvas ?? this.canvas,
        surface: surface ?? this.surface,
        surfaceRaised: surfaceRaised ?? this.surfaceRaised,
        surfaceSunken: surfaceSunken ?? this.surfaceSunken,
        hairline: hairline ?? this.hairline,
        textPrimary: textPrimary ?? this.textPrimary,
        textSecondary: textSecondary ?? this.textSecondary,
        textTertiary: textTertiary ?? this.textTertiary,
        onSurfaceVariant: onSurfaceVariant ?? this.onSurfaceVariant,
        railIdle: railIdle ?? this.railIdle,
      );

  @override
  QxSemantic lerp(covariant QxSemantic? other, double t) {
    if (other == null) return this;
    Color c(Color a, Color b) => Color.lerp(a, b, t)!;
    return QxSemantic(
      running: c(running, other.running),
      waiting: c(waiting, other.waiting),
      done: c(done, other.done),
      detached: c(detached, other.detached),
      failed: c(failed, other.failed),
      accent: c(accent, other.accent),
      onAccent: c(onAccent, other.onAccent),
      canvas: c(canvas, other.canvas),
      surface: c(surface, other.surface),
      surfaceRaised: c(surfaceRaised, other.surfaceRaised),
      surfaceSunken: c(surfaceSunken, other.surfaceSunken),
      hairline: c(hairline, other.hairline),
      textPrimary: c(textPrimary, other.textPrimary),
      textSecondary: c(textSecondary, other.textSecondary),
      textTertiary: c(textTertiary, other.textTertiary),
      onSurfaceVariant: c(onSurfaceVariant, other.onSurfaceVariant),
      railIdle: c(railIdle, other.railIdle),
    );
  }
}

/// 取语义色的便捷入口。
extension QxSemanticX on BuildContext {
  QxSemantic get qx => Theme.of(this).extension<QxSemantic>() ?? QxSemantic.dark;
}

/// 会话状态。
///
/// 状态词表**只有五个**，恰好等于已验证字段能区分的数目。
///
/// 早期版本有七个（多出 `blocked` 紫与 `failed` 红两态），但那两个
/// **没有数据源**：服务端不下发，猜出来的颜色只会误导人。
///
/// 轨宽的分配遵循 `docs/design/05-product-design.md` §3.1 的四档规格，
/// 其中「运行中」与「等你」是仅有的两档粗轨。
enum QxSessionState {
  running,
  waiting,
  done,
  detached,
  failed;

  /// 中文词。**全部是动词性/状态性的词**，不与日志级别、工具状态共用，
  /// 避免同一列出现「运行中」却指两件不同的事。
  ///
  /// 词表以 05 §2 为准：运行中 / 等你 / 已完成 / 已断开 / 失败。
  String get label => switch (this) {
        QxSessionState.running => '运行中',
        QxSessionState.waiting => '等你',
        QxSessionState.done => '已完成',
        QxSessionState.detached => '已断开',
        QxSessionState.failed => '失败',
      };

  /// 状态主色。
  Color colorOf(QxSemantic q) => switch (this) {
        QxSessionState.running => q.running,
        QxSessionState.waiting => q.waiting,
        QxSessionState.done => q.done,
        QxSessionState.detached => q.detached,
        QxSessionState.failed => q.failed,
      };

  /// **轨专用**颜色。
  ///
  /// 与 [colorOf] 只在 `done` 上不同：已完成态的 1dp 轨要**退到几乎看不见**
  /// （`railIdle`，40% 钢蓝），所以它用一条比状态色暗得多的线。
  /// 直接用 `q.done` 的话，1dp 亮蓝在暗背景上仍然很扎眼，「完成要退后
  /// 而不是庆祝」这条设计意图就丢了。
  Color railColor(QxSemantic q) => switch (this) {
        QxSessionState.done => q.railIdle,
        _ => colorOf(q),
      };

  /// 轨的粗细。**这就是排序通道**：用户纯靠余光扫一列竖条的粗细，
  /// 就能把「要我管的」挑出来。
  ///
  /// 分档见 `docs/design/05-product-design.md` §3.1。
  /// 「运行中」与「等你」是**仅有的两档粗轨**，其余两档退到背景里去
  /// （05 §3.2 明确：条目多了也只能保证这两档最粗，不许把等待降级成 1dp）。
  double get railWidth => switch (this) {
        // 在动 = 4dp。失败同为 4dp：它同样是「我这条出事了」，不该比运行中更轻。
        QxSessionState.running => 4,
        QxSessionState.failed => 4,
        // 等你 3dp。要用户介入，但服务是停住的，不该看起来比运行中更忙。
        QxSessionState.waiting => 3,
        QxSessionState.detached => 2,
        // 已完成几乎隐没——它是最不需要用户注意的态。
        QxSessionState.done => 1,
      };

  /// 虚线还是实心。**颜色之外的第二个通道**：色觉障碍用户与灰度截图下，
  /// 「实心 / 虚线 + 粗细 + 中文词」三者合起来仍然可读。
  ///
  /// 虚线 = **服务不在动**（等你、已断开）；实心 = 在动或已定局。
  /// 05 §3.1：「实心（运行中、失败）/ 虚线段（等你、已断开）」。
  bool get railDashed =>
      this == QxSessionState.waiting || this == QxSessionState.detached;

  /// 只有运行中有运动。呼吸表达「它在跑」，别处不动。
  bool get railAnimated => this == QxSessionState.running;

  /// 状态字形。**每态不同**，是颜色之外的第三个通道。
  IconData get glyph => switch (this) {
        QxSessionState.running => Icons.play_arrow_rounded,
        QxSessionState.waiting => Icons.priority_high_rounded,
        QxSessionState.done => Icons.circle_outlined,
        QxSessionState.detached => Icons.block_rounded,
        QxSessionState.failed => Icons.error_outline_rounded,
      };
}

/// 状态优先级。数值越大越靠前。
///
/// 「等你」的行**永远置顶**（设计裁决 3.3），这是本 App 第二签名级
/// 元素——比任何视觉装置都更能兑现「5 到 40 秒内找到要你管的那条」。
int qxStatePriority(QxSessionState s) => switch (s) {
      QxSessionState.waiting => 100,
      QxSessionState.running => 50,
      QxSessionState.failed => 30,
      QxSessionState.done => 10,
      QxSessionState.detached => 0,
    };

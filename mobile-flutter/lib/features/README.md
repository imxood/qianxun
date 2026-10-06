# 方案 B 之后，Flutter 侧哪些代码还参与编译
#
# 背景
# ----
# 方案 B 确立后，**DSH 页面由 WebView 承载**，界面由
# `<dsh_home>/mobile-access/custom.{js,css}` 在页面内重建
# （见 docs/design/11-qx-mobile-layer.md）。
#
# 于是 lib/features/{sessions,workspace} 里的 Flutter 界面
# **不再参与运行**。它们如果继续参与编译：
#   · 拖慢每次 analyze / test
#   · 让人误以为「这个 App 有两套会话 UI」
#   · 它们的测试会挡住真正的门禁（117 个 UI 测试测的是不再跑的东西）
#
# 处理方式：**移出 lib/ 而不是删掉**。
# 理由是它们是这一路的设计证据（05 裁决的落点、信号轨的验证、
# 稀疏行认知的推导），删掉等于丢掉可追溯性；留在原地又会被编译。
#
# 恢复方式：把目录移回 lib/features/ 即可，无需改任何代码
# （它们只是被 analysis_options 排除了）。

# ── 已作废的界面层 ────────────────────────────────────────────
# sessions/  会话列表（真实会话列表由 DSH 页面内的 custom.js 负责）
# workspace/ 转录、输入栏、审批条（同上）
#
# 保留在 lib/data/ 与 lib/shared/ 的（**继续编译**）：
#   data/dsh/        协议层：壳用它做探活与配对
#   data/models/     模型：探针与测试仍要读
#   data/store/      连接持久化
#   core/qx_log.dart release 不写 logcat，排查只能靠它
#   shared/widgets/  通用件
#   app/theme/       主题（对比度闸门仍用）
#   features/connections/  设备列表（壳的根页）
#   features/logs/          运行日志（排查入口）
#   features/dsh/           WebView 宿主（方案 B 的核心）

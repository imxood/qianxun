# 测试清单 —— 方案 B 之后

## 继续进门禁的（活门禁）

| 文件 | 测什么 | 为什么必须继续跑 |
|---|---|---|
| `protocol_test.dart` | RPC 信封、WS mux、`requestId` 去重、assistant-stream 压缩 | 壳用它做探活与配对；这些是**实测过的线格式**，错了会静默失败 |
| `mobile_access_test.dart` | 端点/凭据模型、过期边界、**凭据不泄漏** | 安全相关。含 208 条稀疏行认知的落地 |
| `dsh_back_test.dart` | 返回键三级递进 | 顺序错 = 用户按返回没反应 |
| `connection_equality_test.dart` | `Connection` 的值相等性 | 没有它，family 键每次都变，WebSocket 反复重连（真机才看得见） |
| `attachment_test.dart` | 附件校验（`imageLimits`） | 页面侧也依赖同一套限制的认知 |
| `design_system_test.dart` | 对比度闸门 | `check_contrast.py` 的 Dart 侧对照 |
| `logs_test.dart` | 日志导出 | release 不写 logcat，这是唯一的排查通道 |
| `cold_start_test.dart` | 冷启动落点 | 方案 B 下改成落设备列表 |

## 已移出门禁的（保留文件作对照）

| 文件 | 原测什么 | 为什么停 |
|---|---|---|
| `ui_test.dart` | 信号轨、会话行、工具卡、转录列表、格式化 | 方案 B 下这些界面由页面内 `custom.js` 重建，Flutter 侧不再渲染 |
| `composer_test.dart` | 输入栏、草稿、引导/排队切换 | 同上；且依赖 `file_picker`（已随附件职责移交给页面而移除） |
| `session_list_test.dart` | 会话列表行 | 真实会话列表由 DSH 自己的组件 + `custom.css` 负责 |
| `automation_ids_test.dart` | 自动化 id 标记 | 同上 |

## 为什么是 exclude 而不是删

它们是这一路的设计证据：

- 信号轨的四重编码（粗细/虚实/运动/颜色）在这里被验证过
- 「稀疏行认知」的推导依赖那些 fixture
- 05 裁决的落点能对上这些断言

删掉等于丢掉可追溯性。留在原地又会被编译（`file_picker` 已移除）。
所以移出编译范围，文件保留 —— `analysis_options.yaml` 里两行注释就是开关。

## 若要恢复

删掉 `analysis_options.yaml` 里那 4 行 `test/*.dart`，并把
`lib/features/{sessions,workspace}` 移回 `lib/features/` 下，
同时把 `file_picker` 加回 `pubspec.yaml`。

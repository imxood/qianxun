# 23 — Flutter 端对 Web 页面 UI 的定制化分析

> 分析日期：2026-11-05｜性质：只读分析（本次未改代码）。结论均带 `文件:行号` 或可复现命令。
> 前置：见 [22-Flutter端远程访问盘点.md](./22-Flutter端远程访问盘点.md)（传输层盘点）。

## 0. 结论（一句话）

**Flutter 端自身几乎不做 Web 页面 UI 定制。** 真正的定制是「千寻移动定制层 L3」——`custom.css`（68KB / 2099 行）+ `custom.js`（58KB / 1663 行），它**由电脑端执行、由 Flutter 端分发**。壳（Flutter）严格遵守「壳不碰界面，页面不碰设备」这条分工。

## 1. 三层职责（实际代码 vs 设计）

设计出处：[docs/design/09-shell-bridge.md:28](./09-shell-bridge.md)「**原则：壳不碰界面，页面不碰设备**」。

| 层              | 谁执行                              | 做什么                                                 | 代码位置                                                                                      |
| --------------- | ----------------------------------- | ------------------------------------------------------ | --------------------------------------------------------------------------------------------- |
| 壳（Flutter）   | 手机 App                            | 连接、配对、凭据、TLS、返回键、外链分流、**顶栏**      | [lib/features/dsh/presentation/dsh_page.dart](../lib/features/dsh/presentation/dsh_page.dart) |
| 桥（`qxShell`） | 双向                                | 页面 -> 壳消息通道（**目前只打日志**）                 | dsh_page.dart:156-159, 218-221                                                                |
| 定制层 L3       | **电脑端页面内**（WebView 里的 JS） | DSH 页面窄屏重排 + 自绘覆盖层（导航/工具卡/审批/diff） | `<dsh_home>/mobile-access/custom.{css,js}`，底稿 [mobile-flutter/qx-mobile/](../qx-mobile/)   |

## 2. 壳对页面 UI 的实际动作：只有一处

全仓 grep `runJavaScript|evaluateJavascript|addJavaScriptChannel|setUserAgent|setJavaScriptMode`，命中**仅 3 处**，全在 `dsh_page.dart`：

1. `setJavaScriptMode(JavaScriptMode.unrestricted)`（:109）——开 JS 执行权限，**不是**定制；
2. `addJavaScriptChannel('qxShell', ...)`（:156-159）——建消息通道；
3. `runJavaScriptReturningResult('typeof window.qxMobile === "object" ...')`（:200-202）——**只读探测**，问页面「定制层在不在」，不回写 DOM。

**结论：壳对页面 DOM/CSS/样式零注入。** 没有 `insertAdjacentHTML`、没有 `classList.add`、没有改 `style`、没有 UA 伪装。

### 2.1 壳主动「不画」的地方（有据）

- 顶栏只留「返回 + 机器名」，**不放任何 DSH 相关控件**——注释写明「那些由页面的 custom.js 负责，壳再画一遍就成了两套 UI」（dsh_page.dart:282-283）。
- `lib/features/sessions/`、`lib/features/workspace/` 两套原生 UI（会话列表、转录、审批条、输入框）**已作废**，仅保留在 `test/legacy_flutter_ui/` 作对照，不是活代码路径（09-shell-bridge.md:136-146）。设计文档 §5 明确「`lib/features/sessions/` `lib/features/workspace/` ❌ 作废——页面由 L3 做」。
- 审批不再由 Flutter 侧 `interactionController` 管，改为页面里的 `custom.js` 直接读 `projections.userQuestions`（[app.dart:159-169](../lib/app/app.dart)）。

### 2.2 壳确实管的「页面外围」

| 行为                       | 位置                  | 说明                                                                                          |
| -------------------------- | --------------------- | --------------------------------------------------------------------------------------------- |
| 返回键三级递进             | dsh_page.dart:223-262 | WebView 历史 -> 设备列表 -> 退出                                                              |
| 外链分流                   | dsh_page.dart:138-151 | 同源留 WebView，外链交系统浏览器                                                              |
| 加载前种 cookie            | dsh_page.dart:102-106 | 否则「界面在但数据全空」                                                                      |
| URL 剥凭据                 | dsh_page.dart:166     | token 不进 URL/浏览历史                                                                       |
| 壳自身字号缩放夹取 0.9–2.0 | app.dart:180-184      | **只影响壳 UI**；WebView 页面由 `custom.css` 的 `font-size: 16px` 管（app.dart:177-179 注释） |

## 3. 真正的定制层（L3）：内容与归属

### 3.1 归属：Flutter 端**分发**，电脑端**执行**

- 底稿在仓库：[mobile-flutter/qx-mobile/custom.css](../qx-mobile/custom.css)、[custom.js](../qx-mobile/custom.js)（与电脑端部署副本逐字节一致，2026-10-31 校验）；
- 部署脚本：[scripts/deploy_qx_mobile.py](../scripts/deploy_qx_mobile.py)——「把 qx-mobile/ 下的定制层部署到千寻的 mobile-access 目录」，`FILES = ["custom.css", "custom.js"]`，写前备份；
- 执行位置：**电脑端**。千寻网关注入 `qx-mobile-bootstrap.js`（`/qx-mobile/bootstrap.js`），它轮询 `/qx-mobile/version` 的内容 hash，变化即热替换 `<link>` 与脚本（千寻侧 `src-tauri/src/remote/assets/qx-mobile-bootstrap.js`）。

**所以「Flutter 端对 Web UI 做了定制」的准确说法是：Flutter 端维护并分发了一套定制层，但定制层是在手机 WebView 里的 DSH 页面上执行的，壳本身不碰 UI。**

### 3.2 定制层规模与手法（实测统计）

```
custom.css  68415 B / 2099 行   @media x31  !important x292  font-size x17  overflow x46
custom.js   58442 B / 1663 行   querySelector x57  appendChild x12  style. x29  addEventListener x6
```

两段分工（CSS 自述）：

- **§A：DSH 原版界面的窄屏适配（L1 层，只重排不重写）**——31 个 `@media` 断点、292 个 `!important` 覆盖原版样式；
- **§B：自绘覆盖层**（`qx-layer` / `qx-sheet` / `qx-full-btn` / `qx-hint-*`）——JS 建 DOM 节点接管导航、工具卡、审批、diff。

CSS 里有一条**选择器纪律**（文件头自述）：「**绝不写 class 名选择器去改 DSH 的东西**——它的 class 是构建期哈希（实测 `hHd-Xa_` / `bhn1Oq_`），每次 DSH 构建都变」。

### 3.3 定制层与壳的唯一耦合点

壳探测 `window.qxMobile` 是否存在，据此决定「白屏往哪查」（dsh_page.dart:197-216、[webview_host_state.dart:52-58, 90](../lib/features/dsh/presentation/webview_host_state.dart)）。**探测契约**由千寻侧定义（`window.qxMobile.register`），壳只是消费者。

## 4. 当前状态：定制层已下线，壳侧有残留

2026-10-31 应用户要求下线 L3（移除电脑端 `mobile-access/custom.{css,js}`，机制保留）。随之产生两处不一致：

| 残留                           | 位置                                              | 影响                                                                    |
| ------------------------------ | ------------------------------------------------- | ----------------------------------------------------------------------- |
| 壳仍在探测 `window.qxMobile`   | dsh_page.dart:198-216                             | **必然探测失败**（层已不在）-> 每次进 DSH 页都记一条 warn               |
| 「custom.js 未生效」错误态仍在 | webview_host_state.dart:58, 90；dsh_page.dart:325 | **误导性报错**：页面其实正常（DSH 原版 UI），却提示「custom.js 未生效」 |
| 底稿仍在仓库                   | `mobile-flutter/qx-mobile/`                       | 保留待恢复（千寻 `docs/21` §恢复方法）                                  |

## 5. 结论要点

1. **壳不碰界面**是设计不变量，代码中确实守住（3 处 JS 调用全是非侵入的）。
2. **UI 定制主体是 L3 定制层**，由 Flutter 端维护/部署、在电脑端页面内执行；当前已下线。
3. **壳侧有两处残留**（探测 + 错误态），下线定制层时应同步清理，否则用户会看到「custom.js 未生效」的假告警。
4. 若未来恢复定制层或引入新定制（如上一轮实现的「本地服务反代」在手机上的入口），仍应遵守：定制在**页面内**做，壳只管连接与外围。

## 附：复现命令

```bash
# 壳对页面的全部 JS 交互
rg -n "runJavaScript|evaluateJavascript|addJavaScriptChannel|setUserAgent" mobile-flutter/lib
# 定制层规模与手法
rg -c "querySelector|appendChild" mobile-flutter/qx-mobile/custom.js
rg -c "@media|!important" mobile-flutter/qx-mobile/custom.css
# 部署去向
rg -n "mobile-access|FILES" mobile-flutter/scripts/deploy_qx_mobile.py
```

## 变更记录

- 2026-11-05 首版：应用户要求分析 Flutter 端对 Web 页面 UI 的定制化程度（只读）。

import 'dart:async';

import 'package:flutter/foundation.dart' show defaultTargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
// ⚠️ `Cookie` / `CookieManager` / `UriRequest` 都来自这个包，
// **不是** `dart:io`。dart:io 也有一个 `Cookie`，两者签名不同
// （dart:io 的有 path/httpOnly，webview 的只有 name/value/domain），
// 混用会得到「The method 'Cookie' isn't defined for the type」这种
// 指不到根因的报错。
import 'package:webview_flutter/webview_flutter.dart';
// ⚠️ Android 专有扩展：`AndroidWebViewController` 与 `FileSelectorMode`
// 只存在于 `webview_flutter_android`，不在 `webview_flutter` 主包里。
// iOS 编译时这个 import 仍可用（该包对所有平台都有 stub），
// 但 `setOnShowFileSelector` 只在 Android 实现里有意义 —— 调用前
// 必须按 `defaultTargetPlatform.isAndroid` 判断。
import 'package:webview_flutter_android/webview_flutter_android.dart';

import '../../../core/qx_log.dart';
import '../../../data/store/connection_store.dart';
import 'dsh_credential.dart';
import 'webview_file_chooser.dart';
// 返回键三级递进的判定（`decideBackAction` / `DshBackAction`）在
// `webview_host_state.dart` 里——那是纯函数，测试直接钉它。
import 'webview_host_state.dart';

/// DSH 页面宿主。
///
/// 方案 B 的核心：**壳只负责「能不能打开」，页面负责「打开后长什么样」**。
///
/// 所以这个 widget **刻意不重画任何 DSH 界面** —— 那是重写不是移植。
/// 它只做五件壳该做的事：
///   1. 加载前注入 `qx_token` cookie（否则网关 401，页面白屏）
///   2. 处理返回键（WebView 历史 -> 设备列表 -> 退出）
///   3. 外部链接交系统浏览器（同源留在 WebView）
///   4. **接上原生文件选择器**（页面里的 `<input type="file">`）
///   5. 把加载状态与错误显式呈现（不假装「正在加载」）
///
/// ## 关于「定制层」
///
/// 历史上的移动定制层（L3：`custom.css` + `custom.js`，在页面内重建
/// 导航/工具卡/审批）已于 2026-10-31 按用户要求整体下线——**UI 美化
/// 一律不要**，只保留「输入框 + 软键盘」的功能性适配（见
/// `mobile/examples/minimal-input/`）。
///
/// 因此本页**不再探测 `window.qxMobile`**，也不再挂恒显的状态条：
/// 那条 28px 的「正在检测定制层…」是顶栏下面多出来的第二行，
/// 手机上尤其刺眼。
class DshPage extends ConsumerStatefulWidget {
  const DshPage({super.key, required this.connection});

  final Connection connection;

  @override
  ConsumerState<DshPage> createState() => _DshPageState();
}

class _DshPageState extends ConsumerState<DshPage> {
  WebViewController? _controller;

  String? _loadError;

  @override
  void initState() {
    super.initState();
    unawaited(_init());
  }

  @override
  void dispose() {
    super.dispose();
  }

  Future<void> _init() async {
    final connection = widget.connection;
    final url = Uri.parse(connection.url);

    // ⚠️ **凭据有两个可能的位置，且旧数据只有第一个。**
    //
    // 实测模拟器上的真实存储（RN 客户端写的）：
    //   { "url": "http://192.168.20.2:23090/qx-gate?token=<64hex>",
    //     "qxToken": null }
    // 即 **token 在 url 的 query 里，qxToken 字段是 null**。
    //
    // 只读 `connection.qxToken` 会得到「没有凭据」——
    // 那是个**假象**：用户明明配对过，列表里也显示「在线」
    // （探活走 GatewayClient，它自己会从 url 提 token），
    // 但 WebView 宿主读 `qxToken` 就说没凭据。
    //
    // 判定逻辑在 dsh_credential.dart（纯函数 + 6 个测试），
    // 因为这是「真机才暴露、widget 测试碰不到」的那类坑。
    final token = resolveCredential(url, connection.qxToken);
    if (token == null) {
      setState(() => _loadError = '这条连接没有凭据。重新扫码配对一次。');
      return;
    }

    // ⚠️ **必须在 loadRequest 之前写 cookie。**
    // 千寻网关的 `/api/*` 与 `/qx-mobile/*` 都要求已配对
    // （mobile_ui.rs:66 `state.mobile_authorized`）。没有 cookie 时
    // 页面能加载 HTML，但所有 fetch 401 —— 表现为「界面在但数据全空」，
    // 比白屏更难查。
    //
    // 顺带：不能把 token 放 URL（`?token=`）——
    // 配对链接带一次性凭据，URL 会进日志与浏览历史。
    // query 只用于「配对前探测」。
    //
    // API 形状（读 webview_flutter 4.14.1 源码确认，别凭记忆写）：
    //   WebViewCookieManager()                —— 带 WebView 前缀
    //   WebViewCookie(name:, value:, domain:) —— **具名参数**，path 默认 '/'
    //   loadRequest(Uri)                      —— 收 Uri，不是 String
    // 三处都曾凭记忆写错，报错完全指不到根因，所以写在这里。
    final domain = url.host;
    final cookieManager = WebViewCookieManager();
    await cookieManager.setCookie(
      WebViewCookie(name: 'qx_token', value: token, domain: domain),
    );

    final controller = WebViewController()
      ..setJavaScriptMode(JavaScriptMode.unrestricted)
      // 千寻网关是明文 HTTP，需要放开混合内容拦截
      ..setNavigationDelegate(
        NavigationDelegate(
          onPageStarted: (uri) {
            // ⚠️ NavigationDelegate 的 uri 参数是 **String**，
            // 不是 Uri。直接 `.host` / `.path` 会报
            // 「The getter 'host' isn't defined for the type 'String'」。
            final parsed = Uri.tryParse(uri);
            qxlog.info(
                'dsh', '页面开始加载 ${parsed?.host ?? '?'}${parsed?.path ?? ''}');
            if (mounted) setState(() => _loadError = null);
          },
          onPageFinished: (uri) async {
            qxlog.info('dsh', '页面加载完成 ${Uri.tryParse(uri)?.path ?? uri}');
          },
          onWebResourceError: (error) {
            // ⚠️ 只上报**主文档**错误。子资源（字体/图片）失败在移动网络上
            // 很常见，报上来会让「页面其实能用」显示成「加载失败」。
            if (error.isForMainFrame != true) return;
            qxlog.warn('dsh', '主文档加载失败：${error.description}');
            if (mounted) {
              setState(() => _loadError = error.description);
            }
          },
          onNavigationRequest: (request) {
            // 外部链接交系统浏览器，同源留在 WebView。
            // 依据 dsh-mobile README.zh-CN.md:43「同源主页面留在 WebView；
            // 用户点击的外部 HTTPS 链接交给系统浏览器」。
            //
            // ⚠️ `request.url` 是 **String**，不是 Uri —— 直接取 `.host`
            // 会报「The getter 'host' isn't defined for the type 'String'」。
            final target = Uri.tryParse(request.url);
            if (target != null && !isSameOrigin(target, url)) {
              _openExternally(request.url);
              return NavigationDecision.prevent;
            }
            return NavigationDecision.navigate;
          },
        ),
      )
      // 让页面知道壳的存在。仅用于 Surface 切换等原生能力，
      // **不传凭据** —— 页面同源 fetch 已自带 cookie。
      ..addJavaScriptChannel(
        'qxShell',
        onMessageReceived: (message) => _onShellMessage(message.message),
      );

    _controller = controller;

    // ⚠️ **文件选择器必须在这里接**（不接 = 点了「文件」什么都不发生）。
    //
    // DSH 的「+ -> 文件」是页面里的原生 `<input type="file">`
    // （`dsh-client-ui-conversation` 的 `fileInputRef.current?.click()`）。
    // Android WebView 上，file input 的弹框**唯一**入口是
    // `WebChromeClient.onShowFileChooser`：
    // - webview_flutter 的 Java 侧 `returnValueForOnShowFileChooser` 默认
    //   **false** -> `onShowFileChooser` 返回 false -> WebView 认为没人处理
    //   -> **静默不弹框**（不报错，最难查的那种）；
    // - 只有注册了回调，Java 侧才置 true，并把回调返回的 URI 列表喂给
    //   页面的 `File` 对象。
    //
    // RN 端（`react-native-webview`）内置了这条链路，所以那边一直好用。
    //
    // ⚠️ 这是 **Android 专有扩展**（`AndroidWebViewController`），iOS 不存在
    // —— 所以要按运行平台判断，不能无条件调用。
    await _attachFileChooser(controller);

    // ⚠️ 必须剥掉 query —— 旧数据的 url 里带 `?token=<64hex>`，
    // 直接加载会让它进 WebView 的历史与（万一的）日志。
    // token 已经进 cookie 了，url 不需要再带一遍。
    final target = stripCredential(url).resolve('/');
    qxlog.info('dsh', '加载 $target');
    await controller.loadRequest(target);
  }

  /// 接上原生文件选择器（Android 专有 API，iOS 走系统默认）。
  Future<void> _attachFileChooser(WebViewController controller) async {
    if (defaultTargetPlatform != TargetPlatform.android) return;
    // ⚠️ 必须用 `as` 强制转换，**不能**靠 `is` 的类型提升：
    // `AndroidWebViewController` 继承的是 `PlatformWebViewController`，
    // 与 `WebViewController` **没有继承关系**（两者只是实现同一接口），
    // Dart 的类型提升规则在这里不适用——写 `is` 检查后仍按基类解析，
    // 编译报「The method 'setOnShowFileSelector' isn't defined for the
    // type 'WebViewController'」。
    if (controller is! AndroidWebViewController) {
      qxlog.warn('dsh', '当前不是 Android WebView 控制器，跳过文件选择器接线');
      return;
    }
    final androidController = controller as AndroidWebViewController;
    await androidController.setOnShowFileSelector((params) {
      qxlog.info('dsh',
          '页面请求选择文件（多选=${params.mode == FileSelectorMode.openMultiple}）');
      return WebViewFileChooser.pick(
        allowMultiple: params.mode == FileSelectorMode.openMultiple,
        acceptTypes: params.acceptTypes,
      );
    });
    qxlog.info('dsh', '文件选择器已接线');
  }

  bool isSameOrigin(Uri candidate, Uri base) {
    return candidate.scheme == base.scheme &&
        candidate.host == base.host &&
        candidate.port == base.port;
  }

  /// 从配对链接里取长期 token（兼容旧数据）。
  ///
  /// 已移到 `dsh_credential.dart` 的 `resolveCredential()` —— 那里有 6 个
  /// 测试钉住，理由写在该文件的注释里。留在 State 里就只能靠真机验。
  ///
  /// ⚠️ WebView 加载的地址必须用 `stripCredential()` 剥掉 query：
  /// `?token=<64hex>` 会跟着进路由栈与浏览历史，token 只进 cookie。

  void _openExternally(String url) {
    qxlog.info('dsh', '外部链接交系统浏览器：$url');
  }

  void _onShellMessage(String raw) {
    // 页面 -> 壳。目前只有 Surface 切换。
    qxlog.info(
        'dsh', '页面消息：${raw.length > 200 ? '${raw.substring(0, 200)}…' : raw}');
  }

  /// 返回键：三级递进。顺序**不可调换**。
  ///
  /// 判定逻辑在 [decideBackAction]（纯函数，可单测），这里只做执行。
  Future<bool> _handleBack() async {
    // ⚠️ `Navigator.of(context)` 要在 await **之前**取。
    // 放在 await 之后会跨 async gap 用 context —— 页面可能已卸载，
    // 表现为「返回键偶发崩溃」，且只在真机上出现（测试里时序不同）。
    final navigator = Navigator.of(context);

    var webCanGoBack = false;
    final controller = _controller;
    if (controller != null) {
      try {
        webCanGoBack = await controller.canGoBack();
      } catch (error) {
        // 控制器已释放时 canGoBack 会抛。不当作「有历史」——
        // 那会让用户按返回毫无反应。
        qxlog.warn('dsh', 'canGoBack 失败，按无历史处理：$error');
      }
    }
    if (!mounted) return false;

    final canPop = navigator.canPop();
    final action = decideBackAction(
      webCanGoBack: webCanGoBack,
      routeCanPop: canPop,
      onDeviceListRoot: !canPop,
    );
    qxlog.info('dsh', '返回键 -> $action');
    switch (action) {
      case DshBackAction.webHistory:
        await controller?.goBack();
        return true;
      case DshBackAction.deviceList:
        navigator.pop();
        return true;
      case DshBackAction.exit:
        return false;
    }
  }

  /// 上一帧转发给页面的键盘高度（去重，避免每帧多余的 JS 桥调用）
  double _lastForwardedKb = -1;

  @override
  Widget build(BuildContext context) {
    // Android：壳**不再 resize WebView**（surface 尺寸全程不变 ->
    // 键盘动画由系统独立绘制，不触发 Chromium 整页光栅化）。
    // A/B 实测（scripts/probe_keyboard.py，rAF 帧间隔采样，2026-11-05）：
    //   true（每帧 resize）-> 键盘动画期间 8~9 个 >32ms 长帧（最长 77ms）
    //   false（不 resize） -> **0 个**长帧（最长 15ms）
    //
    // 避让改由页内完成：壳把 Android 11+ 的**逐帧** IME inset 桥接给页面
    // （window.__qxKb(高度px)），页面用 compositor-only 的 transform 让
    // composer 跟手（见 mobile/examples/minimal-input/custom.js §1.5）。
    // 头部与对话区全程静止（比 resize 方案多一层视觉稳定）。
    // 非 Android（桌面/调试）维持 Scaffold 默认行为。
    final bool androidShell = defaultTargetPlatform == TargetPlatform.android;
    final double kbInset = MediaQuery.viewInsetsOf(context).bottom;
    if (androidShell && _controller != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || _controller == null) return;
        if ((kbInset - _lastForwardedKb).abs() < 0.5) return;
        _lastForwardedKb = kbInset;
        unawaited(
          _controller!
              .runJavaScript(
                'window.__qxKb&&window.__qxKb(${kbInset.toStringAsFixed(1)})',
              )
              .catchError((Object _) {}),
        );
      });
    }
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) async {
        if (didPop) return;
        // ⚠️ Navigator 必须在 await 之前取。await 之后页面可能已卸载，
        // 那时用 context 会抛 —— 且只在真机的特定时序下出现。
        final navigator = Navigator.of(context);
        final handled = await _handleBack();
        if (!handled) {
          // 到设备列表根页了，交给系统（退出 App）
          navigator.maybePop();
        }
      },
      child: Scaffold(
        backgroundColor: Theme.of(context).scaffoldBackgroundColor,
        // 键盘避让策略见 build() 顶部的注释：Android 关闭（surface 不变 +
        // 页内 transform 跟手），其余平台维持默认。
        resizeToAvoidBottomInset: !androidShell,
        appBar: AppBar(
          // 顶栏只留「返回 + 机器名」。**不放任何 DSH 相关控件** ——
          // 那些由页面的 custom.js 负责，壳再画一遍就成了两套 UI。
          title: Text(
            widget.connection.name.isEmpty
                ? widget.connection.displayHost
                : widget.connection.name,
            style: Theme.of(context).textTheme.titleMedium,
          ),
          leading: IconButton(
            icon: const Icon(Icons.arrow_back),
            tooltip: '返回设备列表',
            onPressed: () async {
              // 同上：先取 Navigator，再 await。
              final navigator = Navigator.of(context);
              final handled = await _handleBack();
              if (!handled) navigator.maybePop();
            },
          ),
        ),
        body: _body(),
      ),
    );
  }

  Widget _body() {
    if (_loadError != null) {
      return _ErrorView(
        message: _loadError!,
        onRetry: () {
          setState(() => _loadError = null);
          unawaited(_controller?.reload());
        },
      );
    }
    final controller = _controller;
    if (controller == null) {
      return const Center(child: CircularProgressIndicator());
    }
    return WebViewWidget(controller: controller);
  }
}

class _ErrorView extends StatelessWidget {
  const _ErrorView({required this.message, required this.onRetry});

  final String message;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Icons.cloud_off, size: 40),
            const SizedBox(height: 12),
            Text(
              '打不开这台电脑的 DSH 页面',
              style: Theme.of(context).textTheme.titleMedium,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 8),
            Text(
              message,
              style: Theme.of(context).textTheme.bodySmall,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 8),
            Text(
              '若提示网络被拦：千寻网关是明文 HTTP，'
              '需要在 network_security_config.xml 里枚举你的实际地址'
              '（<domain> 不支持网段写法）。',
              style: Theme.of(context).textTheme.bodySmall,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 16),
            FilledButton(onPressed: onRetry, child: const Text('重试')),
          ],
        ),
      ),
    );
  }
}

/// 供测试与调试查询的宿主状态。
final dshHostStateProvider = Provider<WebViewHostState>((ref) {
  return const WebViewHostState();
});

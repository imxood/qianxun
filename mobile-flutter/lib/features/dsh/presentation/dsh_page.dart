import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
// ⚠️ `Cookie` / `CookieManager` / `UriRequest` 都来自这个包，
// **不是** `dart:io`。dart:io 也有一个 `Cookie`，两者签名不同
// （dart:io 的有 path/httpOnly，webview 的只有 name/value/domain），
// 混用会得到「The method 'Cookie' isn't defined for the type」这种
// 指不到根因的报错。
import 'package:webview_flutter/webview_flutter.dart';

import '../../../core/qx_log.dart';
import '../../../data/store/connection_store.dart';
import 'webview_host_state.dart';

/// DSH 页面宿主。
///
/// 方案 B 的核心：**壳只负责「能不能打开」，页面负责「打开后长什么样」**。
/// 界面、导航、工具卡、审批、diff 全部由 `<dsh_home>/mobile-access/custom.js`
/// 在页面内重建（千寻网关注入 `window.qxMobile`）。
///
/// 所以这个 widget **刻意不重画任何 DSH 界面** —— 那是重写不是移植。
/// 它只做四件壳该做的事：
///   1. 加载前注入 `qx_token` cookie（否则网关 401，页面白屏）
///   2. 处理返回键（WebView 历史 → 设备列表 → 退出）
///   3. 外部链接交系统浏览器（同源留在 WebView）
///   4. 把加载状态与错误显式呈现（不假装「正在加载」）
class DshPage extends ConsumerStatefulWidget {
  const DshPage({super.key, required this.connection});

  final Connection connection;

  @override
  ConsumerState<DshPage> createState() => _DshPageState();
}

class _DshPageState extends ConsumerState<DshPage> {
  WebViewController? _controller;
  Timer? _readyTimer;

  /// 页面自己报告的「定制层已就绪」。null = 还没报告。
  ///
  /// 用来区分两种白屏：**网关上没东西**（我们的问题）与
  /// **页面加载了但定制层没生效**（custom.js 的问题）。
  /// 两者都表现为白屏，但排查方向完全相反。
  bool? _customLayerReady;

  String? _loadError;

  @override
  void initState() {
    super.initState();
    unawaited(_init());
  }

  @override
  void dispose() {
    _readyTimer?.cancel();
    super.dispose();
  }

  Future<void> _init() async {
    final connection = widget.connection;
    final url = Uri.parse(connection.url);
    final token = connection.qxToken;

    if (token == null || token.isEmpty) {
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
            qxlog.info('dsh', '页面开始加载 ${parsed?.host ?? '?'}${parsed?.path ?? ''}');
            if (mounted) setState(() => _loadError = null);
          },
          onPageFinished: (uri) async {
            qxlog.info('dsh', '页面加载完成 ${Uri.tryParse(uri)?.path ?? uri}');
            // 控制器在 onPageFinished 之前已赋值（loadRequest 之前），
            // 但闭包里直接引用局部变量会报「referenced_before_declaration」
            // —— 用 State 的字段。
            final controller = _controller;
            if (controller != null) await _probeCustomLayer(controller);
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

    final target = url.resolve('/');
    qxlog.info('dsh', '加载 $target');
    await controller.loadRequest(target);

    // 兜底：页面若没报告定制层状态，20 秒后收掉「正在检测」，
    // 避免一个永远转下去的指示器（那比白屏更让人困惑）。
    _readyTimer = Timer(const Duration(seconds: 20), () {
      if (mounted && _customLayerReady == null) {
        setState(() => _customLayerReady = false);
      }
    });
  }

  bool isSameOrigin(Uri candidate, Uri base) {
    return candidate.scheme == base.scheme &&
        candidate.host == base.host &&
        candidate.port == base.port;
  }

  void _openExternally(String url) {
    qxlog.info('dsh', '外部链接交系统浏览器：$url');
  }

  /// 问页面「定制层在不在」。这决定了白屏该往哪查。
  Future<void> _probeCustomLayer(WebViewController controller) async {
    try {
      final result = await controller.runJavaScriptReturningResult(
        'typeof window.qxMobile === "object" && window.qxMobile !== null',
      );
      final present = result == true || result == 'true';
      if (mounted) setState(() => _customLayerReady = present);
      qxlog.info('dsh', '定制层 window.qxMobile ${present ? "在" : "不在"}');
      if (!present) {
        qxlog.warn('dsh',
          '页面里没有 window.qxMobile —— custom.js 不会执行。'
          '检查千寻是否以含 remote 模块的构建在跑，'
          '以及 <dsh_home>/mobile-access/custom.js 是否存在。');
      }
    } catch (error) {
      qxlog.warn('dsh', '探测定制层失败：$error');
      if (mounted) setState(() => _customLayerReady = false);
    }
  }

  void _onShellMessage(String raw) {
    // 页面 → 壳。目前只有 Surface 切换。
    qxlog.info('dsh', '页面消息：${raw.length > 200 ? '${raw.substring(0, 200)}…' : raw}');
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
    qxlog.info('dsh', '返回键 → $action');
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

  @override
  Widget build(BuildContext context) {
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
          bottom: _statusBar(),
        ),
        body: _body(),
      ),
    );
  }

  PreferredSizeWidget? _statusBar() {
    final error = _loadError;
    if (error != null) {
      return _StatusStrip(
        text: '加载失败：$error',
        color: Theme.of(context).colorScheme.error,
        onRetry: () {
          setState(() => _loadError = null);
          unawaited(_controller?.reload());
        },
      );
    }
    final ready = _customLayerReady;
    if (ready == null) {
      return const _StatusStrip(text: '正在检测定制层…', color: Colors.blueGrey);
    }
    if (!ready) {
      return _StatusStrip(
        text: '页面已加载，但 custom.js 未生效（window.qxMobile 不存在）',
        color: Colors.amber.shade700,
      );
    }
    return null;
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

class _StatusStrip extends StatelessWidget implements PreferredSizeWidget {
  const _StatusStrip({required this.text, required this.color, this.onRetry});

  final String text;
  final Color color;
  final VoidCallback? onRetry;

  @override
  Size get preferredSize => const Size.fromHeight(28);

  @override
  Widget build(BuildContext context) {
    return Container(
      height: 28,
      color: color.withValues(alpha: 0.14),
      padding: const EdgeInsets.symmetric(horizontal: 12),
      child: Row(
        children: [
          Expanded(
            child: Text(
              text,
              style: TextStyle(fontSize: 12, color: color),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          if (onRetry != null)
            GestureDetector(
              onTap: onRetry,
              child: Text(
                '重试',
                style: TextStyle(
                  fontSize: 12,
                  color: color,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
        ],
      ),
    );
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

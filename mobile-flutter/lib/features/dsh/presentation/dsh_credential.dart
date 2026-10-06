/// WebView 宿主的凭据解析 —— 从「真机才暴露的坑」里抽出来的纯函数。
///
/// ## 为什么要抽出来
///
/// 这段逻辑最初直接写在 `dsh_page.dart` 的 `_init()` 里，只读
/// `connection.qxToken`。真机上一跑就挂：
///
/// ```
/// 设备列表：DESKTOP-CLTN8CB  ● 在线        ← 看起来没问题
/// 点进去：  加载失败：这条连接没有凭据        ← 但进不去
/// ```
///
/// 原因在实测的存储里（RN 客户端写的格式）：
///
/// ```json
/// { "url": "http://192.168.20.2:23090/qx-gate?token=<64hex>",
///   "qxToken": null }
/// ```
///
/// **token 在 url 的 query 里，`qxToken` 字段是 null。**
/// 设备列表显示「在线」是因为探活走 `GatewayClient`，它自己会从 url 提
/// token —— 所以列表正常、详情失败，两个页面对「凭据在哪」的认知不一致。
///
/// 抽成纯函数后，这个坑被 6 个测试钉住，且**不需要真机**。
library;

/// token 的最小合理长度。
///
/// 64 位 hex 是当前格式；32 位也见过。低于 32 一律视为无效 ——
/// 一个 1 位的 `?token=x` 拿去发请求只会得到一个毫无意义的 401，
/// 而用户完全看不懂为什么。
const int kMinTokenLength = 32;

/// 取这条连接的凭据。
///
/// **优先级：`qxToken` 字段 > url 的 `?token=`。**
///
/// 两者都要认，因为它们对应两种不同时代的数据：
/// - 新数据：兑换后 token 落在 `qxToken` 字段，url 是干净的
/// - 旧数据（RN 客户端写的）：token 落在 url，`qxToken` 是 null
///
/// 返回 null 表示**没有可用凭据**，调用方应提示「重新扫码配对一次」。
///
/// ⚠️ `?code=` 这种情况**提不出 token**，返回 null 是不够的：
/// `?code=` 是**一次性兑换码**，必须真的去 `/qx-gate` 兑换一次才拿得到
/// token。拿它当 token 发出去必然 401。所以这里不猜。
String? resolveCredential(Uri url, String? qxToken) {
  if (qxToken != null && qxToken.length >= kMinTokenLength) return qxToken;

  final fromUrl = url.queryParameters['token'];
  if (fromUrl != null && fromUrl.length >= kMinTokenLength) return fromUrl;

  return null;
}

/// 剥掉 query 与 fragment，只留 origin —— 给 WebView 加载用。
///
/// ⚠️ **绝不能**把带凭据的 url 直接交给 WebView 或写进日志：
/// `?token=<64hex>` 会跟着进路由栈、浏览历史与 logcat。
/// token 只该进 cookie。
Uri stripCredential(Uri url) => url.replace(query: '', fragment: '');

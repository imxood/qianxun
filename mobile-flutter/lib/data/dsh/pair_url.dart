import 'dart:convert';
import 'qx_protocol.dart';

/// 配对链接。千寻桌面端「远程访问」生成的那条 URL。
///
/// 两种形态，**都要认**：
///
/// - `http://<ip>:<port>/qx-gate?code=<32 hex>` —— 当前格式。一次性凭据，
///   5 分钟有效，兑换后立即消费。
/// - `http://<ip>:<port>/qx-gate?token=<64 hex>` —— 旧格式，直接携带长期
///   token。新版网关仍兼容，但不再签发。
///
/// RN 客户端的正则只认 `?token=`，导致**当前签发的链接一律被拒**，
/// 且 `searchParams.get('token')` 取到空串 → 探测永远 401 → 状态点永远
/// 灰。本类把两种都解析掉。
class PairUrl {
  const PairUrl._({
    required this.origin,
    required this.port,
    required this.code,
    required this.token,
  });

  /// `http://192.168.1.9:23090` —— 已归一化，无尾斜杠。
  final String origin;
  final int port;

  /// 一次性兑换码（`?code=`）。旧链接为 null。
  final String? code;

  /// 长期设备 token（`?token=`）。新链接为 null。
  final String? token;

  /// 是否有可用凭据。
  bool get hasCredential =>
      (code != null && code!.isNotEmpty) || (token != null && token!.isNotEmpty);

  /// 展示用主机名（含端口）。
  String get host => Uri.parse(origin).host;

  String get displayHost => '${Uri.parse(origin).host}:$port';

  /// 从字符串解析。返回 null 表示**不是**一条可用的配对链接。
  ///
  /// 刻意比网关本身更严格：主机名为空时直接拒绝。桌面端在未启用远程
  /// 访问、或没选适配器时会生成 `http://:23090/qx-gate?code=…` 这种
  /// 空主机链接，客户端应当在添加阶段就给出「先去开启远程访问」的提示，
  /// 而不是发一个必然失败的请求。
  static PairUrl? tryParse(String raw) {
    final text = raw.trim();
    if (text.isEmpty) return null;

    final Uri uri;
    try {
      uri = Uri.parse(text);
    } on FormatException {
      return null;
    }

    // 只接受 http：网关没有 TLS，密文性由 EasyTier 隧道提供。
    if (uri.scheme != 'http') return null;
    if (uri.host.isEmpty) return null;
    if (uri.path != QxRoute.gate) return null;

    final port = uri.hasPort ? uri.port : QxPort.release;
    if (port <= 0 || port > 65535) return null;

    final code = _hex(uri.queryParameters['code'], minLen: 16);
    final token = _hex(uri.queryParameters['token'], minLen: 32);

    // 两种凭据都没有 → 不是配对链接（可能是手输的普通地址）。
    if (code == null && token == null) return null;

    return PairUrl._(
      origin: '${uri.scheme}://${uri.host}:$port',
      port: port,
      code: code,
      token: token,
    );
  }

  /// 校验一条输入是否是可接受的配对链接（供 UI 给出即时反馈）。
  static bool isValid(String raw) => tryParse(raw) != null;

  /// 供探测器使用的凭据串。
  ///
  /// 优先用长期 token：它不过期，可以直接挂在每次请求的 query 上，
  /// 免去先兑换 cookie 的那一跳。只有新式 `code=` 链接才需要走兑换。
  String? get bearer => token ?? code;

  /// 走 cookie 还是走 query。
  ///
  /// 网关对 `/api/*` 与 WS 升级同时接受 `qx_token` cookie 和 `?token=`，
  /// 但 **cookie 优先级更高且不会进日志**。我们两种都用：
  /// 兑换后存 cookie（主），同时保留 query token 作为兜底。
  bool get prefersCookieExchange => token == null;

  /// 归一化后的完整链接（用于存储与展示）。
  String get normalized {
    final c = code;
    final t = token;
    if (c != null) return '$origin${QxRoute.gate}?code=$c';
    if (t != null) return '$origin${QxRoute.gate}?token=$t';
    return '$origin${QxRoute.gate}';
  }

  /// 拼出 `?token=` 鉴权可用的绝对 URL。
  ///
  /// 用于 `/qx-mobile/info` 这类「支持 query 鉴权」的探测。
  /// 没有长期 token 时返回 null——`?code=` 不能用于 info，那是一次性的。
  Uri? infoUri() {
    final t = token;
    if (t == null) return null;
    return Uri.parse('$origin${QxRoute.info}?token=$t');
  }

  static String? _hex(String? raw, {required int minLen}) {
    if (raw == null) return null;
    final v = raw.trim();
    if (v.length < minLen) return null;
    if (!RegExp(r'^[0-9a-fA-F]+$').hasMatch(v)) return null;
    return v.toLowerCase();
  }

  @override
  String toString() => normalized;
}

/// 从 URL 中抽出 `qx_token` cookie 的值。
///
/// 网关在兑换成功时返回 `Set-Cookie: qx_token=<64hex>; Path=/; ...`。
/// Dart 的 `HttpClient` 没有自动 cookie jar（`package:http` 也没有），
/// 必须自己解析并手工附带——包括 WebSocket 升级请求的头。
///
/// 这是原生客户端相对 WebView 唯一需要补齐的便利设施。
String? parseQxTokenCookie(Iterable<String> setCookieHeaders) {
  const marker = '${QxCookie.token}=';
  for (final header in setCookieHeaders) {
    for (final part in header.split(';')) {
      final trimmed = part.trim();
      if (!trimmed.startsWith(marker)) continue;
      final value = trimmed.substring(marker.length).trim();
      if (value.isNotEmpty) return value;
    }
  }
  return null;
}

/// 拼接 `Cookie` 请求头。
String? cookieHeader(String? qxToken) =>
    qxToken == null || qxToken.isEmpty ? null : '${QxCookie.token}=$qxToken';

/// 宽松 JSON 解码：网关偶发的空 body / 非 JSON 不应炸掉整条链路。
Object? tryDecodeJson(String body) {
  if (body.trim().isEmpty) return null;
  try {
    return jsonDecode(body);
  } on FormatException {
    return null;
  }
}

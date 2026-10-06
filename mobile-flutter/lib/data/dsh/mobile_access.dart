import 'package:flutter/foundation.dart';

/// 壳层能拿到的、关于一条 mobile-access 连接的全部信息。
///
/// 方案 B 的连接**不是**网关（qx-gate），而是 DSH Mobile 插件自带的
/// HTTPS 端点。两者凭据体系完全独立：
///
/// | | 网关 | mobile-access |
/// |---|---|---|
/// | 地址 | `http://ip:23090` | `https://ip:3443` |
/// | 凭据 | `qx_token`（长期设备 token） | `dsh_ma_device` / `dsh_ma_session` |
/// | TLS | 明文 | **自签 CA，必须固定** |
/// | 换 session | 不需要 | **需要**（`/mobile-access/auth/renew`） |
///
/// 依据：`dsh-mobile/src/http-security.ts:4-10`、`src/gateway.ts:1010-1108`。
@immutable
class MobileAccessTarget {
  const MobileAccessTarget({
    required this.origin,
    required this.caPem,
    this.caSha256,
    this.instanceId,
    this.expiresAt,
  });

  /// 归一化后的 origin，例如 `https://192.168.20.2:3443`。无尾斜杠。
  final Uri origin;

  /// 该 origin 的自签 CA（DER base64 或 PEM 原文，取决于获取方式）。
  ///
  /// ⚠️ **不能把它装进系统信任库**。dsh-mobile 的 CA **没带 Authority
  /// Key Identifier**（AKI），走标准链验证的实现（Python/OpenSSL、部分
  /// 企业代理）会报 `Missing Authority Key Identifier`。
  /// Android WebView 若走系统信任库同样会失败。
  ///
  /// 所以必须**按精确 Origin 固定**：在
  /// `onReceivedSslError` 里核对签发者指纹后 `proceed()`。
  /// 依据：`dsh-mobile/README.zh-CN.md:41`「CA 只在 App 内固定；
  /// 仅接受它为精确主机和有效期签发的服务器证书」。
  final String caPem;

  /// CA 的 SHA-256 指纹（小写 hex，无冒号）。
  ///
  /// 固定**指纹**而不是 CA 原文：CA 会被续签（换 key），
  /// 而续签时会被发现并要求重新确认（`dsh-mobile/README.zh-CN.md:25`
  /// 描述了「核对 SHA-256 指纹」这一步）。存指纹让「同一台电脑」可识别，
  /// 存 CA 原文让续签被误判成攻击。两者都要，语义不同。
  final String? caSha256;

  /// DSH 实例标识。用于「这台电脑还是那台吗」的判定
  /// （`README.zh-CN.md:19`：用稳定的安装标识找回同一台电脑）。
  final String? instanceId;

  /// deviceToken 的到期时间（epoch ms）。null = 未知或不过期。
  final int? expiresAt;

  /// 站点根（不是 API）。WebView 首屏加载它。
  Uri get pageUrl => origin;

  /// 是否已过期。
  bool get isExpired {
    final at = expiresAt;
    if (at == null) return false;
    return DateTime.now().millisecondsSinceEpoch >= at;
  }

  /// 只存 origin 的协议+主机+端口。**不存路径/查询/fragment**。
  ///
  /// 依据：`README.zh-CN.md:43`「只保存协议、规范化主机和端口；
  /// 普通路径、查询和 fragment 不持久化」。理由是配对链接里带一次性
  /// 凭据，持久化路径等于把凭据存下来。
  String get persistKey => '${origin.scheme}://${origin.host}:${origin.port}';

  /// 从 DSH 的 `setup.json` 解析。
  ///
  /// ⚠️ **实测 `setup.json` 没有 `origin` 字段**，只有
  /// `{version, networkInterface, listenPort, upstreamOrigin, tls:{…}}`。
  /// origin 要自己用 `networkInterface` 对应的网卡地址 + `listenPort` 拼 ——
  /// 因为证书上的 SAN 是**按选定网卡签的**，用错网卡就是证书不匹配。
  factory MobileAccessTarget.fromSetup(
    Map<String, dynamic> setup, {
    required Uri lanAddress,
    String? caPem,
  }) {
    final port = (setup['listenPort'] as num?)?.toInt() ?? 3443;
    return MobileAccessTarget(
      origin: Uri(scheme: 'https', host: lanAddress.host, port: port),
      caPem: caPem ?? '',
    );
  }

  @override
  bool operator ==(Object other) =>
      other is MobileAccessTarget && other.persistKey == persistKey;

  @override
  int get hashCode => persistKey.hashCode;

  @override
  String toString() => 'MobileAccessTarget($persistKey, instance=$instanceId)';
}

/// 壳层持有的凭据。**不进日志、不进 toString、不落明文**。
///
/// 三层凭据（`http-security.ts:4-6`）：
/// - `deviceToken` 长期，Keystore 加密，只用于 `/auth/renew`
/// - `sessionToken` 短期，session 期间有效
/// - `csrfToken` 短期，**非** HttpOnly（页面要读）
@immutable
class MobileAccessCredentials {
  const MobileAccessCredentials({
    required this.deviceId,
    required this.instanceId,
    required this.deviceToken,
    required this.sessionToken,
    required this.csrfToken,
    required this.sessionExpiresAt,
    required this.deviceExpiresAt,
  });

  final String deviceId;
  final String instanceId;
  final String deviceToken;
  final String sessionToken;
  final String csrfToken;

  /// epoch ms。
  final int sessionExpiresAt;
  final int deviceExpiresAt;

  /// session 是否已过期（含 30s 提前量，避免边界抖动）。
  bool get sessionExpired =>
      DateTime.now().millisecondsSinceEpoch >= sessionExpiresAt - 30 * 1000;

  /// device 是否已过期。
  bool get deviceExpired =>
      DateTime.now().millisecondsSinceEpoch >= deviceExpiresAt;

  /// 还剩多少秒过期。用于「该续期了」的提前判断。
  int get sessionSecondsLeft {
    final ms = sessionExpiresAt - DateTime.now().millisecondsSinceEpoch;
    return ms <= 0 ? 0 : (ms / 1000).floor();
  }

  /// 构造 WebView 要设的 Cookie 串。
  ///
  /// ⚠️ **不能**把凭据交给页面（`mobile.js`）—— 页面同源 fetch 会
  /// **自动带浏览器已有的 cookie**，壳不需要、也不该把 token 注入到
  /// `window` 上。`dsh-mobile/README.zh-CN.md:53` 明确桥「不暴露
  /// Cookie、设备令牌、配对密钥、CA 私钥」。
  ///
  /// 所以这个方法只给壳自己用（原生请求），不给 JS。
  String get cookieHeader => 'dsh_ma_session=$sessionToken; dsh_ma_csrf=$csrfToken';

  @override
  String toString() =>
      'MobileAccessCredentials(device=$deviceId, instance=$instanceId, '
      'session ${sessionSecondsLeft}s left)';
}

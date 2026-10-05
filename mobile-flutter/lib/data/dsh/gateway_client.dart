import 'dart:async';
import 'dart:io';

import 'package:http/http.dart' as http;
import '../../core/qx_log.dart';

import 'pair_url.dart';
import 'qx_protocol.dart';

/// 网关自述身份（`GET /qx-mobile/info` 的响应）。
class HostInfo {
  const HostInfo({
    this.app,
    this.version,
    this.hostname,
    this.dshReady = false,
  });

  final String? app;
  final String? version;
  final String? hostname;

  /// DSH 启动 URL 是否已知。
  ///
  /// ⚠️ 这**不等于**「DSH 活着」——它只反映桌面端是否拿到了启动地址。
  /// DSH 重启期间它可能是陈旧的。真正的存活判断要看
  /// `/api/remote.mux` 能不能建立。
  final bool dshReady;

  factory HostInfo.fromJson(Map<String, Object?> json) => HostInfo(
        app: json['app'] as String?,
        version: json['version'] as String?,
        hostname: json['hostname'] as String?,
        dshReady: json['dshReady'] == true,
      );

  Map<String, Object?> toJson() => {
        'app': app,
        'version': version,
        'hostname': hostname,
        'dshReady': dshReady,
      };
}

/// 一次连接尝试的结果。
sealed class ProbeResult {
  const ProbeResult();
}

/// 网关可达，且已完成鉴权。
final class ProbeOk extends ProbeResult {
  const ProbeOk(this.info, {this.qxToken});

  final HostInfo info;

  /// 兑换得到的 `qx_token`，新式 `?code=` 链接才会有。
  final String? qxToken;
}

/// 网关可达但拒绝了凭据。
final class ProbeUnauthorized extends ProbeResult {
  const ProbeUnauthorized(this.message);
  final String message;
}

/// 网关可达，但上游 DSH 不可达（HTTP 502）。
final class ProbeUpstreamDown extends ProbeResult {
  const ProbeUpstreamDown(this.message);
  final String message;
}

/// 完全连不上（网络 / 超时 / 主机不存在）。
final class ProbeUnreachable extends ProbeResult {
  const ProbeUnreachable(this.message);
  final String message;
}

/// 链接本身不合法。
final class ProbeBadUrl extends ProbeResult {
  const ProbeBadUrl(this.message);
  final String message;
}

/// 千寻网关客户端。
///
/// 职责边界很窄：只管**网关自有路由**（配对兑换 + 身份探测）。
/// 所有 `/api/*` 与 WebSocket 归 [DshClient] / [StreamMux]。
///
/// 这个划分对应网关的真实结构——`/{*rest}` 是透传，不是网关逻辑。
class GatewayClient {
  GatewayClient({http.Client? client}) : _client = client ?? http.Client();

  final http.Client _client;

  static const Duration _timeout = Duration(seconds: 6);

  void dispose() => _client.close();

  /// 探测一台电脑。
  ///
  /// 流程按凭据形态分两种：
  /// - 旧式 `?token=`：直接带 query 调 `/qx-mobile/info`（网关对该路由
  ///   放开 CORS 且支持 query 鉴权），**无需兑换**。
  /// - 新式 `?code=`：先 `GET /qx-gate?code=…` 换 cookie，再用 cookie 调
  ///   info。这是必须的两跳。
  Future<ProbeResult> probe(String rawUrl) async {
    final pair = PairUrl.tryParse(rawUrl);
    if (pair == null) {
      qxlog.warn('probe', '链接解析失败：${_brief(rawUrl)}');
      return const ProbeBadUrl(
        '链接形态不对：请扫电脑端「远程访问」的配对二维码，或完整粘贴配对链接。',
      );
    }
    qxlog.info('probe', '探测 ${pair.origin}  凭据=${_credHint(pair)}');

    // 旧链接：一步到位。
    final infoUri = pair.infoUri();
    if (infoUri != null) {
      qxlog.debug('probe', '走旧式 query 鉴权（免兑换）');
      final res = await _get(infoUri, qxToken: null);
      return switch (res) {
        _Ok(:final body) => _infoOrBad(body),
        _HttpFail(:final status, :final body) => _classifyHttp(status, body),
        _NetFail(:final message) => ProbeUnreachable(message),
      };
    }

    // 新链接：先兑换。
    qxlog.debug('probe', '走新式 ?code= 兑换');
    final redeem = await _get(
      Uri.parse('${pair.origin}${QxRoute.gate}?code=${pair.code}'),
      qxToken: null,
    );
    switch (redeem) {
      case _HttpFail(:final status, :final body):
        return _classifyHttp(status, body);
      case _NetFail(:final message):
        return ProbeUnreachable(message);
      case _Ok(headers: final headers):
        final qxToken = parseQxTokenCookie(headers);
        if (qxToken == null) {
          qxlog.warn('probe', '兑换返回 2xx 但没有 Set-Cookie: qx_token');
          return const ProbeUnauthorized(
            '配对码无效或已过期（兑换成功但没拿到凭据），请在电脑端重新生成二维码。',
          );
        }
        qxlog.info('probe', '兑换成功，已拿到 qx_token（${qxToken.length} 字符）');
        // 拿到 cookie 后再确认一次身份，顺便验证 cookie 真的可用。
        final verified = await _get(
          Uri.parse('${pair.origin}${QxRoute.info}'),
          qxToken: qxToken,
        );
        return switch (verified) {
          _Ok(:final body) => _infoOrBad(body, qxToken: qxToken),
          _HttpFail(:final status, :final body) => _classifyHttp(status, body),
          _NetFail(:final message) => ProbeUnreachable(message),
        };
    }
  }

  static String _credHint(PairUrl p) => p.token != null ? 'token(旧式)' : 'code(一次性)';

  static String _brief(String s) => s.length <= 80 ? s : '${s.substring(0, 80)}…';

  ProbeResult _infoOrBad(String body, {String? qxToken}) {
    final decoded = tryDecodeJson(body);
    if (decoded is! Map) {
      return const ProbeUnreachable('网关返回了非 JSON 响应（可能不是千寻网关）。');
    }
    return ProbeOk(HostInfo.fromJson(decoded.cast<String, Object?>()), qxToken: qxToken);
  }

  ProbeResult _classifyHttp(int status, String body) {
    final text = body.trim();
    if (status == 401 || status == 403) {
      return ProbeUnauthorized(
        text.isEmpty ? '凭据无效或已过期，请重新配对。' : text,
      );
    }
    if (status == 502) {
      return ProbeUpstreamDown(
        text.isEmpty ? 'DSH 上游不可达，请确认电脑端 DSH 正在运行。' : text,
      );
    }
    if (status == 404) {
      return const ProbeBadUrl('这个地址上没有千寻远程网关（404）。');
    }
    return ProbeUnreachable(text.isEmpty ? 'HTTP $status' : text);
  }

  Future<_Res> _get(Uri uri, {required String? qxToken}) async {
    qxlog.debug('probe', 'GET $uri  cookie=${qxToken == null ? '无' : '有'}');
    try {
      final res = await _client.get(uri, headers: {
        'Accept': 'application/json',
        if (qxToken != null) 'Cookie': cookieHeader(qxToken)!,
        // 原生客户端不受 CORS 约束，但网关对 info 开了 ACAO:*，
        // 带上 User-Agent 让网关侧日志能识别来源。
        'User-Agent': 'QianxunMobile/0.2 (Flutter)',
      }).timeout(_timeout);

      // 302 说明配对成功，Set-Cookie 才是我们要的。手动跟随一次，
      // 避免进入无界重定向（网关只会发一次 302）。
      if (res.statusCode >= 300 && res.statusCode < 400) {
        final location = res.headers['location'];
        if (location != null) {
          return await _get(uri.resolve(location), qxToken: qxToken);
        }
      }
      if (res.statusCode >= 200 && res.statusCode < 300) {
        qxlog.debug('probe', '← ${res.statusCode} (${res.body.length} B)');
        return _Ok(
          res.statusCode,
          res.body,
          (res.headers['set-cookie'] as List?)?.cast<String>() ?? const <String>[],
        );
      }
      qxlog.warn('probe', '← ${res.statusCode}  ${_clip(res.body, 200)}');
      return _HttpFail(res.statusCode, res.body);
    } on TimeoutException {
      qxlog.warn('probe', '超时 ${_timeout.inSeconds}s：$uri');
      return const _NetFail('连接超时：这台电脑可能不在线，或不在同一个网络里。');
    } on SocketException catch (e) {
      qxlog.warn('probe', 'socket 错误：${e.osError?.message ?? e.message}');
      return _NetFail('无法连接这台电脑（${e.osError?.message ?? e.message}）。');
    } on http.ClientException catch (e) {
      qxlog.warn('probe', 'ClientException：${e.message}');
      return _NetFail('网络错误：${e.message}');
    }
  }

  static String _clip(String s, int max) => s.length <= max ? s : '${s.substring(0, max)}…';
}

/// 三态内部结果。
sealed class _Res {
  const _Res();
}

final class _Ok extends _Res {
  const _Ok(this.status, this.body, this.headers);
  final int status;
  final String body;

  /// 全部 `Set-Cookie` 头。配对兑换的凭据就在这里。
  final List<String> headers;
}

final class _HttpFail extends _Res {
  const _HttpFail(this.status, this.body);
  final int status;
  final String body;
}

final class _NetFail extends _Res {
  const _NetFail(this.message);
  final String message;
}

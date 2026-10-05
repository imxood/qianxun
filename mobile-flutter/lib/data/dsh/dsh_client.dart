import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart' show visibleForTesting;
import 'package:http/http.dart' as http;
import '../../core/qx_log.dart';

import 'pair_url.dart';
import 'qx_protocol.dart';
import 'rpc_envelope.dart';

/// DSH 一元 RPC 客户端。
///
/// 线格式（harness 0.1.2 起未变）：
/// ```http
/// POST /api/<namespace>/<method>
/// {"type":"client-request","rpcId":"<uuid>","method":"<ns/method>","payload":{…}}
/// ```
/// 响应体：
/// ```json
/// {"type":"server-response","rpcId":"<同一 id>","result":{"ok":true,"value":{…}}}
/// ```
///
/// 鉴权靠 `Cookie: qx_token=…`。**`dart:io` 与 `package:http` 都没有
/// cookie jar**——每次请求手工附带，包括 WebSocket 升级。
///
/// ⚠️ 载荷有两层信封，且**键名不统一**。参考项目（Kotlin，锁在 harness
/// 0.1.2-alpha.1）把参数摊平当作 payload，与实际部署不兼容。实测于
/// qianxun 0.7.0 内置的 harness（见 `scripts/probe_rpc.py`）：
///
/// ```jsonc
/// // ✅ 正确
/// {"type":"client-request","rpcId":"…","method":"session/list",
///  "payload":{"args":{"_request":{}}}}              // session/list  特例
/// {"type":"client-request","rpcId":"…","method":"session/page",
///  "payload":{"args":{"request":{…}}}}              // 其余单参方法
/// {"type":"client-request","rpcId":"…","method":"agentPresets/list",
///  "payload":{"args":{}}}                            // 无参方法
///
/// // ❌ 摊平（参考项目的写法）
/// {"type":"client-request","rpcId":"…","method":"session/list","payload":{}}
///   → Remote payload must contain exactly one plain-object args field
///
/// // ❌ 键名写错
/// {"payload":{"args":{"_request":{…}}}}  // 对 session/page 而言
///   → args fields do not match the descriptor: missing "request";
///     unexpected "_request"
/// ```
class DshClient {
  DshClient({
    required this.origin,
    required this.qxToken,
    http.Client? client,
  }) : _client = client ?? http.Client();

  /// `http://192.168.1.9:23090`。
  final String origin;

  /// 兑换来的 `qx_token`。
  final String? qxToken;

  final http.Client _client;

  /// 一元调用默认超时。
  ///
  /// 注意：**流式响应（SSE）不能用这个超时**，网关侧对代理请求刻意不设
  /// 总超时以便 SSE 存活。本客户端的调用都是点对点请求，用 30s 是安全的。
  static const Duration _timeout = Duration(seconds: 30);

  /// 不接受任何参数的方法。args 直接是 `{}`。
  static const Set<String> _noArgMethods = {
    'session/modelCatalog',
    'agentPresets/list',
    'settings/describe',
    'settings/openSettingsDocument',
  };

  /// 把单个请求对象命名为 `_request` 的方法。
  ///
  /// 只此一个（实测）。其余单参方法一律用 `request`——参数名由宿主的
  /// descriptor 声明，**猜不得**，只能打一发看它说什么。
  static const Set<String> _underscoreRequest = {'session/list'};

  /// 无参方法。
  static bool takesNoArgs(String method) => _noArgMethods.contains(method);

  /// 该方法是否用 `_request` 作为参数名。
  static bool usesUnderscoreRequest(String method) =>
      _underscoreRequest.contains(method);

  /// 把请求内容包成 `payload.args`。
  @visibleForTesting
  static Map<String, Object?> wrapArgs(String method, Object? request) {
    if (takesNoArgs(method)) return const {};
    final key = usesUnderscoreRequest(method) ? '_request' : 'request';
    return {key: request ?? const <String, Object?>{}};
  }

  void dispose() => _client.close();

  /// 发一次一元调用。
  ///
  /// [request] 是那个「单个请求对象」的内容；本方法负责把它包进正确的
  /// `{"args": {…}}` 形状。调用方**不要**自己拼 args 层——那正是之前
  /// 全部调用失败的原因。
  ///
  /// 返回 [RpcResult]：业务失败（`ok:false`）与传输失败都收敛到
  /// [RpcErr]，但 code 不同，UI 据此决定「重试」还是「重新配对」。
  Future<RpcResult<Object?>> call(String method, {Object? request}) async {
    final rpcId = _mintRpcId();
    final body = jsonEncode({
      'type': 'client-request',
      'rpcId': rpcId,
      'method': method,
      // 显式省略 null：harness 对「本该缺省的 optional」收到 null 会拒绝。
      'payload': {'args': _stripNulls(wrapArgs(method, request))},
    });

    final uri = Uri.parse('$origin${qxApiPath(method)}');
    final started = DateTime.now();
    qxlog.debug('rpc', '→ $method  request=$_brief(request)');
    try {
      final res = await _client
          .post(
            uri,
            headers: {
              'Content-Type': 'application/json',
              'Accept': 'application/json',
              if (qxToken != null) 'Cookie': cookieHeader(qxToken)!,
              'User-Agent': 'QianxunMobile/0.2 (Flutter)',
            },
            body: body,
          )
          .timeout(_timeout);

      final ms = DateTime.now().difference(started).inMilliseconds;
      if (res.statusCode >= 200 && res.statusCode < 300) {
        final result = parseServerResponse(tryDecodeJson(res.body));
        qxlog.info(
          'rpc',
          '← $method ${res.statusCode} ${ms}ms ok=${result.isOk}'
              '${result.errorOrNull == null ? '' : ' err=${result.errorOrNull!.code}:${result.errorOrNull!.message}'}',
        );
        return result;
      }
      final err = RpcError.fromStatus(res.statusCode, res.body);
      qxlog.warn('rpc', '← $method ${res.statusCode} ${ms}ms ${err.code}');
      return RpcErr<Object?>(err);
    } on TimeoutException {
      qxlog.warn('rpc', '→ $method 超时 ${_timeout.inSeconds}s');
      return const RpcErr<Object?>(
        RpcError(code: RpcErrorCode.timeout, message: '请求超时'),
      );
    } on SocketException catch (e) {
      qxlog.warn('rpc', '→ $method socket 错误：${e.osError?.message ?? e.message}');
      return RpcErr<Object?>(
        RpcError(
          code: RpcErrorCode.network,
          message: '网络不可达：${e.osError?.message ?? e.message}',
        ),
      );
    } on http.ClientException catch (e) {
      qxlog.warn('rpc', '→ $method ClientException：${e.message}');
      return RpcErr<Object?>(
        RpcError(code: RpcErrorCode.network, message: '网络错误：${e.message}'),
      );
    }
  }

  static String _brief(Object? payload) {
    final s = jsonEncode(payload ?? const {});
    return s.length <= 200 ? s : '${s.substring(0, 200)}…';
  }

  /// 便捷封装：调用并把 `value` 收成 [T]。
  ///
  /// 解析失败也算业务错误——harness 返回的形状对不上时，明确报错远好过
  /// 拿一个半空的 map 去渲染。
  Future<RpcResult<T>> callAs<T>(
    String method,
    T Function(Object? value) decode, {
    Object? request,
  }) async {
    final res = await call(method, request: request);
    return switch (res) {
      RpcErr<Object?>(:final error) => RpcErr<T>(error),
      RpcOk<Object?>(:final value) => RpcOk<T>(decode(value)),
    };
  }

  /// 回应 `$events` 上的某个 waterfall。
  ///
  /// 三种 outcome 是**三种不同的决定**，不是程度差别：
  /// - `next`：不回答，让宿主的后续监听器接手。
  /// - `result`：认领这个请求。
  /// - `rejected`：让它失败。
  ///
  /// ⚠️ 用户**关闭**提问必须是 `rejected` 且 code 恰为 `cancelled`，
  /// 其它任何 code 都会让等待一直挂着。而「每条都答空选」是一个**合法
  /// 回答**，模型会读成「没有偏好」——与关闭是两回事。
  Future<RpcResult<Object?>> respondWaterfall({
    required String clientId,
    required String eventId,
    required Map<String, Object?> outcome,
  }) =>
      call(
        DshEndpoint.eventsResult,
        request: {
          'clientId': clientId,
          'eventId': eventId,
          'outcome': outcome,
        },
      );

  /// 构造 `result` outcome。
  static Map<String, Object?> outcomeResult(Object? value) => {
        'kind': 'result',
        if (value != null) 'value': value,
      };

  /// 构造 `rejected` outcome。
  static Map<String, Object?> outcomeRejected({
    required String name,
    required String message,
    String? code,
  }) =>
      {
        'kind': 'rejected',
        'error': {
          'name': name,
          'message': message,
          if (code != null) 'code': code,
        },
      };

  /// 构造 `next` outcome。
  static Map<String, Object?> outcomeNext() => const {'kind': 'next'};
}

/// 递归剔除 null 值。
///
/// 对齐 `explicitNulls = false`：可选字段「缺省」要表现为键不存在，
/// 而不是 `null`。
Object? _stripNulls(Object? value) {
  if (value is Map) {
    final out = <String, Object?>{};
    for (final entry in value.entries) {
      final v = entry.value;
      if (v == null) continue;
      out['${entry.key}'] = _stripNulls(v);
    }
    return out;
  }
  if (value is List) return value.map(_stripNulls).toList(growable: false);
  return value;
}

/// 生成一个关联 id。
///
/// 用时间戳 + 随机数而非 `dart:math` 的 UUID：这里只需要进程内唯一且
/// 跨设备不容易撞，服务端并不校验格式。
String _mintRpcId() {
  final now = DateTime.now().microsecondsSinceEpoch.toRadixString(16);
  final salt = List.generate(8, (_) => (DateTime.now().microsecond % 36).toRadixString(36)).join();
  return '$now-$salt';
}

/// 生成一次人类提示的 `requestId`。
///
/// ⚠️ 与 [rpcId] **不是一回事**：这个标识的是**消息**。宿主会把它持久化
/// 在被接受的那条消息上，所以同一条消息的重试必须带同一个 id，而两条
/// 不同的消息绝不能共用。
String mintPromptRequestId() => 'm${DateTime.now().microsecondsSinceEpoch.toRadixString(36)}';

/// DSH RPC 信封。
///
/// 一元调用走 `POST /api/<namespace>/<method>`，请求体与响应体各自带一个
/// `type` 判别字段。**这两个是 harness 0.1.2 唯一没改的线格式**。
///
/// 服务端响应里的 `result` 是一个「业务成功/失败」的二选一：
/// `{"ok":true,"value":{…}}` 或 `{"ok":false,"error":{…}}`。
/// 注意 `ok:false` 是**业务失败**（HTTP 仍是 200），与传输层的 4xx/5xx
/// 是两回事——混为一谈会让「会话不存在」被显示成「网络断了」。
library;

/// RPC 错误码 → HTTP 状态的映射（见桌面端 `DshApiClient.transportError`）。
abstract final class RpcErrorCode {
  /// 网关/DSH 认为该能力不存在（HTTP 404）。通常是 harness 版本不匹配。
  static const String capabilityUnavailable = 'capability-unavailable';

  /// 鉴权失败（HTTP 401）。
  static const String unauthorized = 'unauthorized';

  /// 主机围栏拒绝（HTTP 403）。
  static const String forbidden = 'forbidden';

  /// 参数 / 载荷不合法（HTTP 400）。
  static const String badRequest = 'bad-request';

  /// 载荷过大（HTTP 413）。
  static const String payloadTooLarge = 'payload-too-large';

  /// 上游不可达（HTTP 502）。
  static const String upstream = 'upstream';

  /// 网络层自身失败（无 HTTP 状态）。
  static const String network = 'network';

  /// 超时。
  static const String timeout = 'timeout';

  /// 兜底。
  static const String internal = 'internal';

  /// 用户主动取消某个提问请求——DSH 只认这一个码。
  static const String cancelled = 'cancelled';
}

/// 一条 RPC 错误。
class RpcError implements Exception {
  const RpcError({
    required this.code,
    required this.message,
    this.details = const <String, Object?>{},
  });

  final String code;
  final String message;
  final Map<String, Object?> details;

  factory RpcError.fromJson(Map<String, Object?> json) => RpcError(
        code: (json['code'] as String?) ?? RpcErrorCode.internal,
        message: (json['message'] as String?) ?? '未知错误',
        details: _asMap(json['details']),
      );

  /// 由 HTTP 状态码合成一条传输层错误。
  factory RpcError.fromStatus(int status, String? body) {
    final (code, fallback) = switch (status) {
      400 => (RpcErrorCode.badRequest, '请求不合法'),
      401 => (RpcErrorCode.unauthorized, '凭据无效或已过期，请重新配对'),
      403 => (RpcErrorCode.forbidden, '主机拒绝了这个连接'),
      404 => (RpcErrorCode.capabilityUnavailable, '这台电脑上的 DSH 不支持该能力'),
      413 => (RpcErrorCode.payloadTooLarge, '请求体过大'),
      502 => (RpcErrorCode.upstream, 'DSH 上游不可达，请确认电脑端 DSH 正在运行'),
      _ => (RpcErrorCode.internal, 'HTTP $status'),
    };
    // 网关的纯文本错误体信息量比状态码大，优先展示。
    final message = (body != null && body.trim().isNotEmpty) ? body.trim() : fallback;
    return RpcError(code: code, message: message, details: {'status': status});
  }

  /// 是否「这台电脑可能已经变了状态」——UI 该据此提示重新配对。
  bool get needsRepairing => code == RpcErrorCode.unauthorized;

  /// 是否网络/上游问题——UI 该据此提示重试而非重配。
  bool get isRetryable =>
      code == RpcErrorCode.network || code == RpcErrorCode.timeout || code == RpcErrorCode.upstream;

  @override
  String toString() => 'RpcError($code): $message';
}

/// 一次调用的结果：成功带值，失败带错误。
///
/// 用 sealed class 而非 nullable，这样调用点必须显式处理失败分支。
sealed class RpcResult<T> {
  const RpcResult();

  /// 是否成功。
  bool get isOk => this is RpcOk<T>;

  /// 成功值；失败时抛 [StateError]。
  T get valueOrThrow => switch (this) {
        RpcOk<T>(:final value) => value,
        RpcErr<T>(:final error) => throw error,
      };

  /// 成功值或 null。
  T? get valueOrNull => switch (this) {
        RpcOk<T>(:final value) => value,
        RpcErr<T>() => null,
      };

  /// 错误；成功时为 null。
  RpcError? get errorOrNull => switch (this) {
        RpcOk<T>() => null,
        RpcErr<T>(:final error) => error,
      };

  /// 折叠成单值。
  R map<R>(R Function(T value) ok, R Function(RpcError error) err) => switch (this) {
        RpcOk<T>(:final value) => ok(value),
        RpcErr<T>(:final error) => err(error),
      };
}

final class RpcOk<T> extends RpcResult<T> {
  const RpcOk(this.value);
  final T value;
}

final class RpcErr<T> extends RpcResult<T> {
  const RpcErr(this.error);
  final RpcError error;
}

/// 一元调用返回的 `result` 槽（`RpcResult<Object?>` 的 JSON 形态）。
///
/// 解析规则对齐桌面端 `RpcResultJsonSerializer`：
/// - `ok:true` 且**无** `value` → 视为空对象（void 成功很常见）。
/// - `ok:false` → `error`；缺 `error` 时合成一条 `internal`。
RpcResult<Object?> parseResultSlot(Object? raw) {
  final map = _asMap(raw);
  if (map.isEmpty) {
    return const RpcErr<Object?>(
      RpcError(code: RpcErrorCode.internal, message: '响应缺少 result 槽'),
    );
  }
  final ok = map['ok'] == true;
  if (ok) {
    return RpcOk<Object?>(map['value'] ?? const <String, Object?>{});
  }
  final err = map['error'];
  return RpcErr<Object?>(
    err is Map
        ? RpcError.fromJson(_asMap(err))
        : const RpcError(
            code: RpcErrorCode.internal,
            message: '服务端返回了失败但没有错误详情',
          ),
  );
}

/// 校验并提取服务端响应信封。
///
/// 桌面端在 `decodeServerResponse` 里 `require(type == "server-response")`。
/// 这里放宽成「类型不符当作传输层损坏」，因为多一层保护比崩掉好。
RpcResult<Object?> parseServerResponse(Object? raw) {
  final map = _asMap(raw);
  if (map.isEmpty) {
    return const RpcErr<Object?>(
      RpcError(code: RpcErrorCode.internal, message: '响应不是合法 JSON 对象'),
    );
  }
  final type = map['type'];
  if (type != 'server-response') {
    return RpcErr<Object?>(
      RpcError(
        code: RpcErrorCode.internal,
        message: '信封类型异常：期望 server-response，收到 ${type ?? 'null'}',
      ),
    );
  }
  return parseResultSlot(map['result']);
}

/// 把 `Object?` 安全收成 `Map<String, Object?>`。
Map<String, Object?> _asMap(Object? value) {
  if (value is Map<String, Object?>) return value;
  if (value is Map) return value.map((k, v) => MapEntry('$k', v));
  return const <String, Object?>{};
}

/// 暴露给其他文件用的宽松 map 工具。
Map<String, Object?> asMap(Object? value) => _asMap(value);

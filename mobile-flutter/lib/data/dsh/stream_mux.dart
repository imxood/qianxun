import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';
import '../../core/qx_log.dart';

import 'pair_url.dart';
import 'qx_protocol.dart';
import 'rpc_envelope.dart';

/// 一条逻辑流的下一项。
sealed class StreamSignal {
  const StreamSignal();
}

final class StreamItem extends StreamSignal {
  const StreamItem(this.value);
  final Object? value;
}

final class StreamFailed extends StreamSignal {
  const StreamFailed(this.error, {required this.carrier});

  /// 业务失败。
  final RpcError error;

  /// 是否**载体**失败（socket 断了），而非业务失败。
  ///
  /// 这个区分决定重试是否有意义：载体失败说明请求本身没问题——是连接
  /// 在它脚底下没了，重新开一代连接是合理的；业务失败是宿主的深思熟虑
  /// 的答复，再问一次会得到一模一样的结果。
  final bool carrier;
}

final class StreamEnded extends StreamSignal {
  const StreamEnded();
}

/// 一条已打开的逻辑流。
///
/// 单消费者：从一个地方读。
///
/// ⚠️ **必须自己排队列，不能用 `controller.stream.first`。**
/// `StreamController` 默认是单订阅的；`.first` 每次都会新建一个订阅，
/// 第一次取完值之后**第二次调用直接抛 `Bad state: Stream has already
/// been listened to`**。表现是：第一帧（`$events` 的 ready）能收到，
/// 紧接着读取循环就崩掉，流被 `finally` 关掉——连接永远走不到就绪态。
///
/// 正确做法：底层订阅**只建一次**，把信号压进队列，`receive()` 从队列取；
/// 队列空时挂一个 Completer 等下一条。
class DshStream {
  DshStream(this._mux, this.streamId, this.endpoint) {
    // 唯一的底层订阅。建好就把控制器让渡出去，避免「有事件但没人在听」
    // 导致的静默丢帧。
    _controller.stream.listen(_enqueue);
  }

  final StreamMux _mux;
  final String streamId;
  final String endpoint;

  final _controller = StreamController<StreamSignal>();
  final _queue = <StreamSignal>[];
  Completer<StreamSignal>? _waiter;
  var _done = false;

  Stream<StreamSignal> get signals => _controller.stream;

  /// 收到一条信号：先唤醒等待者，否则排队。
  void _enqueue(StreamSignal signal) {
    final waiter = _waiter;
    if (waiter != null && !waiter.isCompleted) {
      _waiter = null;
      waiter.complete(signal);
      return;
    }
    _queue.add(signal);
  }

  /// 下一项；宿主结束时返回 null。
  Future<StreamSignal?> receive() {
    if (_queue.isNotEmpty) {
      final signal = _queue.removeAt(0);
      // 终态信号不重复返回。
      if (signal is StreamEnded || signal is StreamFailed) {
        _done = true;
      }
      return Future.value(signal);
    }
    if (_done || _controller.isClosed) {
      return Future.value(const StreamEnded());
    }
    final waiter = Completer<StreamSignal>();
    _waiter = waiter;
    return waiter.future;
  }

  /// 通知宿主停止。幂等；对已死的流调用也安全。
  void cancel() {
    if (_done) return;
    _done = true;
    _mux._removeStream(streamId);
    if (!_mux.isClosed) {
      _mux._send({'type': 'cancel', 'streamId': streamId});
    }
    final waiter = _waiter;
    _waiter = null;
    if (waiter != null && !waiter.isCompleted) {
      waiter.complete(const StreamEnded());
    }
    if (!_controller.isClosed) _controller.close();
  }
}

/// `$events` 上的一个事件帧。
sealed class EventFrame {
  const EventFrame();
}

/// 开帧。证明宿主装好了增量监听器才会回答，因此没有基线读取能与它竞争。
///
/// [clientId] 把后续每一次 [DshClient.respondWaterfall] 绑定到这一代连接；
/// 带已退役 clientId 的回复会被拒。
final class EventReady extends EventFrame {
  const EventReady({required this.clientId, required this.home});
  final String clientId;

  /// 宿主 home 目录（`HostDescription` 只有这一个字段）。
  final String home;
}

/// 宿主通知。断线后**不重放**——需要跨重连存活的状态得靠查询或游标。
final class EventEmit extends EventFrame {
  const EventEmit({required this.event, required this.args});
  final String event;
  final List<Object?> args;
}

/// 一个等待本客户端回答的请求。
final class EventWaterfall extends EventFrame {
  const EventWaterfall({
    required this.event,
    required this.eventId,
    required this.agentId,
    required this.request,
  });

  final String event;
  final String eventId;
  final String agentId;

  /// 事件自身的字段，`agent` 与 `signal` 已被剥掉（agent 提升到
  /// [agentId]，取消生命周期变成后续的 [EventCancel] 帧）。
  final Map<String, Object?> request;
}

/// 先前投递的 waterfall 被撤回：别的客户端先答了，或宿主调用方取消了。
/// UI 必须**不回答**地撤掉这个提示。
final class EventCancel extends EventFrame {
  const EventCancel({required this.eventId});
  final String eventId;
}

/// 未知类型的帧，原样保留。
final class EventUnknown extends EventFrame {
  const EventUnknown(this.raw);
  final Map<String, Object?> raw;
}

/// WebSocket 上的 `$events` 事件流。
class EventPlane {
  const EventPlane({required this.ready, required this.frames});

  /// 开帧到达后才有值。
  final EventReady? ready;

  /// 之后的所有帧（不含开帧）。
  final Stream<EventFrame> frames;
}

/// DSH 的 WebSocket 逻辑流多路复用客户端（`/api/remote.mux`）。
///
/// harness 0.1.2 把两条下行 socket（`/api/events.mux`、`/api/events.host`）
/// 合并成这一条**双向** socket：客户端在其中打开具名逻辑流，宿主逐条
/// 回答 item / error / end。
///
/// 本类**只管一条物理 socket，不重连**。代际（generation）的生命周期与
/// 退避归 [DshConnection]——若这里再来一套重试，就会出现两个组件对
/// 「一代何时结束」各执一词。
class StreamMux {
  StreamMux({
    required this.origin,
    required this.qxToken,
  });

  final String origin;
  final String? qxToken;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  Timer? _pingTimer;

  final _streams = <String, DshStream>{};
  int _nextStreamId = 1;

  final _openController = Completer<void>();
  final _frames = StreamController<EventFrame>.broadcast();
  String? _clientId;

  Object? _closedCause;

  /// 本代连接是否已打开。
  Future<void> get awaitOpen {
    if (_closedCause != null) return Future.error(_closedCause!);
    return _openController.future.then((_) {
      final cause = _closedCause;
      if (cause != null) throw cause;
    });
  }

  /// 宿主分配的 clientId（开帧带来）。
  String? get clientId => _clientId;

  /// 事件帧广播。
  Stream<EventFrame> get frames => _frames.stream;

  bool get isClosed => _closedCause != null;

  /// 关闭原因（若有）。
  Object? get failure => _closedCause;

  /// 建立连接并完成握手。
  ///
  /// 升级被拒（401 缺凭据、403 主机围栏）会以 [SocketException] 到达
  /// 这里——**只有在这里，HTTP 状态码才会保留下来**，之后就被
  /// 「连接失败」这个笼统说法吞掉了。
  Future<void> start() async {
    final uri = Uri.parse(
      '$origin${QxRoute.streamMux}'.replaceFirst('http://', 'ws://'),
    );
    qxlog.info('mux', 'WS 握手 → $uri');
    try {
      final channel = IOWebSocketChannel.connect(
        uri,
        // ⚠️ 必须手工带 cookie：WebSocket 升级走 `Cookie` 头，网关
        // 在这里读它（`gateway.rs:414-421`）。浏览器会自动带，dart:io 不会。
        headers: {
          if (qxToken != null && qxToken!.isNotEmpty) 'Cookie': cookieHeader(qxToken)!,
          'User-Agent': 'QianxunMobile/0.2 (Flutter)',
        },
        pingInterval: const Duration(seconds: 25),
        connectTimeout: const Duration(seconds: 8),
      );
      _channel = channel;
      _sub = channel.stream.listen(
        _onMessage,
        onError: (Object e) {
          qxlog.error('mux', 'WS 出错', e);
          _failAll(e);
        },
        onDone: () {
          qxlog.warn('mux', 'WS 被动关闭');
          _failAll(const SocketException('连接被关闭'));
        },
        cancelOnError: true,
      );
      // ⚠️ **成功时必须把 `_openController` 标记为完成。**
      // 之前只在 `_failAll()` 里 complete，于是握手成功的情形下
      // `awaitOpen` 的 Future 永远不完成 → `DshConnection` 永远等不到
      // 这里 → 永远发不出 `LinkReady` → 界面卡在「正在连接电脑…」转圈。
      // `ready` 在升级完成后才完成，正好对应「socket 可用了」。
      channel.ready.then(
        (_) {
          qxlog.info('mux', 'WS 升级成功（101）');
          if (!_openController.isCompleted) _openController.complete();
        },
        onError: (Object e) {
          qxlog.error('mux', 'WS 升级失败', e);
          if (!_openController.isCompleted) _openController.completeError(e);
          _failAll(e);
        },
      );
      // `pingInterval` 已让 IOWebSocketChannel 自动发 WebSocket 层的
      // ping/pong（网关↔DSH 那一腿有 10 分钟空闲超时，中间设备上的
      // 静默链会被自己掐掉）。不需要在应用层伪造协议帧。
    } catch (e, st) {
      qxlog.error('mux', 'WS 构造失败', e, st);
      _failAll(e);
    }
  }

  /// 打开一条逻辑流并立即发 `open`。
  ///
  /// [endpoint] 是 DSH 的流端点名，如 `session/follow` 或内部的 `$events`。
  /// [args] 的键必须与远端描述符**逐字匹配**。
  DshStream open(String endpoint, {Object? args}) {
    final streamId = '${_nextStreamId++}';
    final stream = DshStream(this, streamId, endpoint);
    // 先注册再发送：两者之间到达的失败要落在这条流上，
    // 而不是落在一个还不包含它的 map 里然后消失。
    _streams[streamId] = stream;
    qxlog.debug('mux', 'open  streamId=$streamId endpoint=$endpoint args=$args');
    final cause = _closedCause;
    if (cause != null) {
      _streams.remove(streamId);
      throw StreamFailed(_carrierError(cause), carrier: true);
    }
    final sent = _send({
      'type': 'open',
      'streamId': streamId,
      'endpoint': endpoint,
      'payload': {'args': _cleanArgs(args)},
    });
    if (!sent) {
      _streams.remove(streamId);
      throw StreamFailed(_carrierError(cause ?? const _MuxClosed()), carrier: true);
    }
    return stream;
  }

  /// 打开 `$events` 并返回一个把开帧与后续帧合并的视图。
  EventPlane openEventPlane() {
    final stream = open(DshEndpoint.events);
    late final StreamController<EventFrame> ctrl;
    EventReady? ready;
    var emittedReady = false;

    ctrl = StreamController<EventFrame>(
      onListen: () {
        unawaited(_pump(stream, (value) {
          final frame = _decodeEventFrame(asMap(value));
          switch (frame) {
            case EventReady r:
              ready = r;
              _clientId = r.clientId;
              if (!emittedReady) {
                emittedReady = true;
                ctrl.add(frame);
              }
            default:
              ctrl.add(frame);
          }
        }, ctrl));
      },
      onCancel: () => stream.cancel(),
    );
    return EventPlane(ready: ready, frames: ctrl.stream);
  }

  Future<void> _pump(
    DshStream stream,
    void Function(Object? value) onItem,
    StreamController<EventFrame> owner,
  ) async {
    try {
      while (true) {
        final signal = await stream.receive();
        if (signal == null) break;
        switch (signal) {
          case StreamItem(:final value):
            onItem(value);
          case StreamEnded():
            return;
          case StreamFailed(:final error):
            owner.addError(error, StackTrace.current);
            return;
        }
      }
    } finally {
      stream.cancel();
      if (!owner.isClosed) await owner.close();
    }
  }

  void _onMessage(dynamic raw) {
    final text = raw is String ? raw : utf8.decode((raw as List<int>));
    // 忽略 WebSocket 协议层的 ping/pong 回声。
    if (text.contains('"ping"') && !text.contains('"streamId"')) return;
    final map = asMap(tryDecodeJson(text));
    final streamId = map['streamId'] as String?;
    if (streamId == null) {
      qxlog.warn('mux', '收到无 streamId 的帧，已忽略：${_clip(text)}');
      return;
    }
    final stream = _streams[streamId];
    // 没人读的流收到 item 是常事：每次界面关闭都会有一个 cancel 与一个
    // 在途 item 在线上交叉。
    if (stream == null) {
      qxlog.debug('mux', '孤儿帧 streamId=$streamId type=${map['type']}（流已关闭）');
      return;
    }
    switch (map['type']) {
      case 'item':
        qxlog.debug('mux', 'item   streamId=$streamId ${_clip(text, 300)}');
        stream._enqueue(StreamItem(map['value']));
        break;
      case 'error':
        qxlog.warn('mux', 'error  streamId=$streamId ${_clip(text, 300)}');
        _streams.remove(streamId);
        stream._enqueue(
          StreamFailed(RpcError.fromJson(asMap(map['error'])), carrier: false),
        );
      case 'end':
        qxlog.debug('mux', 'end    streamId=$streamId');
        _streams.remove(streamId);
        stream._enqueue(const StreamEnded());
      default:
        qxlog.warn('mux', '未知帧类型 ${map['type']}：${_clip(text, 200)}');
        // 载体漂移：无法再解释这条 socket 的状态，整代结束，
        // 而不是丢掉一条消息后继续对一个已经不认识的 mux 说话。
        _failAll(FormatException('未知的远端流帧：${map['type']}'));
    }
  }

  /// 截断长文本，避免一条巨量帧把日志文件冲爆。
  static String _clip(String s, [int max = 160]) =>
      s.length <= max ? s : '${s.substring(0, max)}…(+${s.length - max})';

  /// 关掉 socket 并让所有打开的逻辑流失败。幂等。
  void close() {
    _failAll(null);
    _pingTimer?.cancel();
    _pingTimer = null;
    // 主动取消订阅，否则底层 socket 的事件还会往已关闭的控制器里灌。
    unawaited(_sub?.cancel());
    _sub = null;
    try {
      _channel?.sink.close();
    } catch (_) {
      // 已经死了。
    }
    _channel = null;
  }

  void _removeStream(String streamId) => _streams.remove(streamId);

  bool _send(Map<String, Object?> message) {
    final ch = _channel;
    if (ch == null) return false;
    try {
      ch.sink.add(jsonEncode(message));
      return true;
    } catch (_) {
      return false;
    }
  }

  /// 用一个载体失败结束所有逻辑流。
  ///
  /// 先记录原因再广播：并发打开的流应当看到「已关闭」，而不是注册进来
  /// 后永远等不到完成。
  void _failAll(Object? cause) {
    _closedCause ??= cause ?? const _MuxClosed();
    // 让一个升级失败、根本没打开过的 socket 也能解冻 awaitOpen。
    if (!_openController.isCompleted) _openController.complete();
    final error = _carrierError(_closedCause!);
    final ids = _streams.keys.toList();
    for (final id in ids) {
      _streams.remove(id)?.signalsOnError(error);
    }
    if (!_frames.isClosed) _frames.close();
  }

  RpcError _carrierError(Object cause) => RpcError(
        code: RpcErrorCode.internal,
        message: cause.toString(),
        details: {'status': cause is HandshakeRejected ? cause.status : 0},
      );
}

/// WebSocket 升级被明确拒绝时携带 HTTP 状态。
///
/// Dart 的 `WebSocketChannel.connect` 把 401/403 折叠成 `WebSocketException`，
/// 状态码就丢了——而这是「重试」还是「重新配对」的唯一依据。保留这个类型
/// 让上层能在握手上就分清两者。
class HandshakeRejected implements Exception {
  const HandshakeRejected(this.status);
  final int status;

  @override
  String toString() => 'WebSocket 升级被拒：HTTP $status';
}

/// mux 被**主动**关闭的哨兵（不是失败）。
class _MuxClosed implements Exception {
  const _MuxClosed();
  @override
  String toString() => '远端流 mux 已关闭';
}

extension on DshStream {
  /// 用载体错误结束这条流。
  void signalsOnError(RpcError error) {
    if (_controller.isClosed) return;
    _controller.add(StreamFailed(error, carrier: true));
    _controller.close();
  }
}

/// 解码一个 `$events` 帧。
EventFrame _decodeEventFrame(Map<String, Object?> json) {
  switch (json['type']) {
    case 'ready':
      return EventReady(
        clientId: (json['clientId'] as String?) ?? '',
        home: (asMap(json['host'])['home'] as String?) ?? '',
      );
    case 'emit':
      return EventEmit(
        event: (json['event'] as String?) ?? '',
        args: (json['args'] as List? ?? const []).cast<Object?>(),
      );
    case 'waterfall':
      return EventWaterfall(
        event: (json['event'] as String?) ?? '',
        eventId: (json['eventId'] as String?) ?? '',
        agentId: (json['agentId'] as String?) ?? '',
        request: asMap(json['request']),
      );
    case 'cancel':
      return EventCancel(eventId: (json['eventId'] as String?) ?? '');
    default:
      return EventUnknown(json);
  }
}

Object? _cleanArgs(Object? args) {
  if (args is Map) {
    final out = <String, Object?>{};
    for (final e in args.entries) {
      if (e.value == null) continue;
      out['${e.key}'] = e.value;
    }
    return out;
  }
  return <String, Object?>{};
}

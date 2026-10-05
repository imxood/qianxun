import '../../core/qx_log.dart';
import 'dart:async';
import 'dart:math';
import 'dsh_client.dart';
import 'stream_mux.dart';

/// 一代连接的存活状态。
sealed class LinkState {
  const LinkState();
}

/// 空闲（还没有连过）。
final class LinkIdle extends LinkState {
  const LinkIdle();
}

/// 正在握手。
final class LinkConnecting extends LinkState {
  const LinkConnecting(this.attempt);
  final int attempt;
}

/// 已就绪：`$events` 开帧已到，可以正常用。
final class LinkReady extends LinkState {
  const LinkReady({required this.clientId, required this.home, required this.generation});
  final String clientId;
  final String home;
  final int generation;
}

/// 已断开，正在退避重连。
final class LinkReconnecting extends LinkState {
  const LinkReconnecting({
    required this.attempt,
    required this.nextRetryIn,
    required this.cause,
  });

  final int attempt;
  final Duration nextRetryIn;
  final String cause;
}

/// 鉴权失效——**重试没有意义**，只能重新配对。
final class LinkNeedsPairing extends LinkState {
  const LinkNeedsPairing(this.message);
  final String message;
}

/// 主动关闭。
final class LinkClosed extends LinkState {
  const LinkClosed();
}

/// DSH 连接管理器。
///
/// 拥有**代际（generation）**的生命周期：一次握手成功 = 一代。宿主或网关
/// 重建会掐断所有连接（`gateway.rs:216-223`：「网关停止：经由网关的全部
/// 连接（HTTP/SSE/WS）随即断开」），那不是错误，是常态——DSH 恢复、
/// 桌面端改设置、配对、吊销令牌都会触发。所以重连必须是默认行为，
/// 而不是异常处理。
class DshConnection {
  DshConnection({
    required this.origin,
    required this.qxToken,
    this.maxBackoff = const Duration(seconds: 30),
  });

  final String origin;
  final String qxToken;
  final Duration maxBackoff;

  final _state = StreamController<LinkState>.broadcast();
  final _events = StreamController<EventFrame>.broadcast();

  StreamMux? _mux;
  DshClient? _client;
  Timer? _retryTimer;
  int _attempt = 0;
  int _generation = 0;
  bool _stopped = true;
  String? _clientId;
  final _random = Random();

  LinkState _current = const LinkIdle();

  /// 状态流。
  Stream<LinkState> get states => _state.stream;

  /// 当前状态。
  LinkState get state => _current;

  /// `$events` 事件帧（不含开帧）。
  Stream<EventFrame> get events => _events.stream;

  /// 本代 clientId。用于回应 waterfall。
  String? get clientId => _clientId;

  /// 本代的一元 RPC 客户端。未就绪时为 null。
  DshClient? get client => _client;

  /// 本代的 mux，用于打开额外的逻辑流（`session/follow` 等）。
  StreamMux? get currentMux => _mux;

  bool get isReady => _current is LinkReady;

  void _emit(LinkState next) {
    _current = next;
    if (!_state.isClosed) _state.add(next);
  }

  /// 开始连接，并持续维持。
  void start() {
    if (!_stopped) return;
    _stopped = false;
    _attempt = 0;
    unawaited(_connectOnce());
  }

  /// 主动断开，不再重连。
  void stop() {
    _stopped = true;
    _retryTimer?.cancel();
    _retryTimer = null;
    _teardown();
    _emit(const LinkClosed());
  }

  /// 立刻重试一次（用户手动点「重连」）。
  void retryNow() {
    if (_stopped) return;
    _retryTimer?.cancel();
    _attempt = 0;
    unawaited(_connectOnce());
  }

  void dispose() {
    stop();
    _state.close();
    _events.close();
  }

  Future<void> _connectOnce() async {
    if (_stopped) return;
    qxlog.info('link', '发起连接 #$_generation  origin=$origin token=${_tokenHint()}');
    _emit(LinkConnecting(_attempt));

    final mux = StreamMux(origin: origin, qxToken: qxToken);
    _mux = mux;
    // 事件转发：把本代 mux 的帧接到共享广播上。
    mux.frames.listen((frame) {
      if (_mux == mux && !_events.isClosed) _events.add(frame);
    });

    try {
      await mux.start();
      qxlog.info('link', 'socket 已建立，等待 \$events 开帧…');
      await mux.awaitOpen;
      qxlog.info('link', 'socket 就绪');
    } catch (e, st) {
      qxlog.error('link', '握手失败', e, st);
      _handleFailure(e, mux);
      return;
    }

    if (_stopped || _mux != mux) {
      qxlog.warn('link', '握手期间已停止/换代，丢弃本代');
      mux.close();
      return;
    }

    // `$events` 必须**无条件**打开：连接健康度不依赖任何界面是否在监听。
    // 它的开帧才是让这一代「就绪」的东西。
    try {
      final plane = mux.openEventPlane();
      qxlog.info('link', '已发送 \$events open，等待 ready 帧…');
      plane.frames.listen(
        (frame) {
          if (frame is EventReady) {
            _clientId = frame.clientId;
            _attempt = 0;
            _generation++;
            _client = DshClient(origin: origin, qxToken: qxToken);
            qxlog.info(
              'link',
              '★ 就绪 #$_generation  clientId=${frame.clientId}  home=${frame.home}',
            );
            _emit(LinkReady(
              clientId: frame.clientId,
              home: frame.home,
              generation: _generation,
            ));
          } else if (_mux == mux && !_events.isClosed) {
            qxlog.debug('link', 'frame ${frame.runtimeType}');
            _events.add(frame);
          }
        },
        onError: (Object e) {
          qxlog.error('link', '\$events 流错误', e);
          if (_mux == mux) _handleFailure(e, mux);
        },
        onDone: () {
          qxlog.warn('link', '\$events 流结束');
        },
      );
    } catch (e, st) {
      qxlog.error('link', '打开 \$events 失败', e, st);
      _handleFailure(e, mux);
    }
  }

  /// 日志里只打 token 的头尾，够定位「用错了哪个凭据」又不会泄漏完整凭据。
  String _tokenHint() {
    if (qxToken.isEmpty) return '(空)';
    if (qxToken.length <= 8) return qxToken;
    return '${qxToken.substring(0, 4)}…${qxToken.substring(qxToken.length - 4)}';
  }

  void _handleFailure(Object cause, StreamMux mux) {
    if (_mux != mux) return;
    _teardown();

    // 凭据被拒时**重试没有意义**——重试一万次也是同样结果。必须转成
    // 「需要重新配对」让用户去扫新的码，而不是让界面无限转圈。
    //
    // ⚠️ `IOWebSocketChannel.connect` 把 401/403 折叠成
    // `WebSocketException`，状态码不在异常类型里，只能从消息里认。
    if (_looksUnauthorized(cause)) {
      qxlog.error('link', '凭据被拒（401/403）→ 需要重新配对', cause);
      _emit(const LinkNeedsPairing('凭据已失效，请重新扫码配对。'));
      return;
    }
    if (_stopped) return;
    qxlog.warn('link', '连接中断，准备退避重连：$cause');
    _scheduleRetry(cause.toString());
  }

  /// 从异常里认出鉴权失败。
  ///
  /// `WebSocketException` 的文案在不同 platform 上不一致（Android 的
  /// `dart:io` 会带上状态码文本，握手被拒时是 401/403），所以两种都认。
  bool _looksUnauthorized(Object cause) {
    if (cause is HandshakeRejected) {
      return cause.status == 401 || cause.status == 403;
    }
    final text = cause.toString();
    return text.contains('401') ||
        text.contains('403') ||
        text.contains('Unauthorized') ||
        text.contains('Forbidden');
  }

  /// 指数退避 + 抖动，上限 [maxBackoff]。
  ///
  /// 抖动是必要的：网关重建后所有客户端会同时掉线，没有抖动就会在同一
  /// 毫秒一起重连，把刚起来的 DSH 再撞一次。
  void _scheduleRetry(String cause) {
    _attempt++;
    final exp = min(_attempt, 6);
    final base = (1000 * pow(2, exp - 1)).clamp(500, maxBackoff.inMilliseconds.toDouble());
    final jitter = _random.nextDouble() * base * 0.3;
    final delay = Duration(milliseconds: (base + jitter).round());

    _emit(LinkReconnecting(
      attempt: _attempt,
      nextRetryIn: delay,
      cause: _humanize(cause),
    ));
    _retryTimer?.cancel();
    _retryTimer = Timer(delay, () {
      if (!_stopped) unawaited(_connectOnce());
    });
  }

  void _teardown() {
    _retryTimer?.cancel();
    _retryTimer = null;
    _clientId = null;
    _client = null;
    final mux = _mux;
    _mux = null;
    mux?.close();
  }

  /// 把底层异常翻译成用户能懂的一句话。
  String _humanize(String raw) {
    if (raw.contains('Failed host lookup')) return '找不到这台电脑（域名解析失败）。';
    if (raw.contains('Connection refused')) return '连接被拒绝：网关可能没启动。';
    if (raw.contains('Connection timed out') || raw.contains('timed out')) {
      return '连接超时：可能不在同一个网络里。';
    }
    if (raw.contains('Network is unreachable')) return '网络不可达。';
    if (raw.contains('No route to host')) return '找不到路由：请确认 EasyTier 已连通。';
    return '连接中断，正在重试。';
  }
}

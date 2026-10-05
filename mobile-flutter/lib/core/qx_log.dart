import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';

/// 日志级别。数值越大越严重。
enum QxLevel {
  trace(0, 'TRACE'),
  debug(10, 'DEBUG'),
  info(20, 'INFO'),
  warn(30, 'WARN'),
  error(40, 'ERROR');

  const QxLevel(this.severity, this.label);
  final int severity;
  final String label;

  static QxLevel parse(String raw) => QxLevel.values.firstWhere(
        (l) => l.label == raw.toUpperCase(),
        orElse: () => QxLevel.info,
      );
}

/// 一条日志。
@immutable
class QxLogEntry {
  const QxLogEntry({
    required this.at,
    required this.level,
    required this.tag,
    required this.message,
    this.error,
  });

  final DateTime at;
  final QxLevel level;
  final String tag;
  final String message;
  final String? error;

  /// 单行文本形式，用于文件导出与复制。
  String format({bool withDate = true}) {
    final t = withDate
        ? '${at.hour.toString().padLeft(2, '0')}:'
            '${at.minute.toString().padLeft(2, '0')}:'
            '${at.second.toString().padLeft(2, '0')}.'
            '${at.millisecond.toString().padLeft(3, '0')}'
        : '--:--:--.---';
    final buf = StringBuffer()
      ..write('[$t] ')
      ..write('${level.label.padRight(5)} ')
      ..write('${tag.padRight(14)} ')
      ..write(message);
    if (error != null && error!.isNotEmpty) {
      buf.write('\n           ↳ $error');
    }
    return buf.toString();
  }

  Map<String, Object?> toJson() => {
        't': at.millisecondsSinceEpoch,
        'l': level.label,
        'tag': tag,
        'm': message,
        if (error != null) 'e': error,
      };
}

/// 全局日志。
///
/// 为什么需要它：这个客户端出问题的时候，症状是「转圈」或「离线」——
/// **屏幕上没有任何信息能说明卡在哪一步**。没有日志就只能靠猜，
/// 而传输层的问题（cookie、WS 升级、mux 帧、超时）恰恰是「猜不出来」
/// 那一类。所以日志是这个 App 的**诊断基础设施**，不是可选的装饰。
///
/// 行为：
/// - 内存里保留最近 [_capacity] 条，供日志页实时查看。
/// - 同时**追加**写入磁盘文件（环形覆盖），App 崩溃/被杀也能留下现场。
/// - `share()` 导出为临时 txt，走系统分享。
class QxLog {
  QxLog._();

  static final QxLog instance = QxLog._();

  /// 内存与磁盘各保留多少条。单条约 200 字节，2000 条约 400KB。
  static const int _capacity = 2000;

  static const String _fileName = 'qianxun-log.txt';

  final _entries = <QxLogEntry>[];
  final _controller = StreamController<QxLogEntry>.broadcast();

  File? _file;
  bool _fileInitTried = false;

  /// 低于这个级别的不记。默认 info —— 生产包不该被 trace 淹没。
  QxLevel minLevel = QxLevel.info;

  /// 变更流，供日志页订阅。
  Stream<QxLogEntry> get stream => _controller.stream;

  /// 当前内存中的日志（按时间正序）。
  List<QxLogEntry> get entries => List.unmodifiable(_entries);

  int get count => _entries.length;

  /// 初始化。必须在 `runApp` 之前调用。
  ///
  /// 磁盘初始化失败**不阻塞启动**——拿不到日志文件只是少一个诊断渠道，
  /// 不该让整个 App 起不来。
  Future<void> init() async {
    if (_fileInitTried) return;
    _fileInitTried = true;
    try {
      final dir = await getApplicationDocumentsDirectory();
      _file = File('${dir.path}${Platform.pathSeparator}$_fileName');
      if (!await _file!.exists()) {
        await _file!.create();
      }
      await _loadFromDisk();
    } catch (e) {
      debugPrint('[QxLog] 磁盘日志不可用：$e');
      _file = null;
    }
  }

  Future<void> _loadFromDisk() async {
    final f = _file;
    if (f == null || !await f.exists()) return;
    try {
      final lines = await f.readAsLines();
      for (final line in lines) {
        final e = _parseLine(line);
        if (e != null) _entries.add(e);
      }
      _trim();
    } catch (e) {
      debugPrint('[QxLog] 读取历史日志失败：$e');
    }
  }

  /// 反解析一行我们自己写出去的日志。
  ///
  /// 格式是定死的（见 [QxLogEntry.format]），所以按位置切比引入
  /// JSON 依赖更省事，也让用户在文件里肉眼看也读得懂。
  static QxLogEntry? _parseLine(String line) {
    if (!line.startsWith('[') || line.length < 25) return null;
    final close = line.indexOf(']');
    if (close < 0) return null;
    final time = line.substring(1, close);
    final rest = line.substring(close + 1).trimLeft();
    final parts = rest.split(RegExp(r'\s+'));
    if (parts.length < 3) return null;
    final lvl = QxLevel.parse(parts[0]);
    final tag = parts[1];
    final msg = parts.sublist(2).join(' ');
    final hm = RegExp(r'^(\d{2}):(\d{2}):(\d{2})\.(\d{3})$').firstMatch(time);
    final now = DateTime.now();
    final at = hm == null
        ? now
        : DateTime(
            now.year,
            now.month,
            now.day,
            int.parse(hm.group(1)!),
            int.parse(hm.group(2)!),
            int.parse(hm.group(3)!),
            int.parse(hm.group(4)!),
          );
    return QxLogEntry(at: at, level: lvl, tag: tag, message: msg);
  }

  void _trim() {
    if (_entries.length > _capacity) {
      _entries.removeRange(0, _entries.length - _capacity);
    }
  }

  void _add(QxLogEntry entry) {
    if (entry.level.severity < minLevel.severity) return;
    _entries.add(entry);
    _trim();
    if (!_controller.isClosed) _controller.add(entry);
    // 磁盘写失败不影响主流程。
    unawaited(_appendToDisk(entry));
  }

  Future<void> _appendToDisk(QxLogEntry entry) async {
    final f = _file;
    if (f == null) return;
    try {
      await f.writeAsString('${entry.format()}\n', mode: FileMode.append, flush: true);
      // 超过容量就整体重写一次，避免文件无限增长。
      if (await f.length() > 2 * 1024 * 1024) {
        final keep = _entries.map((e) => e.format()).join('\n');
        await f.writeAsString('$keep\n', flush: true);
      }
    } catch (e) {
      debugPrint('[QxLog] 写盘失败：$e');
    }
  }

  void trace(String tag, String m) => _add(_e(QxLevel.trace, tag, m));
  void debug(String tag, String m) => _add(_e(QxLevel.debug, tag, m));
  void info(String tag, String m) => _add(_e(QxLevel.info, tag, m));
  void warn(String tag, String m) => _add(_e(QxLevel.warn, tag, m));

  void error(String tag, String m, [Object? err, StackTrace? st]) => _add(
        QxLogEntry(
          at: DateTime.now(),
          level: QxLevel.error,
          tag: tag,
          message: m,
          error: err == null ? null : '$err${st == null ? '' : '\n$st'}',
        ),
      );

  QxLogEntry _e(QxLevel l, String tag, String m) => QxLogEntry(
        at: DateTime.now(),
        level: l,
        tag: tag,
        message: m,
      );

  /// 清空内存与磁盘。
  Future<void> clear() async {
    _entries.clear();
    try {
      await _file?.writeAsString('', flush: true);
    } catch (_) {
      // 清不掉下次再说，不该因此报错。
    }
  }

  /// 导出内容（带文件头）。[maxEntries] 为 null 时取全部。
  ///
  /// 分享时**正文和文件都要给**：只给文件的话，接到「分享到聊天/笔记」
  /// 这类只收文本的目标时对方拿到的就是空的；只给正文则超出 Binder
  /// 事务上限会直接崩。两边都给，文本那份再按 [maxEntries] 截断。
  String render({int? maxEntries}) {
    final rows = maxEntries == null || _entries.length <= maxEntries
        ? _entries
        : _entries.sublist(_entries.length - maxEntries);
    final dropped = _entries.length - rows.length;
    final buf = StringBuffer()
      ..writeln('千寻移动端 · 运行日志')
      ..writeln('导出时间 : ${DateTime.now().toIso8601String()}')
      ..writeln('平台     : ${Platform.operatingSystem} ${Platform.operatingSystemVersion}')
      ..writeln('条目数   : ${_entries.length}${dropped > 0 ? '（此处只含最近 $dropped→$rows.length 条）' : ''}')
      ..writeln('=' * 72);
    for (final e in rows) {
      buf.writeln(e.format());
    }
    return buf.toString();
  }

  /// 导出成临时 txt 文件。
  ///
  /// 文件名带时间戳，方便多次导出时区分。文件放在系统临时目录，分享完
  /// 由系统自行回收——日志不该在用户手机上留一堆副本。
  Future<File> exportTxt() async {
    final stamp = DateTime.now().toIso8601String().replaceAll(':', '-').split('.').first;
    final file = File('${await _writableDir()}qianxun-log-$stamp.txt');
    return file.writeAsString(render(), flush: true);
  }

  /// 找一个能写的目录。
  ///
  /// 优先 `getTemporaryDirectory()`（系统会自己回收），失败时退到
  /// `Directory.systemTemp`。日志导出**永远不该因为拿不到目录而失败**——
  /// 那正是最需要日志的时候。
  Future<String> _writableDir() async {
    try {
      return '${(await getTemporaryDirectory()).path}${Platform.pathSeparator}';
    } catch (_) {
      return '${Directory.systemTemp.path}${Platform.pathSeparator}';
    }
  }

  /// 写一段到磁盘的「最后手段」日志。
  ///
  /// 专门给**框架级**异常用（`FlutterError.onError` / 平台 dispatcher）：
  /// 那时 UI 可能已经不可用，但文件还在。
  void recordFatal(Object error, StackTrace? stack, {String source = 'flutter'}) {
    _add(
      QxLogEntry(
        at: DateTime.now(),
        level: QxLevel.error,
        tag: 'FATAL/$source',
        message: '未捕获异常',
        error: '$error\n$stack',
      ),
    );
  }
}

/// 便捷入口。
QxLog get qxlog => QxLog.instance;

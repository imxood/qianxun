import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/app/theme/qx_theme.dart';
import 'package:qianxun_mobile/core/qx_log.dart';
import 'package:qianxun_mobile/features/logs/presentation/logs_screen.dart';

void main() {
  setUp(() async {
    await QxLog.instance.clear();
    QxLog.instance.minLevel = QxLevel.trace;
  });

  tearDown(() async {
    await QxLog.instance.clear();
  });

  group('日志记录', () {
    test('按级别记录，低于阈值的丢弃', () {
      final log = QxLog.instance;
      log.minLevel = QxLevel.info;
      log.debug('t', '这条不该被记下来');
      log.info('t', '这条应该留下');
      log.error('t', '出错', StateError('boom'));

      expect(log.entries.length, 2);
      expect(log.entries.every((e) => e.level.severity >= QxLevel.info.severity), isTrue);
      // 异常对象要进 error 槽，否则排查时看不到真正的堆栈。
      expect(log.entries.last.error, contains('boom'));
    });

    test('单行格式可以被自己解析回来（导出→重读不丢信息）', () {
      final log = QxLog.instance;
      log.minLevel = QxLevel.trace;
      log.info('mux', 'WS 升级成功（101）');

      final line = log.entries.single.format();
      // 格式是定死的：[HH:MM:SS.mmm] LEVEL tag  message
      expect(line, matches(RegExp(r'^\[\d{2}:\d{2}:\d{2}\.\d{3}\] ')));
      expect(line, contains('INFO'));
      expect(line, contains('mux'));
      expect(line, contains('WS 升级成功'));
    });

    test('容量上限生效，不会无限增长', () {
      final log = QxLog.instance;
      log.minLevel = QxLevel.trace;
      for (var i = 0; i < 2500; i++) {
        log.trace('bulk', 'line $i');
      }
      expect(log.count, lessThanOrEqualTo(2000));
      // 丢的是**最旧**的，不是随机的。
      expect(log.entries.first.message, isNot('line 0'));
    });

    testWidgets('日志页在有内容时渲染，并显示计数', (tester) async {
      QxLog.instance
        ..minLevel = QxLevel.trace
        ..info('link', '发起连接 #0')
        ..warn('probe', '超时 6s');
      await tester.pumpWidget(
        MaterialApp(theme: QxTheme.dark(), home: const LogsScreen()),
      );
      await tester.pump();
      expect(find.text('运行日志'), findsOneWidget);
      expect(find.textContaining('2 条'), findsOneWidget);
      expect(find.textContaining('发起连接'), findsOneWidget);
      expect(find.textContaining('超时'), findsOneWidget);
    });

    testWidgets('级别筛选只留下达标条目', (tester) async {
      QxLog.instance
        ..minLevel = QxLevel.trace
        ..debug('a', 'debug 行')
        ..error('b', 'error 行');
      await tester.pumpWidget(
        MaterialApp(theme: QxTheme.dark(), home: const LogsScreen()),
      );
      await tester.pump();
      expect(find.textContaining('debug 行'), findsOneWidget);

      // 点 WARN 之后 debug 应被滤掉，error 留下。
      await tester.tap(find.text('WARN'));
      await tester.pump();
      expect(find.textContaining('debug 行'), findsNothing);
      expect(find.textContaining('error 行'), findsOneWidget);
    });

    testWidgets('清空后页面回到空态', (tester) async {
      QxLog.instance.minLevel = QxLevel.trace;
      QxLog.instance.info('a', 'x');
      await tester.pumpWidget(
        MaterialApp(theme: QxTheme.dark(), home: const LogsScreen()),
      );
      await tester.pump();
      expect(find.textContaining('x'), findsOneWidget);

      await QxLog.instance.clear();
      await tester.pump();
      expect(QxLog.instance.count, 0);
    });

    test('导出文件带文件头，可直接读懂', () async {
      final log = QxLog.instance;
      log.minLevel = QxLevel.trace;
      log.info('link', '★ 就绪 #1');
      final file = await log.exportTxt();
      addTearDown(() {
        if (file.existsSync()) file.deleteSync();
      });

      expect(file.existsSync(), isTrue);
      final text = await file.readAsString();
      expect(text, contains('千寻移动端 · 运行日志'));
      expect(text, contains('导出时间'));
      expect(text, contains('★ 就绪 #1'));
    });

    test('异常等级颜色可解析，未知等级回落到 info', () {
      expect(QxLevel.parse('ERROR'), QxLevel.error);
      expect(QxLevel.parse('WARN'), QxLevel.warn);
      expect(QxLevel.parse('nonsense'), QxLevel.info);
    });
  });
}

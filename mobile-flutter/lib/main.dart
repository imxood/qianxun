import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'app/app.dart';
import 'core/qx_log.dart';

Future<void> main() async {
  // 绑定必须在任何日志调用之前。
  WidgetsFlutterBinding.ensureInitialized();

  // 框架级异常兜底：UI 已经不可用时，写盘日志是唯一的现场。
  // 必须装在 runApp 之前，否则早期异常接不住。
  FlutterError.onError = (details) {
    FlutterError.presentError(details);
    qxlog.recordFatal(details.exception, details.stack, source: 'FlutterError');
  };
  PlatformDispatcher.instance.onError = (error, stack) {
    qxlog.recordFatal(error, stack, source: 'PlatformDispatcher');
    return true; // 交给上面的通道，不让平台默认处理（会杀进程丢现场）。
  };

  await qxlog.init();
  qxlog.info('app', '启动 v0.2.0  ${Platform.operatingSystemVersion}');

  runApp(const ProviderScope(child: QxApp()));
}

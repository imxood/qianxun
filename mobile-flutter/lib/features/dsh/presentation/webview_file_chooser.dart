import 'package:file_picker/file_picker.dart';
import 'package:flutter/services.dart' show PlatformException;

import '../../../core/qx_log.dart';

/// WebView 文件选择桥：把页面的 `<input type="file">` 请求接到原生选择器。
///
/// ## 为什么必须有它
///
/// DSH 的「+ -> 文件」是页面里的**原生 file input**（源码
/// `dsh-client-ui-conversation/lib/client.js` 的
/// `installDraftFilePicker` -> `fileInputRef.current?.click()`）。
/// Android WebView 上，file input 的弹框**唯一**入口是
/// `WebChromeClient.onShowFileChooser`——不接它，点了「文件」**什么都不发生**
/// （不报错、不弹框），插件默认返回空列表。
///
/// ## 契约
///
/// `onShowFileChooser` 的返回值是**文件 URI 列表**（`content://` 或
/// `file://`），WebView 据此把文件内容喂给页面的 `File` 对象。
/// 取消选择时返回**空列表**（不是 null，也不是抛错）。
class WebViewFileChooser {
  const WebViewFileChooser._();

  /// 处理一次文件选择请求。
  ///
  /// 返回选中的文件 URI 列表；用户取消或出错时返回空列表
  /// （WebView 侧表现为「没有选文件」，页面自行处理）。
  static Future<List<String>> pick({
    required bool allowMultiple,
    List<String> acceptTypes = const [],
  }) async {
    try {
      final result = await FilePicker.pickFiles(
        allowMultiple: allowMultiple,
        type: _fileTypeFor(acceptTypes),
      );
      if (result == null) {
        qxlog.info('file', '用户取消文件选择');
        return const [];
      }
      final uris = <String>[];
      for (final file in result.files) {
        final uri = _uriOf(file);
        if (uri != null) {
          uris.add(uri);
        } else {
          qxlog.warn('file', '文件「${file.name}」拿不到可用 URI，跳过');
        }
      }
      qxlog.info('file', '已选 ${uris.length} 个文件');
      return uris;
    } on PlatformException catch (error) {
      qxlog.warn('file', '文件选择失败：${error.message}');
      return const [];
    } catch (error) {
      qxlog.warn('file', '文件选择异常：$error');
      return const [];
    }
  }

  /// 把 acceptTypes 映射到 file_picker 的类型。
  ///
  /// DSH 的 file input 通常不带 accept（`acceptTypes` 为空），
  /// 此时用 `FileType.any`——**不要**因为「只想要图片」就收紧：
  /// 那个按钮的语义是「文件」，图片是文件的子集。
  static FileType _fileTypeFor(List<String> acceptTypes) {
    if (acceptTypes.isEmpty) return FileType.any;
    final allImages = acceptTypes.every((type) => type.startsWith('image/'));
    return allImages ? FileType.image : FileType.any;
  }

  /// 取文件 URI。
  ///
  /// 优先用 `path` 转 `file://` URI——WebView 能读本地文件；
  /// Android 上 `content://` 需要权限授予，file:// 在同进程 WebView 里
  /// 可直接读（WebView 与 App 同 UID）。
  static String? _uriOf(PlatformFile file) {
    final path = file.path;
    if (path == null || path.isEmpty) return null;
    if (path.startsWith('content://') || path.startsWith('file://')) {
      return path;
    }
    return Uri.file(path).toString();
  }
}

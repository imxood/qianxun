/// 千寻网关层（Rust/axum）的地址与常量。
///
/// 这些值全部来自 `src-tauri/src/remote/gateway.rs`，改动前先对齐桌面端。
library;

/// 网关默认端口。
///
/// ⚠️ **不要写死 17400**。那是 `LEGACY_GATEWAY_PORT`，仅为迁移旧
/// `settings.json` 而保留。真实默认值按构建模式走 `RemoteSettings.port`：
/// debug 23091 / release 23090，且用户可改。
///
/// 客户端的正确做法是**永远从配对链接里读端口**，见 [PairUrl]。
abstract final class QxPort {
  static const int debug = 23091;
  static const int release = 23090;

  /// 出现在旧文档/旧客户端注释里的历史端口，仅用于识别并提示迁移。
  static const int legacy = 17400;
}

/// 配对 cookie 名。网关兑换成功后写入，客户端需自行保存并附带。
abstract final class QxCookie {
  static const String token = 'qx_token';

  /// 网关向上游 DSH 换 cookie 时使用的 cookie 前缀。客户端**看不到**这个值
  /// ——它只存在于网关进程内，客户端只拿 `qx_token`。
  static const String dshPrefix = 'dsh-auth-';
}

/// 网关自有的路由。只有这几条是网关实现的，其余全部透传给 DSH。
abstract final class QxRoute {
  /// 配对兑换入口。`?code=`（一次性，5 分钟）新格式，`?token=` 旧格式。
  static const String gate = '/qx-gate';

  /// 电脑端身份。唯一带 `Access-Control-Allow-Origin: *` 的路由。
  static const String info = '/qx-mobile/info';

  /// 定制层版本号（哈希），供 custom.js 热替换。
  static const String version = '/qx-mobile/version';

  /// DSH 唯一的双向 WebSocket 流多路复用入口。
  static const String streamMux = '/api/remote.mux';
}

/// DSH 一元 RPC 的路径模板：方法名 `ns/method` → `POST /api/ns/method`。
String qxApiPath(String method) => '/api/$method';

/// 附件上传路由。
///
/// ⚠️ **这是全 App 唯一一条不走 `{"args":{…}}` 信封的路由**，
/// 也不是 `client-request` 信封。用 `DshClient.call()` 打它会拿到
/// `415 Unsupported Media Type`。
///
/// 实测（`scripts/probe_upload.py`）：
///
/// ```http
/// POST /api/session/uploadFileBinary?sessionId=<sid>&name=<文件名>
/// Content-Type: application/octet-stream
///
/// <裸二进制 body>
/// ```
///
/// ```json
/// {"ok":true,"value":{"receiptId":"<uuid>",
///   "file":{"attachmentId":"sha256:<hex>","name":"…","bytes":70}}}
/// ```
///
/// 三个必须记住的点：
/// 1. `sessionId` 走 **query**，不是 body、不是 header
/// 2. `name` 走 query；不传时服务端默认叫 `file`
/// 3. 送进消息的是 **`receiptId`**（一次性消费凭证），
///    不是 `attachmentId`（持久标识，用于 `session/attachment` 读回）
String qxUploadPath(String sessionId, String? name) {
  final q = <String, String>{'sessionId': sessionId};
  if (name != null && name.isNotEmpty) q['name'] = name;
  return '/api/session/uploadFileBinary?${Uri(queryParameters: q).query}';
}

/// 事件流与事件应答的内部端点名。
///
/// 这两个不是 DSH 的公开 API，而是 gateway 内部逻辑流的名字（以 `$` 开头）。
abstract final class DshEndpoint {
  /// 连接存活 + 通知 + 主机请求（waterfall）。**无条件打开**，
  /// 连接健康度不依赖任何界面是否在监听。
  static const String events = r'$events';

  /// 回应 [DshEndpoint.events] 上某个 waterfall 的一元调用。
  static const String eventsResult = r'$events/result';

  /// 会话实时流。参数 `{ "sessionId": "..." }`。
  static const String sessionFollow = 'session/follow';

  /// 会话控制流（队列 / 后台 job / 投影）。参数 `{ "sessionId": "..." }`。
  static const String sessionControl = 'session/control';

  /// 工作区注册表流。
  static const String workspaceFollow = 'workspace/follow';
}

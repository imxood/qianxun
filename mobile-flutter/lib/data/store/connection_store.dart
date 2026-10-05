import 'package:shared_preferences/shared_preferences.dart';

import '../dsh/pair_url.dart';
import 'dart:convert';

/// 一条已保存的电脑连接。
///
/// 存储键沿用 RN 客户端的 `qx.connections.v1`，schema 一致——不是为了兼容
/// 旧客户端（两个 App 不共享沙箱），而是为了让 schema 本身有明确出处、
/// 便于人工核对。
class Connection {
  const Connection({
    required this.id,
    required this.name,
    required this.url,
    required this.addedAt,
    this.lastUsedAt = 0,
    this.qxToken,
    this.hostname,
    this.appVersion,
  });

  final String id;
  final String name;

  /// 配对链接（归一化后）。
  final String url;
  final int addedAt;
  final int lastUsedAt;

  /// 兑换得到的长期 `qx_token`。
  ///
  /// 存下来是因为 `?code=` 是一次性的：兑换一次之后，原始链接就废了。
  /// 不存就等于每次冷启动都要重新扫码。
  final String? qxToken;

  /// 最近一次探测拿到的电脑名。
  final String? hostname;
  final String? appVersion;

  factory Connection.fromJson(Map<String, Object?> json) => Connection(
        id: (json['id'] as String?) ?? '',
        name: (json['name'] as String?) ?? '未命名',
        url: (json['url'] as String?) ?? '',
        addedAt: (json['addedAt'] as num?)?.toInt() ?? 0,
        lastUsedAt: (json['lastUsedAt'] as num?)?.toInt() ?? 0,
        qxToken: json['qxToken'] as String?,
        hostname: json['hostname'] as String?,
        appVersion: json['appVersion'] as String?,
      );

  Map<String, Object?> toJson() => {
        'id': id,
        'name': name,
        'url': url,
        'addedAt': addedAt,
        'lastUsedAt': lastUsedAt,
        'qxToken': qxToken,
        'hostname': hostname,
        'appVersion': appVersion,
      };

  Connection copyWith({
    String? name,
    String? url,
    int? lastUsedAt,
    String? qxToken,
    String? hostname,
    String? appVersion,
    bool clearToken = false,
  }) =>
      Connection(
        id: id,
        name: name ?? this.name,
        url: url ?? this.url,
        addedAt: addedAt,
        lastUsedAt: lastUsedAt ?? this.lastUsedAt,
        qxToken: clearToken ? null : (qxToken ?? this.qxToken),
        hostname: hostname ?? this.hostname,
        appVersion: appVersion ?? this.appVersion,
      );

  /// 配对链接解析结果。存的是归一化形式，解析应当总是成功。
  PairUrl? get pair => PairUrl.tryParse(url);

  /// 鉴权用 token：优先存下来的兑换结果。
  String? get effectiveToken => qxToken ?? pair?.token;

  /// 展示用主机。
  String get displayHost => pair?.displayHost ?? url;

  /// 列表排序用：最近用过优先，其次最近添加。
  static int compareFresh(Connection a, Connection b) {
    final byUsed = (b.lastUsedAt).compareTo(a.lastUsedAt);
    if (byUsed != 0) return byUsed;
    return b.addedAt.compareTo(a.addedAt);
  }
}

/// 一次操作的结果（成功带提示语，失败带可展示的原因）。
class OpResult {
  const OpResult.ok(this.message) : error = null;
  const OpResult.fail(this.error) : message = null;

  final String? message;
  final String? error;

  bool get isOk => error == null;
}

/// 连接仓库。持久化到 `SharedPreferences`。
class ConnectionStore {
  static const String _key = 'qx.connections.v1';

  Future<List<Connection>> load() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final raw = prefs.getString(_key);
      if (raw == null || raw.isEmpty) return const [];
      final decoded = jsonDecode(raw);
      if (decoded is! List) return const [];
      return decoded
          .whereType<Map>()
          .map((e) => Connection.fromJson(e.cast<String, Object?>()))
          .where((c) => c.id.isNotEmpty && c.url.isNotEmpty)
          .toList();
    } catch (_) {
      // 存储损坏不该让 App 起不来。
      return const [];
    }
  }

  Future<void> save(List<Connection> connections) async {
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString(
        _key,
        jsonEncode(connections.map((c) => c.toJson()).toList()),
      );
    } catch (_) {
      // 存不进去也不能让界面崩。
    }
  }

  /// 生成一个新 id。
  static String newId() => 'c${DateTime.now().millisecondsSinceEpoch.toRadixString(36)}';
}

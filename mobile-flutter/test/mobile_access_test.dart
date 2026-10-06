import 'package:flutter_test/flutter_test.dart';
import 'package:qianxun_mobile/data/dsh/mobile_access.dart';

void main() {
  int now() => DateTime.now().millisecondsSinceEpoch;

  group('MobileAccessTarget', () {
    test('persistKey 只含协议+主机+端口，**不含路径与查询**', () {
      // 配对链接带一次性凭据（?token= / ?code=）。持久化路径等于把凭据存下来，
      // 依据 dsh-mobile README.zh-CN.md:43。
      final target = MobileAccessTarget(
        origin: Uri.parse('https://192.168.20.2:3443/mobile-access/pair#token=secret'),
        caPem: 'x',
      );
      expect(target.persistKey, 'https://192.168.20.2:3443');
      expect(target.persistKey.contains('secret'), isFalse);
      expect(target.persistKey.contains('/mobile-access'), isFalse);
    });

    test('pageUrl 指向站点根，**不是** API 路径', () {
      final target = MobileAccessTarget(
        origin: Uri.parse('https://192.168.20.2:3443'),
        caPem: 'x',
      );
      // 首屏必须落在站点根，因为 /mobile-access/pair 那些页在根下
      expect(target.pageUrl.path, isEmpty);
    });

    test('setup.json 实测无 origin 字段，要用网卡地址 + listenPort 拼', () {
      // 依据：probe_l3_desktop.py 实测 setup.json 的键是
      // {version, networkInterface, listenPort, upstreamOrigin, tls}
      final setup = <String, dynamic>{
        'version': 2,
        'networkInterface': 'et_3_2avf',
        'listenPort': 3443,
        'upstreamOrigin': 'http://127.0.0.1:3080',
      };
      final target = MobileAccessTarget.fromSetup(
        setup,
        lanAddress: Uri.parse('http://192.168.20.2'),
        caPem: 'ca',
      );
      expect(target.origin.toString(), 'https://192.168.20.2:3443');
      expect(target.origin.scheme, 'https');
      // 绝不能用 127.0.0.1：证书 SAN 是按选定网卡签的，用错网卡就不匹配
      expect(target.origin.host, isNot('127.0.0.1'));
    });

    test('缺 listenPort 时退回 3443（setup 的默认）', () {
      final target = MobileAccessTarget.fromSetup(
        const {'version': 2},
        lanAddress: Uri.parse('http://10.0.0.5'),
        caPem: 'ca',
      );
      expect(target.origin.port, 3443);
    });

    test('值相等按 origin 判 —— CA 变了仍算同一台', () {
      final a = MobileAccessTarget(
        origin: Uri.parse('https://192.168.20.2:3443'),
        caPem: 'ca-old',
      );
      final b = MobileAccessTarget(
        origin: Uri.parse('https://192.168.20.2:3443'),
        caPem: 'ca-new',
      );
      final c = MobileAccessTarget(
        origin: Uri.parse('https://192.168.1.11:3443'),
        caPem: 'ca-old',
      );
      // CA 会被续签（换 key）。按 CA 判等会把续签误判成「换了一台电脑」，
      // 用户于是被要求重新配对 —— 而实际上只是证书换了。
      expect(a, equals(b));
      expect(a.hashCode, b.hashCode);
      expect(a, isNot(equals(c)));
    });
  });

  group('MobileAccessTarget 过期', () {
    test('expiresAt 为 null 时永不过期', () {
      final target = MobileAccessTarget(
        origin: Uri.parse('https://192.168.20.2:3443'),
        caPem: 'x',
      );
      expect(target.isExpired, isFalse);
    });

    test('已过期为 true', () {
      final target = MobileAccessTarget(
        origin: Uri.parse('https://192.168.20.2:3443'),
        caPem: 'x',
        expiresAt: now() - 1000,
      );
      expect(target.isExpired, isTrue);
    });
  });

  group('MobileAccessCredentials 过期判定', () {
    test('session 在到期前 30s 就算过期（避免边界抖动）', () {
      final creds = MobileAccessCredentials(
        deviceId: 'd',
        instanceId: 'i',
        deviceToken: 'dt',
        sessionToken: 'st',
        csrfToken: 'ct',
        sessionExpiresAt: now() + 20 * 1000, // 还剩 20s
        deviceExpiresAt: now() + 3600 * 1000,
      );
      // 20s < 30s 提前量 → 判定为已过期，触发续期。
      // 否则用户会在最后 20 秒里看到「突然连不上」。
      expect(creds.sessionExpired, isTrue);
      expect(creds.sessionSecondsLeft, inInclusiveRange(19, 20));
    });

    test('session 还有 5 分钟时不算过期', () {
      final creds = MobileAccessCredentials(
        deviceId: 'd',
        instanceId: 'i',
        deviceToken: 'dt',
        sessionToken: 'st',
        csrfToken: 'ct',
        sessionExpiresAt: now() + 300 * 1000,
        deviceExpiresAt: now() + 3600 * 1000,
      );
      expect(creds.sessionExpired, isFalse);
      expect(creds.deviceExpired, isFalse);
      expect(creds.sessionSecondsLeft, inInclusiveRange(299, 300));
    });

    test('已过期的 sessionSecondsLeft 是 0 而不是负数', () {
      final creds = MobileAccessCredentials(
        deviceId: 'd',
        instanceId: 'i',
        deviceToken: 'dt',
        sessionToken: 'st',
        csrfToken: 'ct',
        sessionExpiresAt: now() - 5000,
        deviceExpiresAt: now() + 3600 * 1000,
      );
      expect(creds.sessionSecondsLeft, 0);
      expect(creds.sessionExpired, isTrue);
    });

    test('device 过期与 session 过期是两件事', () {
      // device 过期 = 配对失效，只能重新扫码；
      // session 过期 = 还能用 deviceToken 续期。混淆这两者会让 App
      // 在续期成功时错误地把用户踢回配对页。
      final creds = MobileAccessCredentials(
        deviceId: 'd',
        instanceId: 'i',
        deviceToken: 'dt',
        sessionToken: 'st',
        csrfToken: 'ct',
        sessionExpiresAt: now() - 1000,
        deviceExpiresAt: now() + 3600 * 1000,
      );
      expect(creds.sessionExpired, isTrue);
      expect(creds.deviceExpired, isFalse);
    });

    test('device 已过期时两个判定都为 true', () {
      final creds = MobileAccessCredentials(
        deviceId: 'd',
        instanceId: 'i',
        deviceToken: 'dt',
        sessionToken: 'st',
        csrfToken: 'ct',
        sessionExpiresAt: now() - 1000,
        deviceExpiresAt: now() - 1000,
      );
      expect(creds.deviceExpired, isTrue);
      expect(creds.sessionExpired, isTrue);
    });

    test('toString **不泄漏任何凭据**', () {
      final creds = MobileAccessCredentials(
        deviceId: 'dev-1',
        instanceId: 'inst-1',
        deviceToken: 'DEVICE_TOKEN_SECRET',
        sessionToken: 'SESSION_TOKEN_SECRET',
        csrfToken: 'CSRF_SECRET',
        sessionExpiresAt: now() + 60000,
        deviceExpiresAt: now() + 3600 * 1000,
      );
      final text = creds.toString();
      expect(text.contains('DEVICE_TOKEN_SECRET'), isFalse);
      expect(text.contains('SESSION_TOKEN_SECRET'), isFalse);
      expect(text.contains('CSRF_SECRET'), isFalse);
      // 保留可诊断的部分
      expect(text.contains('dev-1'), isTrue);
      expect(text.contains('inst-1'), isTrue);
    });
  });
}

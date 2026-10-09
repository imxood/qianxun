/** 审计事件的一句话解释（docs/20 §4.5）。
 *
 *  这个模块从 `Audit.svelte` 里搬出来不是为了好看，是因为这份词表错过两次：
 *  ① 写的是设想中的 `pair.ok`/`instance.up`，服务端根本不发，落到 default
 *     把 `pair.submit_ok` 这样的原始标识符直接印给用户；
 *  ② 事件改名后，线上库里已写进去的老行没人管，页面把 `pair.console_create`
 *     当成未分类事件原样显示。
 *
 *  审计表只追加不改写，所以「服务端现在会发什么」和「库里已经有什么」是两套
 *  都必须覆盖的词表，而它们都会漂。搬到纯模块里 + `audit.test.ts` 把两侧钉死，
 *  漂了就是测试红，而不是上线以后看截图才发现。
 */

export interface AuditEventLike {
  event: string;
  instanceId: string | null;
  deviceId: string | null;
  detail: Record<string, unknown>;
}

/** 服务端**现在**会发出的事件名。与 db.rs / web.rs / hub.rs / main.rs 一一对应。
 *  改动服务端事件名时，这里必须同步改，否则下面的测试会红。 */
export const SERVER_EVENTS = [
  'login.ok',
  'login.fail',
  'login.locked',
  'pair.create',
  'pair.submit_ok',
  'pair.submit_fail',
  'pair.locked',
  'pair.grant',
  'instance.add',
  'instance.rotate',
  'instance.offline',
  'device.enter',
  'device.enter_fail',
  'device.revoke',
  'device.restore',
  'device.revoke_all',
  'device.rotate',
  'device.import',
] as const;

/** 2026-10 改名前写进审计表的事件名。它们不会被改写——审计只追加。 */
export const LEGACY_EVENTS = ['pair.console_create', 'device.console_revoke'] as const;

/** 一句话说清「谁、做了什么、在哪」。detail 里没有的字段就不编，
 *  主体缺了就退到 id，再缺就写「设备」。 */
export function describeEvent(event: AuditEventLike): string {
  const detail = event.detail ?? {};
  const who = (detail['name'] as string) ?? null;
  const ip = (detail['ip'] as string) ?? null;
  const tail = ip ? ` · ${ip}` : '';
  const device = event.deviceId ?? '设备';
  const instance = event.instanceId ?? '实例';
  const via = (detail['via'] as string) ?? '';
  // 同一个动作只写一条事件（db.rs 一次动作只写一条），来源记在 detail.via。
  const where = via === 'console' ? '控制台' : via === 'instance' ? '实例' : '命令行';

  switch (event.event) {
    case 'login.ok':
      return `${who ?? '用户'} 登录成功${tail}`;
    case 'login.fail':
      return `${who ?? '用户'} 密码错误${tail}`;
    case 'login.locked':
      return `${who ?? '用户'} 触发限流锁定${tail}`;
    // 配对：发码、手机兑换、手机进场三段。
    case 'pair.create':
      return `${where}生成配对码`;
    case 'pair.submit_ok':
      return `${device} 兑换配对码成功${tail}`;
    case 'pair.submit_fail':
      return `配对码错误${tail}`;
    case 'pair.locked':
      return `配对提交触发限流锁定${tail}`;
    case 'pair.grant':
      return `${device} 已接入`;
    // 实例
    case 'instance.add':
      return `${instance} 已登记`;
    case 'instance.rotate':
      return `${instance} 令牌已轮换`;
    case 'instance.offline':
      return `${instance} 掉线`;
    // 设备
    case 'device.enter':
      return `${device} 建立连接${tail}`;
    case 'device.enter_fail':
      return `设备凭据无效${tail}`;
    case 'device.revoke':
      return `${device} 已被${where}吊销`;
    case 'device.restore':
      return `${device} 已恢复`;
    case 'device.revoke_all':
      return '吊销全部设备';
    case 'device.rotate':
      return `${device} 凭据已轮换`;
    case 'device.import':
      return `${device} 由命令行导入`;
    // 遗留名 → 新名。这两条是「一个动作写两条事件」修掉的那对重复记录的一半，
    // 另一半（pair.create / device.revoke）还在，所以读起来仍然像同一个动作。
    case 'pair.console_create':
      return '控制台生成配对码';
    case 'device.console_revoke':
      return `${device} 已被控制台吊销`;
    default:
      // 不认识的事件不编句子，但也不能装作不存在：原始键仍然可见、可搜索。
      return `${event.event}（未分类事件）`;
  }
}

/** 失败事件名有两种拼法：老的 `login.fail` 是点号，新的 `device.enter_fail`
 *  是下划线。只判其中一种，下划线那批就会在页面上长得跟普通事件一样。 */
export function isFailure(event: string): boolean {
  return event.endsWith('.fail') || event.endsWith('_fail');
}

/** 徽标配色：成功绿、失败红、其余中性蓝。 */
export function eventTone(event: string): 'tl-ok' | 'tl-fail' | 'tl-info' {
  if (isFailure(event)) return 'tl-fail';
  if (event.endsWith('.ok') || event.endsWith('_ok')) return 'tl-ok';
  return 'tl-info';
}

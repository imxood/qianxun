import { describe, expect, it } from 'vitest';
import { LEGACY_EVENTS, SERVER_EVENTS, describeEvent, eventTone, isFailure } from './audit';

/** 这份词表错过两次，代价是「把原始标识符印给用户」和「线上老行认不出来」。
 *  两次都不是靠肉眼发现的，所以钉在这里。 */
describe('审计事件词表', () => {
  const withEvent = (event: string, rest: Partial<Parameters<typeof describeEvent>[0]> = {}) => ({
    event,
    instanceId: null,
    deviceId: null,
    detail: {},
    ...rest,
  });

  it('服务端会发的每个事件名都有中文句子，不落到「未分类」', () => {
    for (const name of SERVER_EVENTS) {
      expect(describeEvent(withEvent(name)), `事件 ${name} 没有对应句子`).not.toContain(
        '未分类事件',
      );
    }
  });

  it('历史遗留事件名也读得懂（审计只追加，老行不会消失）', () => {
    for (const name of LEGACY_EVENTS) {
      expect(describeEvent(withEvent(name)), `遗留事件 ${name} 没有对应句子`).not.toContain(
        '未分类事件',
      );
    }
  });

  it('遗留名只有那两个：多加一个就说明又编了个没发生过的键', () => {
    // 上一轮就是因为写了服务端根本不发的 pair.ok / instance.up 才翻车的。
    expect([...LEGACY_EVENTS].sort()).toEqual(['device.console_revoke', 'pair.console_create']);
  });

  it('真的遇到不认识的事件时，如实显示原始键而不是编句子', () => {
    expect(describeEvent(withEvent('some.future_event'))).toBe('some.future_event（未分类事件）');
  });

  it('来源记在 detail.via，一个动作只写一条事件', () => {
    expect(describeEvent(withEvent('pair.create', { detail: { via: 'console' } }))).toBe(
      '控制台生成配对码',
    );
    expect(describeEvent(withEvent('pair.create', { detail: { via: 'instance' } }))).toBe(
      '实例生成配对码',
    );
    expect(describeEvent(withEvent('pair.create', { detail: { via: 'cli' } }))).toBe(
      '命令行生成配对码',
    );
  });

  it('detail 缺字段时退到 id，再缺才写「设备」，不编造', () => {
    expect(describeEvent(withEvent('device.revoke'))).toBe('设备 已被吊销');
    expect(describeEvent(withEvent('device.revoke', { deviceId: 'dv_ab12' }))).toBe(
      'dv_ab12 已被吊销',
    );
    expect(describeEvent(withEvent('instance.add', { instanceId: 'inst-1' }))).toBe(
      'inst-1 已登记',
    );
  });

  it('没有 detail.via 就不说来源——宁可不说，也不凭空指控命令行', () => {
    // 改名前写进去的 pair.create 带的是空 detail，线上看得到它们同一秒还配着一行
    // pair.console_create，也就是多半来自控制台。曾经一律显示成「命令行生成配对码」。
    expect(describeEvent(withEvent('pair.create', { detail: {} }))).toBe('生成配对码');
    expect(describeEvent(withEvent('device.revoke', { deviceId: 'dv_ab12', detail: {} }))).toBe(
      'dv_ab12 已被吊销',
    );
    // 凭空猜一个不认识的 via 也照样不说，而不是当成命令行。
    expect(describeEvent(withEvent('pair.create', { detail: { via: 'who-knows' } }))).toBe(
      '生成配对码',
    );
  });

  it('来源 IP 只在 detail.ip 存在时才带出来', () => {
    expect(describeEvent(withEvent('login.ok', { detail: { name: 'manager' } }))).toBe(
      'manager 登录成功',
    );
    expect(
      describeEvent(withEvent('login.ok', { detail: { name: 'manager', ip: '1.2.3.4' } })),
    ).toBe('manager 登录成功 · 1.2.3.4');
  });

  it('徽标配色：失败红、成功绿、其余中性（下划线拼法也算失败）', () => {
    expect(eventTone('login.fail')).toBe('tl-fail');
    expect(eventTone('device.enter_fail')).toBe('tl-fail');
    expect(eventTone('pair.submit_fail')).toBe('tl-fail');
    expect(eventTone('login.ok')).toBe('tl-ok');
    expect(eventTone('pair.submit_ok')).toBe('tl-ok');
    expect(eventTone('device.revoke')).toBe('tl-info');
  });

  it('连续失败徽标认得下划线拼法（否则 enter_fail 永远不计数）', () => {
    expect(isFailure('login.fail')).toBe(true);
    expect(isFailure('device.enter_fail')).toBe(true);
    expect(isFailure('pair.submit_fail')).toBe(true);
    expect(isFailure('login.ok')).toBe(false);
    expect(isFailure('device.revoke')).toBe(false);
  });
});

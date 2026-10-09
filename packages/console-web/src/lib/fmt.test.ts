import { describe, expect, it } from 'vitest';

import { bytes, countdown, date, isoDate, percent, relativeTime, stamp } from './fmt';

// 固定基准时刻，避免用例随真实时间漂移。
const NOW = 1728000000000; // 2024-10-04T16:00:00Z
const seconds = (delta: number) => Math.floor(NOW / 1000) - delta;

describe('relativeTime', () => {
  it('空值与 0 都回落到破折号', () => {
    expect(relativeTime(null)).toBe('—');
    expect(relativeTime(undefined)).toBe('—');
    expect(relativeTime(0)).toBe('—');
  });

  it('分档：刚刚 / 分钟 / 小时 / 天 / 一周后回落到日期', () => {
    expect(relativeTime(seconds(5), NOW)).toBe('刚刚');
    expect(relativeTime(seconds(59), NOW)).toBe('刚刚');
    expect(relativeTime(seconds(60), NOW)).toBe('1 分钟前');
    expect(relativeTime(seconds(14 * 60), NOW)).toBe('14 分钟前');
    expect(relativeTime(seconds(3600), NOW)).toBe('1 小时');
    expect(relativeTime(seconds(3 * 3600), NOW)).toBe('3 小时');
    expect(relativeTime(seconds(86400), NOW)).toBe('1 天前');
    expect(relativeTime(seconds(6 * 86400), NOW)).toBe('6 天前');
  });

  it('超过一周显示日历日期而不是「30 天前」', () => {
    expect(relativeTime(seconds(30 * 86400), NOW)).toBe(date(seconds(30 * 86400)));
  });

  it('未来时间（服务器时钟超前）不显示负数', () => {
    expect(relativeTime(seconds(-120), NOW)).toBe('刚刚');
  });
});

describe('bytes', () => {
  it('零与负数归零', () => {
    expect(bytes(0)).toBe('0 B');
    expect(bytes(null)).toBe('0 B');
    expect(bytes(-5)).toBe('0 B');
  });

  it('整数不带小数，小数一位', () => {
    expect(bytes(512)).toBe('512 B');
    expect(bytes(1024)).toBe('1 KiB');
    expect(bytes(1536)).toBe('1.5 KiB');
  });

  it('三位数以上不再带小数，避免图表标签抖动', () => {
    expect(bytes(1024 * 1024)).toBe('1 MiB');
    expect(bytes(100 * 1024 * 1024)).toBe('100 MiB');
  });

  it('单位到 TiB 封顶，不溢出成 KiB（会读成「1 PiB」但其实是 1024 TiB）', () => {
    expect(bytes(1024 ** 4)).toBe('1 TiB');
    expect(bytes(1024 ** 5)).toBe('1024 TiB');
  });
});

describe('percent', () => {
  it('夹紧到 0..1，避免溢出条撑破布局', () => {
    expect(percent(0.2416)).toBe('24.2%');
    expect(percent(1.8)).toBe('100%');
    expect(percent(-0.2)).toBe('0%');
  });

  it('整百分比不带小数', () => {
    expect(percent(0.5)).toBe('50%');
    expect(percent(0)).toBe('0%');
  });
});

describe('countdown', () => {
  it('配对码倒计时 mm:ss，负数归零', () => {
    expect(countdown(300)).toBe('05:00');
    expect(countdown(59)).toBe('00:59');
    expect(countdown(0)).toBe('00:00');
    expect(countdown(-5)).toBe('00:00');
  });
});

describe('日期', () => {
  it('isoDate 输出 ISO 前 10 位（UTC，与库里 at 同口径）', () => {
    expect(isoDate(NOW / 1000)).toBe('2024-10-04');
  });

  it('isoDate 对空值给破折号', () => {
    expect(isoDate(null)).toBe('—');
    expect(isoDate(0)).toBe('—');
  });

  it('stamp 是本时区的 mm-dd hh:mm:ss（审计列表用）', () => {
    const text = stamp(NOW / 1000);
    expect(text).toMatch(/^\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
    expect(text.slice(0, 5)).toBe(date(NOW / 1000));
  });
});

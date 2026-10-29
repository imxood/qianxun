import { describe, expect, it } from 'vitest';
import { formatCompact, formatContext, formatIndex, formatPrice, formatTime } from './format';

describe('formatTime', () => {
  it('今天显示 HH:mm，更早显示 MM-DD', () => {
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 9, 5);
    expect(formatTime(today.getTime())).toBe(`09:05`);
    const earlier = new Date(now.getFullYear(), 0, 5, 23, 59);
    expect(formatTime(earlier.getTime())).toBe(`01-05`);
    expect(formatTime(null)).toBe('');
  });

  it('紧凑数字与价格', () => {
    expect(formatCompact(12_400_000)).toBe('12.4M');
    expect(formatCompact(768_000)).toBe('768K');
    expect(formatCompact(96)).toBe('96');
    expect(formatCompact(null)).toBe('—');
    expect(formatContext(1_000_000)).toBe('1M');
    expect(formatContext(1_048_576)).toBe('1M');
    expect(formatContext(131_072)).toBe('128K');
    expect(formatContext(null)).toBe('—');
    expect(formatPrice(2.04)).toBe('2.0');
    expect(formatPrice(0.078)).toBe('0.08');
    expect(formatPrice(15)).toBe('15');
    expect(formatPrice(null)).toBe('—');
    expect(formatIndex(45.36)).toBe('45.4');
    expect(formatIndex(null)).toBe('—');
  });
});

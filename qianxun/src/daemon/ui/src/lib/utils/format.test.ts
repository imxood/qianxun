// ──────────────────────────────────────────────────────────────────────────
// Stage 7a — format 工具测试
// 覆盖: 时间/字节/latency/截断
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest';
import { formatBytes, formatLatency, formatTimestamp, truncate } from './format';

describe('format utilities', () => {
	beforeAll(() => {
		vi.useFakeTimers();
		vi.setSystemTime(new Date('2026-06-02T12:00:00Z'));
	});
	afterAll(() => {
		vi.useRealTimers();
	});

	describe('formatTimestamp', () => {
		it('空值返 "—"', () => {
			expect(formatTimestamp(null)).toBe('—');
			expect(formatTimestamp(undefined)).toBe('—');
			expect(formatTimestamp('')).toBe('—');
		});

		it('无效输入返原值', () => {
			expect(formatTimestamp('not-a-date')).toBe('not-a-date');
		});

		it('30s 前返 "30s ago"', () => {
			const iso = new Date(Date.now() - 30 * 1000).toISOString();
			expect(formatTimestamp(iso)).toBe('30s ago');
		});

		it('5 分钟前返 "5m ago"', () => {
			const iso = new Date(Date.now() - 5 * 60 * 1000).toISOString();
			expect(formatTimestamp(iso)).toBe('5m ago');
		});

		it('2 小时前返 "2h ago"', () => {
			const iso = new Date(Date.now() - 2 * 3600 * 1000).toISOString();
			expect(formatTimestamp(iso)).toBe('2h ago');
		});

		it('3 天前返 "3d ago"', () => {
			const iso = new Date(Date.now() - 3 * 86400 * 1000).toISOString();
			expect(formatTimestamp(iso)).toBe('3d ago');
		});
	});

	describe('formatBytes', () => {
		it('空值返 "—"', () => {
			expect(formatBytes(null)).toBe('—');
			expect(formatBytes(undefined)).toBe('—');
		});

		it('< 1KB 走 B 单位', () => {
			expect(formatBytes(512)).toBe('512 B');
		});

		it('KB 范围', () => {
			expect(formatBytes(1024)).toBe('1.0 KB');
			expect(formatBytes(1024 * 100)).toBe('100.0 KB');
		});

		it('MB 范围', () => {
			expect(formatBytes(1024 * 1024)).toBe('1.0 MB');
			expect(formatBytes(1024 * 1024 * 5)).toBe('5.0 MB');
		});

		it('GB 范围', () => {
			expect(formatBytes(1024 * 1024 * 1024)).toBe('1.00 GB');
		});
	});

	describe('formatLatency', () => {
		it('空值返 "—"', () => {
			expect(formatLatency(null)).toBe('—');
		});

		it('< 1s 走 ms', () => {
			expect(formatLatency(234)).toBe('234ms');
		});

		it('>= 1s 走 s', () => {
			expect(formatLatency(1500)).toBe('1.50s');
		});
	});

	describe('truncate', () => {
		it('空字符串', () => {
			expect(truncate('', 10)).toBe('');
		});

		it('短字符串原样返回', () => {
			expect(truncate('hello', 10)).toBe('hello');
		});

		it('长字符串截断', () => {
			const r = truncate('hello world!', 8);
			expect(r.length).toBe(8);
			expect(r.endsWith('…')).toBe(true);
		});
	});
});

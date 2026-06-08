// ──────────────────────────────────────────────────────────────────────────
// Stage 7a — cn 工具测试
// 覆盖: clsx + tailwind-merge 行为, 冲突时后者覆盖
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect } from 'vitest';
import { cn } from '../utils';

describe('cn() utility', () => {
	it('合并简单 className 数组', () => {
		expect(cn('a', 'b', 'c')).toBe('a b c');
	});

	it('跳过 falsy 值', () => {
		expect(cn('a', undefined, false, null, 'b')).toBe('a b');
	});

	it('支持对象语法 (clsx 语义)', () => {
		expect(cn('base', { active: true, disabled: false })).toBe('base active');
	});

	it('冲突 class 走 tailwind-merge (后者覆盖前者)', () => {
		// bg-red-500 被 bg-blue-500 覆盖
		expect(cn('bg-red-500', 'bg-blue-500')).toBe('bg-blue-500');
		// p-2 被 p-4 覆盖
		expect(cn('p-2', 'p-4')).toBe('p-4');
	});

	it('truncate + tailwind classes 一起', () => {
		const result = cn('text-sm', 'text-lg', 'font-bold');
		// text-lg 覆盖 text-sm
		expect(result).toContain('text-lg');
		expect(result).not.toContain('text-sm');
		expect(result).toContain('font-bold');
	});
});

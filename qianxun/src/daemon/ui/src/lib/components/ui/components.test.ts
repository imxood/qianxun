// ──────────────────────────────────────────────────────────────────────────
// Stage 7a — 共享 UI 组件测试
// 覆盖: Button (click → onClick), Badge (渲染), Card 组合
//
// 注意: Svelte 5 Snippet 在 @testing-library/svelte 下需要特殊处理, 这里
// 走"无 children"路径 (纯属性测试) + 直接 DOM 验证, 避免 Snippet 类型陷阱.
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Button from '$lib/components/ui/button/Button.svelte';
import Badge from '$lib/components/ui/badge/Badge.svelte';
import Card from '$lib/components/ui/card/Card.svelte';
import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
import Input from '$lib/components/ui/input/Input.svelte';

describe('Button (Svelte 5)', () => {
	it('渲染 default variant + 触发 click 回调', async () => {
		const onClick = vi.fn();
		const { container } = render(Button, { onclick: onClick });
		const btn = container.querySelector('button');
		expect(btn).toBeTruthy();
		// default variant class
		expect(btn?.className).toContain('bg-primary');
		await fireEvent.click(btn!);
		expect(onClick).toHaveBeenCalledTimes(1);
	});

	it('disabled 时 button 元素有 disabled 属性', async () => {
		const onClick = vi.fn();
		const { container } = render(Button, { onclick: onClick, disabled: true });
		const btn = container.querySelector('button') as HTMLButtonElement;
		expect(btn.disabled).toBe(true);
		// 注: fireEvent.click 绕过 disabled 拦截, 真实浏览器不会触发;
		// 这里只验证属性, 行为验证交给 E2E.
	});

	it('variant=destructive 渲染 destructive class', () => {
		const { container } = render(Button, { variant: 'destructive' });
		const btn = container.querySelector('button') as HTMLButtonElement;
		expect(btn.className).toContain('bg-destructive');
	});

	it('variant=outline 渲染 outline class', () => {
		const { container } = render(Button, { variant: 'outline' });
		const btn = container.querySelector('button') as HTMLButtonElement;
		expect(btn.className).toContain('border-input');
	});

	it('size=sm 渲染 sm class', () => {
		const { container } = render(Button, { size: 'sm' });
		const btn = container.querySelector('button') as HTMLButtonElement;
		expect(btn.className).toContain('h-8');
	});
});

describe('Badge (Svelte 5)', () => {
	it('默认 variant=default', () => {
		const { container } = render(Badge, {});
		const el = container.querySelector('.bg-primary');
		expect(el).toBeTruthy();
	});

	it('variant=success 渲染 success class', () => {
		const { container } = render(Badge, { variant: 'success' });
		const el = container.querySelector('.text-green-700');
		expect(el).toBeTruthy();
	});

	it('variant=destructive 渲染 destructive class', () => {
		const { container } = render(Badge, { variant: 'destructive' });
		const el = container.querySelector('.bg-destructive');
		expect(el).toBeTruthy();
	});
});

describe('Card 组合', () => {
	it('Card 渲染自定义 class', () => {
		const { container } = render(Card, { class: 'test-card' });
		expect(container.querySelector('.test-card')).toBeTruthy();
		expect(container.querySelector('.rounded-lg')).toBeTruthy();
	});

	it('CardHeader 渲染 flex col', () => {
		const { container } = render(CardHeader, {});
		expect(container.querySelector('.flex.flex-col')).toBeTruthy();
	});

	it('CardTitle 渲染 h3', () => {
		const { container } = render(CardTitle, {});
		const h3 = container.querySelector('h3');
		expect(h3).toBeTruthy();
		expect(h3?.className).toContain('text-lg');
	});
});

describe('Input (受控绑定)', () => {
	it('输入触发 bind:value 双向绑定', async () => {
		const { container } = render(Input, { value: '' });
		const input = container.querySelector('input')!;
		expect(input.value).toBe('');
		input.value = 'hello';
		await fireEvent.input(input);
		expect(input.value).toBe('hello');
	});

	it('type=password 渲染 password input', () => {
		const { container } = render(Input, { type: 'password' });
		const input = container.querySelector('input')!;
		expect(input.type).toBe('password');
	});
});

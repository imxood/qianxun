// ──────────────────────────────────────────────────────────────────────────
// Stage 7a — +layout.svelte 渲染测试 (简化版: 不挂载整个 layout, 单独测 Sidebar)
// 避免 onMount + tick() 在 vitest 下的复杂交互, 走更轻量的子组件测试
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render } from '@testing-library/svelte';
import Sidebar from '$lib/components/layout/Sidebar.svelte';
import TopBar from '$lib/components/layout/TopBar.svelte';
import { authStore } from '$lib/stores/auth.svelte';

describe('Sidebar 路由导航', () => {
	beforeEach(() => {
		authStore.clear();
	});

	it('8 个导航链接都在 (4 核心管理 + 4 运维)', () => {
		const { getByTestId } = render(Sidebar);
		// 管理区
		expect(getByTestId('nav-llm').getAttribute('href')).toBe('/llm');
		expect(getByTestId('nav-skills').getAttribute('href')).toBe('/skills');
		expect(getByTestId('nav-mcp').getAttribute('href')).toBe('/mcp');
		expect(getByTestId('nav-tools').getAttribute('href')).toBe('/tools');
		// 运维区 (Stage 7b)
		expect(getByTestId('nav-memory').getAttribute('href')).toBe('/memory');
		expect(getByTestId('nav-sessions').getAttribute('href')).toBe('/sessions');
		expect(getByTestId('nav-config').getAttribute('href')).toBe('/config');
		expect(getByTestId('nav-system').getAttribute('href')).toBe('/system');
	});

	it('Sidebar 渲染 "管理" / "运维" 两个分区标签', () => {
		const { container } = render(Sidebar);
		// 找 group/section 文字
		expect(container.textContent).toContain('管理');
		expect(container.textContent).toContain('运维');
	});
});

describe('TopBar 状态指示 + token 配置', () => {
	beforeEach(() => {
		authStore.clear();
	});

	it('渲染 token 配置按钮', () => {
		const { getByTestId } = render(TopBar);
		expect(getByTestId('topbar-configure-token')).toBeTruthy();
	});

	it('未配置 token 时, 按钮文字 = "设置"', () => {
		const { getByTestId } = render(TopBar);
		const btn = getByTestId('topbar-configure-token');
		expect(btn.textContent?.trim()).toBe('设置');
	});

	it('已配置 token 时, 按钮文字 = "更换"', () => {
		authStore.setToken('my-jwt-token');
		const { getByTestId } = render(TopBar);
		const btn = getByTestId('topbar-configure-token');
		expect(btn.textContent?.trim()).toBe('更换');
	});

	it('onConfigureToken 回调被点击时触发', () => {
		const onConfigure = vi.fn();
		const { getByTestId } = render(TopBar, { onConfigureToken: onConfigure });
		const btn = getByTestId('topbar-configure-token') as HTMLButtonElement;
		btn.click();
		expect(onConfigure).toHaveBeenCalledTimes(1);
	});
});

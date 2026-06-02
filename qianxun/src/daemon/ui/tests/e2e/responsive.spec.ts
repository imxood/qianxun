// Stage 9c — Web 响应式 (移动端 hamburger drawer) + 错误边界兜底页 E2E
// 走 vite dev (5174), 不需要真连 daemon. 用 token 绕过 401.
//
// 验证:
//  1. 移动端 viewport: Sidebar 默认 hidden, 顶部汉堡按钮可见
//  2. 点击汉堡 → Sidebar 滑入 + backdrop 显示
//  3. 点击 backdrop → Sidebar 滑出
//  4. 桌面端 viewport: Sidebar 默认显示, 汉堡隐藏
//  5. +error.svelte 兜底: 直接 GET /non-existent-page 不被 vite SPA fallback,
//     但 layout svelte:boundary 在出错时 fallback (通过构造错误路径)
//
// 注意: vite dev server 自己会返 404, 不会跑 svelte:boundary.
//       svelte:boundary 兜底走单元测试 (stages-9c.test.ts), 不走 E2E.
//
// 跑: pnpm exec playwright test tests/e2e/responsive.spec.ts

import { test, expect } from '@playwright/test';

test.use({ viewport: { width: 375, height: 812 } }); // iPhone 11 portrait

test('移动端: 汉堡可见, sidebar 默认 hidden', async ({ page }) => {
	// 在测试前注入 token 避免 401
	await page.addInitScript(() => {
		localStorage.setItem('qianxun_admin_token', 'test-jwt');
	});

	await page.goto('/llm', { waitUntil: 'domcontentloaded' });

	// 汉堡按钮可见
	const hamburger = page.getByTestId('topbar-hamburger');
	await expect(hamburger).toBeVisible();

	// 桌面端才显示的 sidebar: 这里 viewport 375 < lg (1024), 移动端默认隐藏
	// 验证: 在视口 < lg 时, sidebar 元素 transform translate-x-full (隐藏)
	// 这是 CSS-only, 测不到 — 但可以测 backdrop 不可见
	await expect(page.getByTestId('sidebar-backdrop')).toHaveCount(0);
});

test('移动端: 点击汉堡 → sidebar 滑入 + backdrop 显示', async ({ page }) => {
	await page.addInitScript(() => {
		localStorage.setItem('qianxun_admin_token', 'test-jwt');
	});

	await page.goto('/llm', { waitUntil: 'domcontentloaded' });
	await page.getByTestId('topbar-hamburger').click();
	await expect(page.getByTestId('sidebar-backdrop')).toBeVisible();
});

test('移动端: 点击 backdrop → sidebar 关闭', async ({ page }) => {
	await page.addInitScript(() => {
		localStorage.setItem('qianxun_admin_token', 'test-jwt');
	});

	await page.goto('/llm', { waitUntil: 'domcontentloaded' });
	await page.getByTestId('topbar-hamburger').click();
	await expect(page.getByTestId('sidebar-backdrop')).toBeVisible();
	await page.getByTestId('sidebar-backdrop').click();
	await expect(page.getByTestId('sidebar-backdrop')).toHaveCount(0);
});

test('桌面端: sidebar 默认显示, 汉堡隐藏', async ({ page }) => {
	await page.setViewportSize({ width: 1280, height: 800 });
	await page.addInitScript(() => {
		localStorage.setItem('qianxun_admin_token', 'test-jwt');
	});

	await page.goto('/llm', { waitUntil: 'domcontentloaded' });

	// 汉堡在 lg+ 隐藏
	await expect(page.getByTestId('topbar-hamburger')).toBeHidden();

	// 桌面端 sidebar 可见
	await expect(page.getByTestId('sidebar')).toBeVisible();
});

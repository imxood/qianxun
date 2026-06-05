// ──────────────────────────────────────────────────────────────────────────
// Stage 9c — Settings 面板测试
// 覆盖:
//   1. Settings 页面 4 section 都渲染
//   2. Theme 切换: 3 button 触发 themeStore.setMode
//   3. Language 切换: 2 button 触发 setLocale
//   4. Token rotate: mock fetch, 验证 POST /v1/system/admin/rotate-token
//   5. Token rotate 失败 → ErrorBanner
//   6. Copy 按钮触发 navigator.clipboard
//   7. i18n 切换: en → settings.about.github 文案变
//   8. Sidebar 渲染 Settings 链接
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, waitFor } from '@testing-library/svelte';

const fetchMock = vi.fn();
beforeEach(() => {
	fetchMock.mockReset();
	vi.stubGlobal('fetch', fetchMock);
	localStorage.clear();
});
afterEach(() => {
	vi.unstubAllGlobals();
	localStorage.clear();
});

function jsonResponse(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

import { authStore } from '$lib/stores/auth.svelte';
import { themeStore } from '$lib/stores/theme.svelte';
import { setLocale, t, locale } from '$lib/i18n';
import { get } from 'svelte/store';

const loadSettingsPage = () => import('../routes/settings/+page.svelte');
const loadSidebar = () => import('../lib/components/layout/Sidebar.svelte');

// ── 1. 4 section 都在 ──────────────────────────────────────────
describe('Settings 页面 (Stage 9c)', () => {
	beforeEach(() => {
		authStore.setToken('test-jwt');
		// status endpoint 返 mock
		fetchMock.mockResolvedValue(
			jsonResponse({ status: 'running', version: '0.4.0', stage: '9c' })
		);
	});

	it('渲染 4 个 section: theme / language / token / about', async () => {
		const SettingsPage = (await loadSettingsPage()).default;
		const { findByTestId } = render(SettingsPage);
		await findByTestId('settings-page');
		expect(await findByTestId('settings-theme-section')).toBeTruthy();
		expect(await findByTestId('settings-language-section')).toBeTruthy();
		expect(await findByTestId('settings-token-section')).toBeTruthy();
		expect(await findByTestId('settings-about-section')).toBeTruthy();
	});

	// ── 2. Theme 切换 ─────────────────────────────────────────
	it('Theme 切换: 点击 dark 按钮 → themeStore.mode = dark + localStorage 持久化', async () => {
		const SettingsPage = (await loadSettingsPage()).default;
		const { getByTestId } = render(SettingsPage);
		await fireEvent.click(getByTestId('settings-theme-dark'));
		expect(themeStore.mode).toBe('dark');
		expect(localStorage.getItem('qianxun_web_theme')).toBe('dark');
		// 切到 system
		await fireEvent.click(getByTestId('settings-theme-system'));
		expect(themeStore.mode).toBe('system');
		expect(localStorage.getItem('qianxun_web_theme')).toBe('system');
		// 切到 light
		await fireEvent.click(getByTestId('settings-theme-light'));
		expect(themeStore.mode).toBe('light');
	});

	it('Theme 当前值在按钮上有 aria-pressed=true 高亮', async () => {
		themeStore.setMode('dark');
		const SettingsPage = (await loadSettingsPage()).default;
		const { getByTestId } = render(SettingsPage);
		const darkBtn = getByTestId('settings-theme-dark');
		expect(darkBtn.getAttribute('aria-pressed')).toBe('true');
		const lightBtn = getByTestId('settings-theme-light');
		expect(lightBtn.getAttribute('aria-pressed')).toBe('false');
	});

	// ── 3. Language 切换 ──────────────────────────────────────
	it('Language 切换: 点击 en 按钮 → locale = en + localStorage', async () => {
		setLocale('zh-CN');
		const SettingsPage = (await loadSettingsPage()).default;
		const { getByTestId } = render(SettingsPage);
		await fireEvent.click(getByTestId('settings-language-en'));
		expect(get(locale)).toBe('en');
		expect(localStorage.getItem('qianxun_lang')).toBe('en');
		// 切回 zh-CN
		await fireEvent.click(getByTestId('settings-language-zh-CN'));
		expect(get(locale)).toBe('zh-CN');
		expect(localStorage.getItem('qianxun_lang')).toBe('zh-CN');
	});

	// ── 4. Token rotate (mock fetch) ──────────────────────────
	it('Token rotate: 点击旋转按钮 → POST /v1/system/admin/rotate-token + 更新 authStore', async () => {
		// 第一次: status (settings about); 第二次: rotate
		fetchMock
			.mockResolvedValueOnce(jsonResponse({ status: 'running', version: '0.4.0', stage: '9c' }))
			.mockResolvedValueOnce(
				jsonResponse({
					token: 'eyJ_NEW_FAKE_TOKEN_xxx',
					exp: Math.floor(Date.now() / 1000) + 86400,
					sub: 'admin',
					expires_in: 86400
				})
			);
		authStore.setToken('old-token');
		const SettingsPage = (await loadSettingsPage()).default;
		const { getByTestId, findByTestId } = render(SettingsPage);
		await fireEvent.click(getByTestId('settings-token-rotate'));
		await findByTestId('settings-token-rotate-success');
		// fetch 被调三次: status (onMount) + rotate (用户点按钮) + status
		// (rotate 后 token 变, $effect 触发重 fetch, 拿新 token 下的 daemon 状态).
		// 2026-06-04: 之前是 2 次 (status + rotate), 改成 3 次.
		expect(fetchMock).toHaveBeenCalledTimes(3);
		const rotateCall = fetchMock.mock.calls[1]!;
		expect(rotateCall[0]).toBe('/v1/system/admin/rotate-token');
		expect((rotateCall[1] as RequestInit).method).toBe('POST');
		const headers = (rotateCall[1] as RequestInit).headers as Record<string, string>;
		expect(headers['Authorization']).toBe('Bearer old-token');
		// authStore 应被更新为新 token
		expect(authStore.token).toBe('eyJ_NEW_FAKE_TOKEN_xxx');
	});

	// ── 5. Token rotate 失败 → ErrorBanner ───────────────────
	it('Token rotate 失败 → ErrorBanner 显示', async () => {
		// status 成功 + rotate 失败
		fetchMock
			.mockResolvedValueOnce(jsonResponse({ status: 'running', version: '0.4.0', stage: '9c' }))
			.mockResolvedValueOnce(jsonResponse({ error: 'forbidden' }, 403));
		authStore.setToken('valid-jwt');
		const SettingsPage = (await loadSettingsPage()).default;
		const { getByTestId, findByTestId } = render(SettingsPage);
		await fireEvent.click(getByTestId('settings-token-rotate'));
		// ErrorBanner 出现 (Stage 7a 通用组件, 用 class 找)
		const err = await findByTestId((_: string, el: Element | null) =>
			Boolean(el?.className?.includes('text-destructive')) ||
			Boolean(el?.className?.includes('error-state'))
		).catch(() => null);
		// 至少 fetch 被调且抛错
		await waitFor(() => {
			expect(fetchMock).toHaveBeenCalledTimes(2);
		});
		expect(err).toBeTruthy();
	});

	// ── 6. Copy 按钮调 navigator.clipboard ────────────────────
	it('Copy 按钮调 navigator.clipboard.writeText', async () => {
		const writeTextMock = vi.fn().mockResolvedValue(undefined);
		Object.defineProperty(navigator, 'clipboard', {
			value: { writeText: writeTextMock },
			configurable: true
		});
		fetchMock.mockResolvedValue(
			jsonResponse({ status: 'running', version: '0.4.0', stage: '9c' })
		);
		authStore.setToken('my-jwt-token-for-copy');
		const SettingsPage = (await loadSettingsPage()).default;
		const { getByTestId } = render(SettingsPage);
		await fireEvent.click(getByTestId('settings-token-copy'));
		expect(writeTextMock).toHaveBeenCalledWith('my-jwt-token-for-copy');
	});

	// ── 7. i18n 切换影响显示 ─────────────────────────────────
	it('i18n 切换: en → 关于文案变 (English)', async () => {
		setLocale('zh-CN');
		fetchMock.mockResolvedValue(
			jsonResponse({ status: 'running', version: '0.4.0', stage: '9c' })
		);
		const SettingsPage = (await loadSettingsPage()).default;
		const { findByTestId, getByTestId } = render(SettingsPage);
		await findByTestId('settings-about-section');
		// zh-CN: 简体中文
		const langBtnZh = getByTestId('settings-language-zh-CN');
		expect(langBtnZh.textContent).toContain('简体中文');
		// 切到 en
		await fireEvent.click(getByTestId('settings-language-en'));
		// en: 'English' 文案
		expect(getByTestId('settings-language-en').textContent).toContain('English');
		// 关于里的链接文案也变
		expect(getByTestId('settings-about-github').textContent).toContain('GitHub');
		// 验证: i18n 触发的 re-render 让 about.desc 也变
		expect(t('settings.about.docs')).toBe('Documentation');
	});

	// ── 8. Sidebar 渲染 Settings 链接 ────────────────────────
	it('Sidebar 渲染 Settings 链接 (/settings, 独立 "系统" 区)', async () => {
		setLocale('zh-CN');
		const Sidebar = (await loadSidebar()).default;
		const { getByTestId } = render(Sidebar);
		const link = getByTestId('nav-settings');
		// 2026-06-05 fix v2: paths.base='/ui' 下 href 用 `base` 拼 = `/ui/settings`
		// (跟 Sidebar 9 nav 一致, 见 layout.test.ts)
		expect(link.getAttribute('href')).toBe('/ui/settings');
		// 链接应包含 'Settings' 文字
		expect(link.textContent?.toLowerCase()).toContain('settings');
		// 容器里应出现"系统"分组标签
		const navEl = link.closest('aside');
		expect(navEl?.textContent).toContain('系统');
		expect(navEl?.textContent).toContain('管理');
		expect(navEl?.textContent).toContain('运维');
	});
});

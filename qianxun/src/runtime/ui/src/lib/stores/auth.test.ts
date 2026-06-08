// ──────────────────────────────────────────────────────────────────────────
// Stage 10a — authStore + auth API 单元测试 (扩展自 Stage 7a)
//
// 新增覆盖:
//   - login() 走通 — 调 api/auth.login + 写 token/exp/sub
//   - login() 错误密码 — 抛 ApiError, store 不写
//   - token 过期检测 — init() 时清过期 token
//   - logout() 清 store + 调 api
//   - changePassword() 调 API + Bearer 头
//   - i18n 切换 — 错误消息随 locale 变
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { authStore } from './auth.svelte';
import { login as apiLogin, changePassword as apiChangePassword, logout as apiLogout } from '$lib/api/auth';
import { setLocale, rawMessage, t } from '$lib/i18n';

// mock global.fetch (auth API calls go through fetch / fetchWithAuth)
const fetchMock = vi.fn();

beforeEach(() => {
	localStorage.clear();
	authStore.clear();
	fetchMock.mockReset();
	vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
	vi.unstubAllGlobals();
});

/** helper: 构造 fetch Mock 一个 ok response (JSON) */
function mockOk(body: unknown): void {
	fetchMock.mockResolvedValueOnce({
		ok: true,
		status: 200,
		headers: { get: (k: string) => (k.toLowerCase() === 'content-type' ? 'application/json' : null) },
		json: async () => body,
		text: async () => JSON.stringify(body)
	});
}

function mockErr(status: number, body: unknown): void {
	fetchMock.mockResolvedValueOnce({
		ok: false,
		status,
		statusText: status === 401 ? 'Unauthorized' : 'Bad Request',
		headers: { get: (k: string) => (k.toLowerCase() === 'content-type' ? 'application/json' : null) },
		json: async () => body,
		text: async () => JSON.stringify(body)
	});
}

describe('authStore (Stage 10a — password login + JWT)', () => {
	it('login (走通): 调 api, 写 token + exp + sub 到 localStorage', async () => {
		const exp = Math.floor(Date.now() / 1000) + 3600;
		mockOk({ token: 'jwt-from-server', exp, sub: 'admin', expires_in: 3600 });

		await authStore.login('correct-password');

		expect(authStore.token).toBe('jwt-from-server');
		expect(authStore.expiresAt).toBe(exp);
		expect(authStore.sub).toBe('admin');
		expect(authStore.isAuthenticated).toBe(true);

		expect(localStorage.getItem('qianxun_admin_token')).toBe('jwt-from-server');
		expect(localStorage.getItem('qianxun_admin_token_exp')).toBe(String(exp));
		expect(localStorage.getItem('qianxun_admin_sub')).toBe('admin');
	});

	it('login (错误密码): mock 401 → 抛 ApiError, store 不写', async () => {
		mockErr(401, { error: 'invalid_credentials', message: 'Invalid password' });

		await expect(authStore.login('wrong-password')).rejects.toThrow();

		expect(authStore.token).toBeNull();
		expect(authStore.expiresAt).toBeNull();
		expect(authStore.isAuthenticated).toBe(false);
		expect(localStorage.getItem('qianxun_admin_token')).toBeNull();
	});

	it('init (token 过期): localStorage 有过期 token → init 时自动清', () => {
		const pastExp = Math.floor(Date.now() / 1000) - 100;
		localStorage.setItem('qianxun_admin_token', 'old-jwt');
		localStorage.setItem('qianxun_admin_token_exp', String(pastExp));
		localStorage.setItem('qianxun_admin_sub', 'admin');

		authStore.init();

		// init 之后应清掉过期 token
		expect(authStore.token).toBeNull();
		expect(authStore.expiresAt).toBeNull();
		expect(authStore.isAuthenticated).toBe(false);
		expect(localStorage.getItem('qianxun_admin_token')).toBeNull();
	});

	it('init (token 有效): 加载到内存, isAuthenticated=true', () => {
		const futureExp = Math.floor(Date.now() / 1000) + 3600;
		localStorage.setItem('qianxun_admin_token', 'valid-jwt');
		localStorage.setItem('qianxun_admin_token_exp', String(futureExp));
		localStorage.setItem('qianxun_admin_sub', 'admin');

		authStore.init();

		expect(authStore.token).toBe('valid-jwt');
		expect(authStore.expiresAt).toBe(futureExp);
		expect(authStore.isAuthenticated).toBe(true);
	});

	it('logout: 调 api + 清 store + 清 localStorage', async () => {
		// 先登录
		const exp = Math.floor(Date.now() / 1000) + 3600;
		mockOk({ token: 'jwt', exp, sub: 'admin', expires_in: 3600 });
		await authStore.login('pw');
		expect(authStore.isAuthenticated).toBe(true);

		// 登出 — 第二个 mock 是 logout
		mockOk({ status: 'ok', message: 'logged out' });
		await authStore.logout();

		expect(authStore.token).toBeNull();
		expect(authStore.expiresAt).toBeNull();
		expect(authStore.isAuthenticated).toBe(false);
		expect(localStorage.getItem('qianxun_admin_token')).toBeNull();
	});

	it('logout (api 失败): 仍清 localStorage (token 是 stateless)', async () => {
		// 先登录
		const exp = Math.floor(Date.now() / 1000) + 3600;
		mockOk({ token: 'jwt', exp, sub: 'admin', expires_in: 3600 });
		await authStore.login('pw');

		// 登出 — API 返 500
		mockErr(500, { error: 'internal' });
		await authStore.logout();

		// 客户端仍清 (fire-and-forget)
		expect(authStore.token).toBeNull();
		expect(localStorage.getItem('qianxun_admin_token')).toBeNull();
	});

	it('setSession: 手动写 token/exp/sub (给 rotate 用)', () => {
		const exp = Math.floor(Date.now() / 1000) + 7200;
		authStore.setSession('new-jwt', exp, 'admin');

		expect(authStore.token).toBe('new-jwt');
		expect(authStore.expiresAt).toBe(exp);
		expect(authStore.sub).toBe('admin');
		expect(localStorage.getItem('qianxun_admin_token')).toBe('new-jwt');
	});

	it('isAuthenticated: 各种情况下判定', () => {
		// 1. 无 token → false
		expect(authStore.isAuthenticated).toBe(false);

		// 2. 有 token 但无 exp (legacy) → true
		authStore.setToken('legacy-jwt');
		expect(authStore.isAuthenticated).toBe(true);

		// 3. 有 token + 未来 exp → true
		const futureExp = Math.floor(Date.now() / 1000) + 3600;
		authStore.setSession('jwt', futureExp, 'admin');
		expect(authStore.isAuthenticated).toBe(true);

		// 4. 有 token + 过去 exp → false
		const pastExp = Math.floor(Date.now() / 1000) - 1;
		authStore.setSession('jwt', pastExp, 'admin');
		expect(authStore.isAuthenticated).toBe(false);
	});
});

describe('auth API (Stage 10a)', () => {
	it('login: 走 /v1/auth/login, 返 LoginResponse', async () => {
		const mockResponse = {
			token: 'eyJ.fake.jwt',
			exp: 1234567890,
			sub: 'admin',
			expires_in: 86400
		};
		mockOk(mockResponse);

		const result = await apiLogin('test-password');
		expect(result).toEqual(mockResponse);
		expect(fetchMock).toHaveBeenCalledWith(
			'/v1/auth/login',
			expect.objectContaining({
				method: 'POST',
				body: JSON.stringify({ password: 'test-password' })
			})
		);
	});

	it('login (401): 抛 ApiError (status=401)', async () => {
		mockErr(401, { error: 'invalid_credentials', message: 'Invalid password' });

		await expect(apiLogin('wrong')).rejects.toThrow();
	});

	it('changePassword: POST /v1/auth/change-password, 带 Bearer token', async () => {
		// 先登录以注入 token 到 store
		const futureExp = Math.floor(Date.now() / 1000) + 3600;
		authStore.setSession('test-jwt', futureExp, 'admin');
		authStore.init();

		mockOk({ status: 'ok', message: 'changed' });

		await apiChangePassword('old-pw', 'new-pw');

		const calls = fetchMock.mock.calls;
		expect(calls).toHaveLength(1);
		const [url, init] = calls[0] as [string, RequestInit];
		expect(url).toBe('/v1/auth/change-password');
		// fetch header 名称大小写不敏感 — fetchMock 返 "Authorization" 首字母大写
		const headers = init.headers as Record<string, string>;
		const authHeader = headers['Authorization'] ?? headers['authorization'];
		expect(authHeader).toBe('Bearer test-jwt');
		expect(init.method).toBe('POST');
		expect(init.body).toBe(JSON.stringify({ old_password: 'old-pw', new_password: 'new-pw' }));

		authStore.clear();
	});

	it('logout: POST /v1/auth/logout, 带 Bearer token', async () => {
		// 先登录
		const futureExp = Math.floor(Date.now() / 1000) + 3600;
		authStore.setSession('test-jwt', futureExp, 'admin');
		authStore.init();

		mockOk({ status: 'ok', message: 'logged out' });
		await apiLogout();

		const calls = fetchMock.mock.calls;
		expect(calls).toHaveLength(1);
		const [url, init] = calls[0] as [string, RequestInit];
		expect(url).toBe('/v1/auth/logout');
		const headers = init.headers as Record<string, string>;
		const authHeader = headers['Authorization'] ?? headers['authorization'];
		expect(authHeader).toBe('Bearer test-jwt');
		expect(init.method).toBe('POST');

		authStore.clear();
	});
});

describe('i18n (Stage 10a auth messages)', () => {
	beforeEach(() => {
		setLocale('zh-CN');
	});

	it('zh-CN: auth.login.title 翻译', () => {
		expect(rawMessage('zh-CN', 'auth.login.title')).toBe('登录');
		expect(rawMessage('zh-CN', 'auth.login.error')).toBe('密码错误');
	});

	it('en: auth.login.title 翻译', () => {
		expect(rawMessage('en', 'auth.login.title')).toBe('Sign in');
		expect(rawMessage('en', 'auth.login.error')).toBe('Incorrect password');
	});

	it('zh-CN: settings.token.change_password / logout 翻译', () => {
		expect(rawMessage('zh-CN', 'settings.token.change_password')).toBe('修改密码');
		expect(rawMessage('zh-CN', 'settings.token.logout')).toBe('登出');
	});

	it('en: settings.token.change_password / logout 翻译', () => {
		expect(rawMessage('en', 'settings.token.change_password')).toBe('Change password');
		expect(rawMessage('en', 'settings.token.logout')).toBe('Logout');
	});

	it('i18n 切换: t() 跟随 setLocale', () => {
		setLocale('zh-CN');
		expect(t('auth.login.error')).toBe('密码错误');
		setLocale('en');
		expect(t('auth.login.error')).toBe('Incorrect password');
		setLocale('zh-CN'); // restore
	});
});

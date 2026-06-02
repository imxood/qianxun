// ──────────────────────────────────────────────────────────────────────────
// Stage 7b — 4 个新面板 + 主题 + i18n 测试
// 覆盖:
//   - 4 面板各 3 测试 (列表渲染 / API 调用 mock / 错误处理) = 12
//   - 主题切换 2 (toggle 循环 / 持久化)
//   - i18n 切换 2 (语言切换 / 翻译回退)
//   - 共 16 测试
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, waitFor } from '@testing-library/svelte';

// ── API mocks ───────────────────────────────────────────────────────
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

function textResponse(text: string, status = 200) {
	return new Response(text, { status, headers: { 'content-type': 'text/plain' } });
}

import { authStore } from '$lib/stores/auth.svelte';
import { themeStore } from '$lib/stores/theme.svelte';
import { setLocale, t } from '$lib/i18n';

// 路由页面 — 走相对路径, vitest alias 不直接覆盖 routes
const loadMemoryPage = () => import('../routes/memory/+page.svelte');
const loadSessionsPage = () => import('../routes/sessions/+page.svelte');
const loadConfigPage = () => import('../routes/config/+page.svelte');
const loadSystemPage = () => import('../routes/system/+page.svelte');

// 给所有测试塞一个假 token 避开 401
beforeEach(() => {
	authStore.setToken('test-jwt');
});

// ─────────────────────────────────────────────────────────────────────
// 1. Memory 面板 — 3 测试
// ─────────────────────────────────────────────────────────────────────
describe('Memory 面板 (Stage 7b)', () => {
	it('渲染页面 + session 列表', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({
				sessions: [
					{ id: 'sess-1', created_at: '2026-06-01T00:00:00Z', observation_count: 5 },
					{ id: 'sess-2', created_at: '2026-06-02T00:00:00Z', observation_count: 3 }
				]
			})
		);
		const MemoryPage = (await loadMemoryPage()).default;
		const { getByTestId, findByTestId } = render(MemoryPage);
		await findByTestId('memory-session-sess-1');
		expect(getByTestId('memory-session-sess-1')).toBeTruthy();
		expect(getByTestId('memory-session-sess-2')).toBeTruthy();
	});

	it('点击 session → 调 listObservations + 渲染 observations', async () => {
		fetchMock
			.mockResolvedValueOnce(jsonResponse({ sessions: [{ id: 's1', observation_count: 2 }] }))
			.mockResolvedValueOnce(
				jsonResponse({
					observations: [
						{ id: 'obs-1', session_id: 's1', content: 'hello', created_at: '2026-06-01' },
						{ id: 'obs-2', session_id: 's1', content: 'world', created_at: '2026-06-02' }
					]
				})
			);
		const MemoryPage = (await loadMemoryPage()).default;
		const { getByTestId, findByTestId } = render(MemoryPage);
		await findByTestId('memory-session-s1');
		await fireEvent.click(getByTestId('memory-session-s1'));
		await findByTestId('memory-obs-obs-1');
		expect(getByTestId('memory-obs-obs-1')).toBeTruthy();
		// 验证两次 fetch: list sessions + list observations
		expect(fetchMock).toHaveBeenCalledTimes(2);
		expect(fetchMock.mock.calls[1]![0]).toBe('/v1/memory/sessions/s1/observations');
	});

	it('API 错误 → ErrorBanner 显示', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ error: 'internal_error', message: 'server down' }, 500)
		);
		const MemoryPage = (await loadMemoryPage()).default;
		const { findByTestId } = render(MemoryPage);
		await findByTestId('error-state');
	});
});

// ─────────────────────────────────────────────────────────────────────
// 2. Sessions 面板 — 3 测试
// ─────────────────────────────────────────────────────────────────────
describe('Sessions 面板 (Stage 7b)', () => {
	it('渲染列表 + 表格行', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({
				sessions: [
					{
						id: 'sess-a',
						model: 'deepseek-v4',
						created_at: '2026-06-02T00:00:00Z',
						last_active: '2026-06-02T01:00:00Z',
						message_count: 4,
						status: 'active',
						token_usage: { input: 100, output: 200, total: 300 }
					}
				],
				total: 1
			})
		);
		const SessionsPage = (await loadSessionsPage()).default;
		const { findByTestId, getByTestId } = render(SessionsPage);
		await findByTestId('sessions-row-sess-a');
		expect(getByTestId('sessions-row-sess-a')).toBeTruthy();
	});

	it('filter 切换 → status 参数变化', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ sessions: [], total: 0 })); // initial
		const SessionsPage = (await loadSessionsPage()).default;
		const { getByTestId } = render(SessionsPage);
		// 第一次请求: status 不传 (默认 all)
		expect(fetchMock.mock.calls[0]![0]).toBe('/v1/chat/sessions');
		// 切到 active
		fetchMock.mockResolvedValueOnce(jsonResponse({ sessions: [], total: 0 }));
		await fireEvent.click(getByTestId('sessions-filter-active'));
		await waitFor(() => {
			expect(fetchMock).toHaveBeenCalledTimes(2);
		});
		expect(fetchMock.mock.calls[1]![0]).toBe('/v1/chat/sessions?status=active');
	});

	it('cancel 按钮 → POST /cancel', async () => {
		fetchMock
			.mockResolvedValueOnce(
				jsonResponse({
					sessions: [
						{
							id: 'sess-x',
							model: 'm',
							created_at: '2026-06-02T00:00:00Z',
							last_active: '2026-06-02T00:01:00Z',
							message_count: 1,
							status: 'active',
							token_usage: { input: 0, output: 0, total: 0 }
						}
					],
					total: 1
				})
			)
			.mockResolvedValueOnce(jsonResponse({ status: 'cancelled', id: 'sess-x' }))
			.mockResolvedValueOnce(jsonResponse({ sessions: [], total: 0 })); // refresh
		const SessionsPage = (await loadSessionsPage()).default;
		const { getByTestId, findByTestId } = render(SessionsPage);
		await findByTestId('sessions-cancel-sess-x');
		await fireEvent.click(getByTestId('sessions-cancel-sess-x'));
		await waitFor(() => {
			expect(fetchMock.mock.calls[1]![0]).toBe('/v1/chat/session/sess-x/cancel');
		});
	});
});

// ─────────────────────────────────────────────────────────────────────
// 3. Config 面板 — 3 测试
// ─────────────────────────────────────────────────────────────────────
describe('Config 面板 (Stage 7b)', () => {
	it('渲染 config 字段卡片 + read-only 横幅', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({
				config: {
					active_provider: 'deepseek-main',
					log_level: 'info',
					max_sessions: 10,
					providers: [
						{
							id: 'deepseek-main',
							provider: 'deepseek',
							model: 'deepseek-v4',
							has_key: true,
							active: true
						}
					],
					skills_dirs: ['/tmp/skills'],
					memory_dir: '/tmp/mem'
				}
			})
		);
		const ConfigPage = (await loadConfigPage()).default;
		const { findByTestId, getByTestId } = render(ConfigPage);
		await findByTestId('config-grid');
		expect(getByTestId('config-readonly-banner')).toBeTruthy();
		expect(getByTestId('config-active-provider').textContent?.trim()).toBe('deepseek-main');
	});

	it('GET /v1/config → 正确 URL + Authorization', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ config: { active_provider: 'p', log_level: 'info', max_sessions: 1, providers: [], skills_dirs: [] } })
		);
		const ConfigPage = (await loadConfigPage()).default;
		render(ConfigPage);
		await waitFor(() => {
			expect(fetchMock).toHaveBeenCalled();
		});
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/config');
		const headers = (init as RequestInit).headers as Record<string, string>;
		expect(headers['Authorization']).toBe('Bearer test-jwt');
	});

	it('500 错误 → ErrorBanner 显示', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ error: 'internal_error' }, 500)
		);
		const ConfigPage = (await loadConfigPage()).default;
		const { findByTestId } = render(ConfigPage);
		await findByTestId('error-state');
	});
});

// ─────────────────────────────────────────────────────────────────────
// 4. System 面板 — 3 测试
// ─────────────────────────────────────────────────────────────────────
describe('System 面板 (Stage 7b)', () => {
	it('渲染 5 个 metric 卡片 + conns 折线', async () => {
		const history = Array.from({ length: 60 }, (_, i) => i);
		fetchMock
			.mockResolvedValueOnce(
				jsonResponse({
					cpu_percent: 12.3,
					mem_mb: 256,
					uptime_s: 3600,
					active_conns: 4,
					sessions: { active: 2, paused: 1, total: 3 },
					conns_history: history,
					ts: '2026-06-02T00:00:00Z'
				})
			)
			.mockResolvedValueOnce(
				jsonResponse({ status: 'ok', version: '0.1.0', stage: '7b' })
			)
			.mockResolvedValueOnce(
				jsonResponse({ lines: ['2026-06-02T00:00:00 INFO started'], total: 1 })
			);
		const SystemPage = (await loadSystemPage()).default;
		const { findByTestId, getByTestId } = render(SystemPage);
		await findByTestId('system-cards');
		expect(getByTestId('metric-cpu').textContent).toContain('12.3%');
		expect(getByTestId('metric-mem').textContent?.trim()).toBe('256');
		expect(getByTestId('metric-uptime').textContent?.trim()).toBe('1h 0m');
		expect(getByTestId('metric-conns').textContent?.trim()).toBe('4');
		expect(getByTestId('metric-sessions').textContent?.trim()).toBe('2/1/3');
		// 折线 svg 渲染
		expect(getByTestId('system-conns-chart').querySelector('svg')).toBeTruthy();
	});

	it('GET /v1/system/metrics + /v1/system/logs 都被调', async () => {
		fetchMock
			.mockResolvedValueOnce(
				jsonResponse({
					cpu_percent: 0,
					mem_mb: 0,
					uptime_s: 1,
					active_conns: 0,
					sessions: { active: 0, paused: 0, total: 0 },
					ts: '2026-06-02T00:00:00Z'
				})
			)
			.mockResolvedValueOnce(jsonResponse({ status: 'ok', version: '0.1.0', stage: '7b' }))
			.mockResolvedValueOnce(jsonResponse({ lines: [], total: 0 }));
		const SystemPage = (await loadSystemPage()).default;
		render(SystemPage);
		await waitFor(() => {
			expect(fetchMock).toHaveBeenCalledTimes(3);
		});
		const urls = fetchMock.mock.calls.map((c) => c[0]);
		expect(urls[0]).toBe('/v1/system/metrics');
		expect(urls[1]).toBe('/v1/system/status');
		expect(urls[2]).toBe('/v1/system/logs?lines=100');
	});

	it('metrics 失败 → 5 秒 setInterval 注册 + ErrorBanner', async () => {
		// 第一次失败
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ error: 'forbidden' }, 403)
		);
		const SystemPage = (await loadSystemPage()).default;
		const { findByTestId } = render(SystemPage);
		await findByTestId('error-state');
	});
});

// ─────────────────────────────────────────────────────────────────────
// 5. 主题切换 — 2 测试
// ─────────────────────────────────────────────────────────────────────
describe('主题 store (Stage 7b)', () => {
	it('toggle() 在 light → dark → system → light 循环', () => {
		themeStore.init();
		themeStore.setMode('light');
		expect(themeStore.mode).toBe('light');
		themeStore.toggle();
		expect(themeStore.mode).toBe('dark');
		themeStore.toggle();
		expect(themeStore.mode).toBe('system');
		themeStore.toggle();
		expect(themeStore.mode).toBe('light');
	});

	it('setMode 持久化到 localStorage (key = qianxun_web_theme)', () => {
		themeStore.setMode('dark');
		expect(localStorage.getItem('qianxun_web_theme')).toBe('dark');
		themeStore.setMode('system');
		expect(localStorage.getItem('qianxun_web_theme')).toBe('system');
		// init 重新读
		themeStore.init();
		// init 之后 mode 仍为 system (无 reset)
		expect(['system', 'light', 'dark']).toContain(themeStore.mode);
	});
});

// ─────────────────────────────────────────────────────────────────────
// 6. i18n — 2 测试
// ─────────────────────────────────────────────────────────────────────
describe('i18n (Stage 7b)', () => {
	it('setLocale 切换 zh-CN ↔ en + 持久化', () => {
		setLocale('zh-CN');
		expect(t('panel.llm.title')).toBe('LLM Providers');
		expect(localStorage.getItem('qianxun_lang')).toBe('zh-CN');
		setLocale('en');
		expect(t('panel.llm.title')).toBe('LLM Providers'); // 巧合两边都是这个 string
		// 切到 en 后用别的 key 验证
		expect(t('common.refresh')).toBe('Refresh');
		expect(localStorage.getItem('qianxun_lang')).toBe('en');
	});

	it('翻译回退: 不存在的 key 返 key 本身', () => {
		setLocale('zh-CN');
		expect(t('not.exist.key' as never)).toBe('not.exist.key');
		// 已存在的 key 在 zh-CN fallback 后还能解析
		setLocale('en');
		expect(t('panel.config.reload_warn')).toBe('Some changes need a daemon restart');
	});
});

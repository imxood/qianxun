// ──────────────────────────────────────────────────────────────────────────
// Stage 10c — 真实 daemon endpoint 集成测试 (mock fetch)
// 跟 docs/30_子项目规划/_shared-contract.md §3.1.1 endpoint 一致
//
// 覆盖 lib/api/ 6 个函数 (Stage 8c 计划):
//   1. listProviders()        → GET /v1/llm/providers
//   2. testProvider(id)       → POST /v1/llm/providers/{id}/test
//   3. reloadSkills()         → POST /v1/skills
//   4. listMcpServers()       → GET /v1/mcp/servers
//   5. listChatSessionsAll()  → GET /v1/chat/sessions (Stage 9c 新增)
//   6. getMetrics()           → GET /v1/system/metrics (Stage 7b)
//
// 不真连 daemon — vi.stubGlobal('fetch') mock 响应, 避免 CI 网络依赖
// 也不测 auth/401 行为 (那是 api.test.ts 覆盖的), 这里只测 happy path
// 验证 client 函数正确解析 daemon 返回的 schema.
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { listProviders, testProvider } from './llm';
import { reloadSkills } from './skills';
import { listMcpServers } from './mcp';
import { listChatSessionsAll } from './chat';
import { getMetrics } from './system';
import { authStore } from '$lib/stores/auth.svelte';

// global fetch mock
const fetchMock = vi.fn();
beforeEach(() => {
	fetchMock.mockReset();
	vi.stubGlobal('fetch', fetchMock);
	// 给一个 fake token, 避免 fetchWithAuth 因为没 token 走 401 路径
	// (实际验证: Authorization 注入是 client.ts 的职责, 我们只关心 endpoint 行为)
	authStore.setToken('integration-test-token');
});
afterEach(() => {
	authStore.clear();
	vi.unstubAllGlobals();
});

function jsonResponse(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json' }
	});
}

describe('integration: 真实 daemon endpoint 契约', () => {
	// 1. listProviders — Stage 7a §4.1
	it('test_llm_providers_list_via_fetch: GET /v1/llm/providers 返 2 个 provider 摘要', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({
				providers: [
					{
						id: 'minimax',
						provider: 'minimax',
						model: 'MiniMax-Text-01',
						has_key: true,
						active: true
					},
					{
						id: 'deepseek',
						provider: 'deepseek',
						model: 'deepseek-v4-flash',
						has_key: true,
						active: false
					}
				]
			})
		);

		const providers = await listProviders();
		expect(providers).toHaveLength(2);
		expect(providers[0]?.id).toBe('minimax');
		expect(providers[0]?.active).toBe(true);
		expect(providers[1]?.id).toBe('deepseek');
		expect(providers[1]?.active).toBe(false);

		// 验证 endpoint URL + method 正确
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/llm/providers');
		expect((init as RequestInit).method).toBe('GET');
	});

	// 2. testProvider — Stage 7a §4.1 (测连接的延迟)
	it('test_llm_test_connection: POST /v1/llm/providers/{id}/test 返 {ok, latency_ms}', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ ok: true, latency_ms: 234, model_version: 'v4-flash' })
		);

		const result = await testProvider('minimax');
		expect(result.ok).toBe(true);
		expect(typeof result.latency_ms).toBe('number');
		expect(result.latency_ms).toBe(234);
		expect(result.model_version).toBe('v4-flash');

		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/llm/providers/minimax/test');
		expect((init as RequestInit).method).toBe('POST');
	});

	// 3. reloadSkills — Stage 7a §4.2
	it('test_skills_reload: POST /v1/skills 返 {status: "reloaded", count: N}', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'reloaded', count: 7 }));

		const result = await reloadSkills();
		expect(result.status).toBe('reloaded');
		expect(result.count).toBe(7);

		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/skills');
		expect((init as RequestInit).method).toBe('POST');
	});

	// 4. listMcpServers — Stage 7a §4.3
	it('test_mcp_servers_list: GET /v1/mcp/servers 返空数组 (无 server 时)', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ servers: [] }));

		const servers = await listMcpServers();
		expect(servers).toEqual([]);

		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/mcp/servers');
		expect((init as RequestInit).method).toBe('GET');
	});

	// 5. listChatSessionsAll — Stage 9c (chat.ts 新增, 不带 filter)
	it('test_chat_sessions_list: GET /v1/chat/sessions 返 {sessions, total}', async () => {
		// listChatSessionsAll 直接返 sessions 数组 (不是带 total 的对象)
		fetchMock.mockResolvedValueOnce(
			jsonResponse({
				sessions: [
					{
						id: 'sess_1',
						model: 'MiniMax-Text-01',
						created_at: '2026-06-01T10:00:00Z',
						last_active: '2026-06-01T10:05:00Z',
						message_count: 4,
						status: 'active',
						token_usage: { input: 100, output: 200, total: 300 }
					},
					{
						id: 'sess_2',
						model: 'MiniMax-Text-01',
						created_at: '2026-06-01T11:00:00Z',
						last_active: '2026-06-01T11:30:00Z',
						message_count: 12,
						status: 'completed',
						token_usage: { input: 500, output: 800, total: 1300 }
					}
				],
				total: 2
			})
		);

		const sessions = await listChatSessionsAll();
		expect(sessions).toHaveLength(2);
		expect(sessions[0]?.id).toBe('sess_1');
		expect(sessions[0]?.status).toBe('active');
		expect(sessions[0]?.token_usage.total).toBe(300);
		expect(sessions[1]?.id).toBe('sess_2');
		expect(sessions[1]?.status).toBe('completed');

		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/chat/sessions');
		// chat.ts 走 fetchWithAuth 直调, 不显式 method='GET' (fetch 默认就是 GET)
		const method = (init as RequestInit).method;
		expect(method === 'GET' || method === undefined).toBe(true);
	});

	// 6. getMetrics — Stage 7b (system 指标: cpu / mem / conns / uptime / sessions 摘要)
	it('test_system_metrics: GET /v1/system/metrics 返 cpu/mem/conns/uptime/sessions', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({
				cpu_percent: 12.5,
				mem_mb: 156,
				uptime_s: 3600,
				active_conns: 2,
				sessions: { active: 1, paused: 0, total: 5 },
				ts: '2026-06-03T00:00:00Z'
			})
		);

		const metrics = await getMetrics();
		// 验证 SystemMetrics schema 字段
		expect(metrics.cpu_percent).toBe(12.5);
		expect(metrics.mem_mb).toBe(156);
		expect(metrics.uptime_s).toBe(3600);
		expect(metrics.active_conns).toBe(2);
		expect(metrics.sessions.active).toBe(1);
		expect(metrics.sessions.total).toBe(5);
		expect(metrics.ts).toBe('2026-06-03T00:00:00Z');

		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/system/metrics');
		expect((init as RequestInit).method).toBe('GET');
	});
});

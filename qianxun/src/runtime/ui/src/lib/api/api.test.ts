// ──────────────────────────────────────────────────────────────────────────
// Stage 7a — fetchWithAuth + 4 个 API 客户端测试
// mock global fetch, 验证请求 URL/headers/body 正确, 401 → 清 token
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
	activateProvider,
	createProvider,
	deleteProvider,
	getProvider,
	listProviders,
	testProvider,
	updateProvider
} from './llm';
import { listSkills, reloadSkills, toggleSkill } from './skills';
import { addMcpServer, deleteMcpServer, listMcpServers, testMcpServer } from './mcp';
import { invokeTool, listTools } from './tools';
import { apiGet, ApiError, AuthRequiredError, fetchWithAuth } from './client';
import { authStore } from '$lib/stores/auth.svelte';

// global fetch mock
const fetchMock = vi.fn();
beforeEach(() => {
	fetchMock.mockReset();
	vi.stubGlobal('fetch', fetchMock);
	authStore.clear();
});
afterEach(() => {
	authStore.clear();
	vi.unstubAllGlobals();
});

function jsonResponse(body: unknown, status = 200, headers: Record<string, string> = {}) {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'content-type': 'application/json', ...headers }
	});
}

describe('fetchWithAuth (client)', () => {
	it('GET 无 token → 不带 Authorization header', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ ok: true }));
		await apiGet('/v1/x');
		const [, init] = fetchMock.mock.calls[0]!;
		expect((init as RequestInit).method).toBe('GET');
		const headers = (init as RequestInit).headers as Record<string, string>;
		expect(headers['Authorization']).toBeUndefined();
	});

	it('GET 有 token → 带 Bearer Authorization', async () => {
		authStore.setToken('test-jwt-token');
		fetchMock.mockResolvedValueOnce(jsonResponse({ ok: true }));
		await apiGet('/v1/x');
		const [, init] = fetchMock.mock.calls[0]!;
		const headers = (init as RequestInit).headers as Record<string, string>;
		expect(headers['Authorization']).toBe('Bearer test-jwt-token');
	});

	it('POST body 自动 JSON 序列化 + Content-Type', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'ok' }));
		await fetchWithAuth('/v1/y', { method: 'POST', body: { foo: 'bar' } });
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/y');
		expect((init as RequestInit).body).toBe('{"foo":"bar"}');
		const headers = (init as RequestInit).headers as Record<string, string>;
		expect(headers['Content-Type']).toBe('application/json');
	});

	it('401 → 抛 AuthRequiredError + 清 token + 派发 qianxun:auth:failed 事件', async () => {
		authStore.setToken('expired-token');
		const dispatchSpy = vi.fn();
		window.addEventListener('qianxun:auth:failed', dispatchSpy);
		fetchMock.mockResolvedValueOnce(new Response(null, { status: 401 }));

		await expect(apiGet('/v1/x')).rejects.toBeInstanceOf(AuthRequiredError);
		expect(authStore.token).toBeNull();
		expect(dispatchSpy).toHaveBeenCalledTimes(1);

		window.removeEventListener('qianxun:auth:failed', dispatchSpy);
	});

	it('非 2xx → 抛 ApiError, 包含 status + body', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ error: 'bad_request', message: 'oops' }, 400)
		);
		try {
			await apiGet('/v1/x');
			expect.fail('should have thrown');
		} catch (e) {
			expect(e).toBeInstanceOf(ApiError);
			const err = e as ApiError;
			expect(err.status).toBe(400);
			expect((err.body as { error: string }).error).toBe('bad_request');
		}
	});

	it('SSR 环境 (window=undefined) → 抛错', async () => {
		const origWindow = globalThis.window;
		// 模拟 SSR
		(globalThis as unknown as { window: undefined }).window = undefined;
		try {
			await expect(apiGet('/v1/x')).rejects.toThrow(/browser-only/);
		} finally {
			(globalThis as unknown as { window: typeof origWindow }).window = origWindow;
		}
	});
});

describe('LLM API client', () => {
	const sample: import('$lib/types/api').LlmProviderSummary = {
		id: 'deepseek-main',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		has_key: true,
		active: true
	};

	it('listProviders → GET /v1/llm/providers, 返 providers[]', async () => {
		authStore.setToken('t');
		fetchMock.mockResolvedValueOnce(jsonResponse({ providers: [sample] }));
		const r = await listProviders();
		expect(r).toHaveLength(1);
		expect(r[0]?.id).toBe('deepseek-main');
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/llm/providers');
		expect((init as RequestInit).method).toBe('GET');
	});

	it('getProvider → GET /v1/llm/providers/{id}', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ provider: { ...sample, has_key: true } }));
		const p = await getProvider('deepseek-main');
		expect(p.id).toBe('deepseek-main');
		expect(fetchMock.mock.calls[0]![0]).toBe('/v1/llm/providers/deepseek-main');
	});

	it('createProvider → POST + body', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'added' }));
		await createProvider({
			id: 'x',
			provider: 'anthropic',
			model: 'claude-sonnet-4',
			api_key: 'sk-test'
		});
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/llm/providers');
		expect((init as RequestInit).method).toBe('POST');
		expect(JSON.parse((init as RequestInit).body as string)).toMatchObject({
			id: 'x',
			provider: 'anthropic'
		});
	});

	it('updateProvider → PUT + URL encode', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'updated' }));
		await updateProvider('prov with space', { id: 'p', provider: 'p', model: 'm' });
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/llm/providers/prov%20with%20space');
		expect((init as RequestInit).method).toBe('PUT');
	});

	it('deleteProvider → DELETE', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'deleted' }));
		await deleteProvider('x');
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/llm/providers/x');
		expect((init as RequestInit).method).toBe('DELETE');
	});

	it('activateProvider → POST /activate', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'active' }));
		await activateProvider('x');
		expect(fetchMock.mock.calls[0]![0]).toBe('/v1/llm/providers/x/activate');
	});

	it('testProvider → POST /test, 返 ok + latency', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ ok: true, latency_ms: 234, model_version: 'v4' })
		);
		const r = await testProvider('x');
		expect(r.ok).toBe(true);
		expect(r.latency_ms).toBe(234);
	});
});

describe('Skills API client', () => {
	it('listSkills → 返 skills[]', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ skills: [{ name: 'demo', description: 'd', enabled: true, path: '/p' }] })
		);
		const r = await listSkills();
		expect(r).toHaveLength(1);
		expect(r[0]?.name).toBe('demo');
	});

	it('reloadSkills → POST /v1/skills', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'reloaded', count: 5 }));
		const r = await reloadSkills();
		expect(r.count).toBe(5);
		expect((fetchMock.mock.calls[0]![1] as RequestInit).method).toBe('POST');
	});

	it('toggleSkill → POST /v1/skills/{name}/toggle', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'disabled' }));
		const r = await toggleSkill('web-search');
		expect(r.status).toBe('disabled');
		expect(fetchMock.mock.calls[0]![0]).toBe('/v1/skills/web-search/toggle');
	});
});

describe('MCP API client', () => {
	const sample: import('$lib/types/api').McpServerSummary = {
		id: 'fs',
		name: 'fs',
		transport: 'stdio',
		command_or_url: 'npx ...',
		connected: true,
		tool_count: 3
	};

	it('listMcpServers → 返 servers[]', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ servers: [sample] }));
		const r = await listMcpServers();
		expect(r).toHaveLength(1);
		expect(r[0]?.id).toBe('fs');
	});

	it('addMcpServer → POST body', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'added' }));
		await addMcpServer({
			id: 'http-1',
			name: 'http-1',
			transport: 'http',
			command_or_url: 'http://localhost:3000'
		});
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/mcp/servers');
		expect((init as RequestInit).method).toBe('POST');
	});

	it('deleteMcpServer → DELETE', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ status: 'deleted' }));
		await deleteMcpServer('fs');
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/mcp/servers/fs');
		expect((init as RequestInit).method).toBe('DELETE');
	});

	it('testMcpServer → POST /test, 返 ok + tools[]', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ ok: true, tools: [{ name: 'read_file' }] })
		);
		const r = await testMcpServer('fs');
		expect(r.ok).toBe(true);
		expect(r.tools?.[0]?.name).toBe('read_file');
	});
});

describe('Tools API client', () => {
	it('listTools → 返 tools[]', async () => {
		fetchMock.mockResolvedValueOnce(
			jsonResponse({ tools: [{ name: 'read_text_file', description: 'd', input_schema: {} }] })
		);
		const r = await listTools();
		expect(r).toHaveLength(1);
		expect(r[0]?.name).toBe('read_text_file');
	});

	it('invokeTool → POST /v1/tools/{name}/invoke + body.arguments', async () => {
		fetchMock.mockResolvedValueOnce(jsonResponse({ output: 'hi', elapsed_ms: 50 }));
		const r = await invokeTool('read_text_file', { path: '/tmp' });
		expect(r.output).toBe('hi');
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/tools/read_text_file/invoke');
		expect(JSON.parse((init as RequestInit).body as string)).toEqual({
			arguments: { path: '/tmp' }
		});
	});
});

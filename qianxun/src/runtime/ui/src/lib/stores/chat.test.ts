// ──────────────────────────────────────────────────────────────────────────
// Stage 9c — Chat Store 单元测试
// 跟 docs/30_子项目规划/01b-daemon-web-console.md §10 Chat 视图 一致
//
// 覆盖:
//   - sendPrompt 触发 SSE 流并累积 text_delta 到 assistant message
//   - cancel 关闭流 (AbortController.abort)
//   - loadSessions 从 GET /v1/chat/sessions 拉取列表
//   - selectSession 切换 active session
//   - deleteSession 从列表移除
//   - createNewSession 调 POST /v1/chat/session
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { chatStore } from './chat.svelte';
import { authStore } from './auth.svelte';
import { encodeSseEvents } from './__test-helpers__/sse-encode';

// global fetch mock
const fetchMock = vi.fn();
beforeEach(() => {
	fetchMock.mockReset();
	vi.stubGlobal('fetch', fetchMock);
	authStore.clear();
	chatStore.reset();
});
afterEach(() => {
	authStore.clear();
	chatStore.reset();
	vi.unstubAllGlobals();
});

function sseResponse(events: object[]): Response {
	return new Response(encodeSseEvents(events), {
		status: 200,
		headers: { 'content-type': 'text/event-stream' }
	});
}

describe('chatStore (Stage 9c)', () => {
	it('loadSessions: 调 GET /v1/chat/sessions, 写入 sessions[]', async () => {
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({
				sessions: [
					{
						id: 'sess_1',
						model: 'MiniMax-M3',
						created_at: '2026-06-03T01:00:00Z',
						last_active: '2026-06-03T05:00:00Z',
						message_count: 3,
						status: 'active',
						token_usage: { input: 100, output: 50, total: 150 }
					}
				],
				total: 1
			}),
			text: async () => ''
		});

		await chatStore.loadSessions();
		expect(chatStore.sessions).toHaveLength(1);
		expect(chatStore.sessions[0]?.id).toBe('sess_1');
		expect(chatStore.sessions[0]?.model).toBe('MiniMax-M3');
	});

	it('sendPrompt: 累积 text_delta 到 assistant message', async () => {
		authStore.setToken('test-token');
		// 1) 预 create session (selectSession 不自动创建, 我们手动 mock create)
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_abc' }),
			text: async () => ''
		});
		await chatStore.createNewSession();
		expect(chatStore.activeSessionId).toBe('sess_abc');

		// 2) sendPrompt → mock SSE 流
		fetchMock.mockResolvedValueOnce(
			sseResponse([
				{ type: 'message_start', session_id: 'sess_abc', model: 'm', max_tokens: 1024 },
				{ type: 'content_block_start', index: 0, block_type: 'text' },
				{ type: 'text_delta', index: 0, text: 'Hello' },
				{ type: 'text_delta', index: 0, text: ', world' },
				{ type: 'content_block_stop', index: 0 },
				{
					type: 'usage',
					input_tokens: 10,
					output_tokens: 5,
					cache_creation_input_tokens: 0,
					cache_read_input_tokens: 0
				},
				{ type: 'message_delta', stop_reason: 'end_turn' },
				{ type: 'message_stop' }
			])
		);

		await chatStore.sendPrompt('hi');
		// 等流消费完
		await new Promise((r) => setTimeout(r, 0));

		// user + assistant 2 个 message
		expect(chatStore.messages).toHaveLength(2);
		const assistant = chatStore.messages[1];
		expect(assistant?.role).toBe('assistant');
		expect(assistant?.content[0]?.type).toBe('text');
		if (assistant?.content[0]?.type === 'text') {
			expect(assistant.content[0].text).toBe('Hello, world');
		}
		// usage 已记录
		expect(chatStore.usage$).toEqual({ input: 10, output: 5 });
		expect(chatStore.isStreaming).toBe(false);
	});

	it('cancel: abort 正在跑的流, 设置 lastError', async () => {
		authStore.setToken('test-token');
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_x' }),
			text: async () => ''
		});
		await chatStore.createNewSession();

		// 模拟 fetch 返回一个会响应 abort signal 的 stream
		let streamController: ReadableStreamDefaultController<Uint8Array> | null = null;
		const abortableStream = new ReadableStream<Uint8Array>({
			start(controller) {
				streamController = controller;
			}
		});
		// 包装: 在 abort 时关闭 stream
		const origAdd = abortableStream;
		void origAdd;
		fetchMock.mockImplementationOnce((_url: string, init?: RequestInit) => {
			if (init?.signal) {
				init.signal.addEventListener('abort', () => {
					streamController?.error(new DOMException('Aborted', 'AbortError'));
				});
			}
			return Promise.resolve(
				new Response(abortableStream, {
					status: 200,
					headers: { 'content-type': 'text/event-stream' }
				})
			);
		});

		// fire-and-forget
		const sendPromise = chatStore.sendPrompt('test');
		// 等 fetch 被调 + isStreaming 翻 true
		await new Promise((r) => setTimeout(r, 0));
		expect(chatStore.isStreaming).toBe(true);

		// cancel — 应触发 abort signal → stream error
		chatStore.cancel();
		// 等流结束
		await sendPromise.catch(() => undefined);
		await new Promise((r) => setTimeout(r, 0));

		expect(chatStore.isStreaming).toBe(false);
		expect(chatStore.lastError).toBe('已取消');
	});

	it('createNewSession: 调 POST /v1/chat/session, 插入到 sessions 顶部', async () => {
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_new' }),
			text: async () => ''
		});
		const id = await chatStore.createNewSession();
		expect(id).toBe('sess_new');
		expect(chatStore.activeSessionId).toBe('sess_new');
		expect(chatStore.sessions).toHaveLength(1);
		expect(chatStore.sessions[0]?.id).toBe('sess_new');
		expect(fetchMock).toHaveBeenCalledTimes(1);
		const [url, init] = fetchMock.mock.calls[0]!;
		expect(url).toBe('/v1/chat/session');
		expect((init as RequestInit).method).toBe('POST');
	});

	it('deleteSession: 从 sessions 移除 + 清 active', async () => {
		// 准备 2 个 session
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_a' }),
			text: async () => ''
		});
		await chatStore.createNewSession();
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_b' }),
			text: async () => ''
		});
		await chatStore.createNewSession();
		expect(chatStore.sessions).toHaveLength(2);

		// delete sess_b
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ status: 'deleted' }),
			text: async () => ''
		});
		await chatStore.deleteSession('sess_b');
		expect(chatStore.sessions.find((s) => s.id === 'sess_b')).toBeUndefined();
		expect(chatStore.activeSessionId).toBeNull();
	});

	it('selectSession: 切换 activeSessionId + 清空 messages', () => {
		chatStore.selectSession('sess_xyz');
		expect(chatStore.activeSessionId).toBe('sess_xyz');
		expect(chatStore.messages).toEqual([]);
	});

	it('sendPrompt: tool_use_complete → 创建 tool_use 块', async () => {
		authStore.setToken('test-token');
		// 1) create session
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_tool' }),
			text: async () => ''
		});
		await chatStore.createNewSession();

		// 2) mock tool_use 流
		fetchMock.mockResolvedValueOnce(
			sseResponse([
				{ type: 'message_start', session_id: 'sess_tool', model: 'm', max_tokens: 1024 },
				{ type: 'content_block_start', index: 0, block_type: 'tool_use' },
				{
					type: 'tool_use_complete',
					index: 0,
					id: 'toolu_1',
					name: 'read_file',
					arguments: { path: '/tmp/a.txt' }
				},
				{ type: 'content_block_stop', index: 0 },
				{ type: 'message_stop' }
			])
		);

		await chatStore.sendPrompt('read');
		await new Promise((r) => setTimeout(r, 0));

		const assistant = chatStore.messages[1];
		expect(assistant?.content[0]?.type).toBe('tool_use');
		if (assistant?.content[0]?.type === 'tool_use') {
			expect(assistant.content[0].name).toBe('read_file');
			expect(assistant.content[0].input).toEqual({ path: '/tmp/a.txt' });
		}
		expect(chatStore.isStreaming).toBe(false);
	});
});

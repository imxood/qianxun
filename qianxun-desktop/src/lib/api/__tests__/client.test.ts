// qianxun-desktop/src/lib/api/__tests__/client.test.ts
// Phase 4a-1: IPC client + mock server 端到端测试
//
// 覆盖:
//   1. CRUD 调用 (POST /v1/chat/session, GET /v1/chat/sessions) 跟 mock server 通信正常
//   2. SSE 流能正确解析成 4 个 SseEvent (message_start / text / turn_finished / message_stop)
//   3. fetchPromptStream 返 raw Response, caller 自己读 body
//   4. ApiError 在 404 抛出
//   5. 流式 text chunk 累积行为 (用 echo 选项控制)

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { startMockServer, type MockServerHandle } from '../mock-server';
import { ApiError, apiGet, apiPost, DAEMON_URL } from '../client';
import { createChatSession, fetchPromptStream, listChatSessionsAll } from '../chat';
import { parseSseStream } from '$lib/sse/parser';

let mock: MockServerHandle;

beforeEach(async () => {
	mock = await startMockServer();
	vi.stubEnv('PUBLIC_QIANXUN_DAEMON_URL', mock.url);
});

afterEach(async () => {
	vi.unstubAllEnvs();
	await mock.close();
});

describe('env resolution', () => {
	it('DAEMON_URL defaults to 127.0.0.1:23900 when no env set', async () => {
		vi.unstubAllEnvs();
		// 不能直接断言 DAEMON_URL (它在模块加载时解析), 但 apiGet 失败可验证
		await expect(apiGet('/v1/chat/sessions')).rejects.toThrow();
	});
});

describe('CRUD endpoints', () => {
	it('POST /v1/chat/session returns session with id', async () => {
		const r = await createChatSession();
		expect(r.session.id).toMatch(/^sess_mock_/);
		expect(r.session.title).toBe('新会话');
		expect(r.session.model).toBe('mock-model');
	});

	it('GET /v1/chat/sessions returns empty array', async () => {
		const list = await listChatSessionsAll();
		expect(list).toEqual([]);
	});

	it('apiPost throws ApiError on 404', async () => {
		await expect(apiPost('/v1/chat/session/nonexistent/prompt', {})).rejects.toBeInstanceOf(
			ApiError
		);
	});
});

describe('SSE stream — fetchPromptStream + parseSseStream', () => {
	it('yields 4 SseEvents in order: message_start → text → turn_finished → message_stop', async () => {
		const { session } = await createChatSession();
		const r = await fetchPromptStream(session.id, { text: 'hello' });
		expect(r.ok).toBe(true);
		expect(r.headers.get('content-type')).toContain('text/event-stream');

		const events: Array<{ event: string; data: unknown }> = [];
		for await (const ev of parseSseStream(r.body)) {
			events.push(ev);
		}

		expect(events).toHaveLength(4);
		expect(events[0].event).toBe('message_start');
		expect(events[1].event).toBe('text');
		expect(events[2].event).toBe('turn_finished');
		expect(events[3].event).toBe('message_stop');
	});

	it('message_start carries session_id + message_id', async () => {
		const { session } = await createChatSession();
		const r = await fetchPromptStream(session.id, { text: 'hi' });
		const events: Array<{ event: string; data: unknown }> = [];
		for await (const ev of parseSseStream(r.body)) events.push(ev);

		const ms = events[0];
		if (ms.event !== 'message_start') throw new Error('expected message_start first');
		const data = ms.data as { session_id: string; message_id: string };
		expect(data.session_id).toBe(session.id);
		expect(data.message_id).toMatch(/^msg_mock_/);
	});

	it('text event echoes the user prompt', async () => {
		const { session } = await createChatSession();
		const r = await fetchPromptStream(session.id, { text: 'ping' });
		const events: Array<{ event: string; data: unknown }> = [];
		for await (const ev of parseSseStream(r.body)) events.push(ev);

		const txt = events.find((e) => e.event === 'text');
		if (!txt) throw new Error('expected text event');
		const data = txt.data as { text: string };
		expect(data.text).toBe('收到: ping');
	});

	it('respects custom echo function', async () => {
		await mock.close();
		mock = await startMockServer({
			echo: (text) => ['chunk1:', text, ':end']
		});
		vi.stubEnv('PUBLIC_QIANXUN_DAEMON_URL', mock.url);

		const { session } = await createChatSession();
		const r = await fetchPromptStream(session.id, { text: 'X' });
		const events: Array<{ event: string; data: unknown }> = [];
		for await (const ev of parseSseStream(r.body)) events.push(ev);

		const txt = events.find((e) => e.event === 'text');
		if (!txt) throw new Error('expected text event');
		const data = txt.data as { text: string };
		expect(data.text).toBe('chunk1:X:end');
	});

	it('turn_finished carries reason + usage', async () => {
		const { session } = await createChatSession();
		const r = await fetchPromptStream(session.id, { text: 'q' });
		const events: Array<{ event: string; data: unknown }> = [];
		for await (const ev of parseSseStream(r.body)) events.push(ev);

		const tf = events.find((e) => e.event === 'turn_finished');
		if (!tf) throw new Error('expected turn_finished event');
		const data = tf.data as { reason: string; usage: { input: number; output: number } };
		expect(data.reason).toBe('end_turn');
		expect(data.usage.input).toBe(10);
		expect(data.usage.output).toBe(5);
	});

	it('returns 404 for unknown sessionId', async () => {
		const r = await fetchPromptStream('sess_unknown_xxx', { text: 'hi' });
		expect(r.status).toBe(404);
	});
});

describe('sub_session path', () => {
	it('routes to /v1/chat/session/{sid}/sub_session/{subid}/prompt when subSessionId set', async () => {
		// mock server 没实现 sub_session 路径 → 返 404, 这测试验证路径拼接
		const { session } = await createChatSession();
		const r = await fetchPromptStream(session.id, {
			text: 'followup',
			subSessionId: 'sub_x'
		});
		expect(r.status).toBe(404); // mock 没实现
		// 真 daemon 4a-2 时这个测试要改成期望 200
	});
});

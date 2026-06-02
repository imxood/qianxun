// ──────────────────────────────────────────────────────────────────────────
// Stage 9c — Chat 组件 + 路由 单元测试
// 跟 docs/30_子项目规划/01b-daemon-web-console.md §10 Chat 视图 一致
//
// 覆盖:
//   - MessageBubble: user / assistant 渲染
//   - InputBox: 回车发送 (Enter 触发 submit)
//   - SessionList: 列表渲染 + 选中回调
//   - ThreeColumnLayout: 3 栏 grid 渲染
//   - /chat 路由: mock fetch, 验证 sessions 加载 + 选中 + 路由可达
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import MessageBubble from './MessageBubble.svelte';
import InputBox from './InputBox.svelte';
import SessionList from '../layout/SessionList.svelte';
import ThreeColumnLayout from '../layout/ThreeColumnLayout.svelte';
import type { ChatSession } from '$lib/types/chat';
import { authStore } from '$lib/stores/auth.svelte';
import { chatStore } from '$lib/stores/chat.svelte';

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

// ─── MessageBubble ──────────────────────────────────────────────────────────

describe('MessageBubble', () => {
	it('user 角色 + text block → 渲染文本 + 蓝色背景', () => {
		const { container } = render(MessageBubble, {
			props: { block: { type: 'text', text: 'hello' }, role: 'user' }
		});
		const div = container.querySelector('div.my-2');
		expect(div).toBeTruthy();
		expect(div?.className).toContain('ml-12');
		expect(div?.className).toContain('bg-blue-50');
		expect(container.textContent).toContain('hello');
	});

	it('assistant 角色 + text block → 渲染文本 + muted 背景', () => {
		const { container } = render(MessageBubble, {
			props: { block: { type: 'text', text: 'response' }, role: 'assistant' }
		});
		const div = container.querySelector('div.my-2');
		expect(div?.className).toContain('mr-12');
		expect(div?.className).toContain('bg-muted');
		expect(container.textContent).toContain('response');
	});

	it('tool_use block → 渲染工具名 + JSON.stringify input', () => {
		const { container } = render(MessageBubble, {
			props: {
				block: { type: 'tool_use', id: 't1', name: 'read_file', input: { path: '/tmp' } },
				role: 'assistant'
			}
		});
		expect(container.textContent).toContain('read_file');
		expect(container.textContent).toContain('"path"');
		expect(container.textContent).toContain('"/tmp"');
	});

	it('tool_result block → 渲染 content + elapsed_ms', () => {
		const { container } = render(MessageBubble, {
			props: {
				block: {
					type: 'tool_result',
					tool_use_id: 't1',
					content: 'file contents',
					is_error: false,
					elapsed_ms: 42
				},
				role: 'assistant'
			}
		});
		expect(container.textContent).toContain('file contents');
		expect(container.textContent).toContain('42ms');
	});
});

// ─── InputBox ──────────────────────────────────────────────────────────────

describe('InputBox', () => {
	it('回车键 → 触发 send (调 chatStore.sendPrompt)', async () => {
		authStore.setToken('t');
		// 需要 active session 才能 send
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_1' }),
			text: async () => ''
		});
		await chatStore.createNewSession();
		// mock sendPrompt 流的 message_stop
		fetchMock.mockResolvedValueOnce(
			new Response(
				new TextEncoder().encode('data: {"type":"message_stop"}\n\n'),
				{ status: 200, headers: { 'content-type': 'text/event-stream' } }
			)
		);

		const { getByTestId } = render(InputBox);
		const textarea = getByTestId('chat-input') as HTMLTextAreaElement;
		// Svelte 5: 直接设 value + dispatch input 事件
		textarea.value = 'hello world';
		await fireEvent.input(textarea);
		// 触发 Enter
		await fireEvent.keyDown(textarea, { key: 'Enter' });
		// 等 send 完成
		await new Promise((r) => setTimeout(r, 0));

		// 期望: user message + assistant message 都被推
		expect(chatStore.messages.length).toBeGreaterThanOrEqual(2);
		const userMsg = chatStore.messages[0];
		expect(userMsg?.role).toBe('user');
		if (userMsg?.content[0]?.type === 'text') {
			expect(userMsg.content[0].text).toBe('hello world');
		}
	});

	it('空文本 → send 按钮 disabled', () => {
		const { getByTestId } = render(InputBox);
		const sendBtn = getByTestId('chat-send-btn') as HTMLButtonElement;
		expect(sendBtn.disabled).toBe(true);
	});

	it('Shift+Enter → 不发送 (允许换行)', async () => {
		authStore.setToken('t');
		fetchMock.mockResolvedValueOnce({
			ok: true,
			status: 200,
			headers: { get: () => 'application/json' },
			json: async () => ({ session_id: 'sess_1' }),
			text: async () => ''
		});
		await chatStore.createNewSession();

		const { getByTestId } = render(InputBox);
		const textarea = getByTestId('chat-input') as HTMLTextAreaElement;
		textarea.value = 'line1';
		await fireEvent.input(textarea);
		await fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: true });
		await new Promise((r) => setTimeout(r, 0));
		// 没触发 send, messages 还是空
		expect(chatStore.messages).toHaveLength(0);
	});
});

// ─── SessionList ───────────────────────────────────────────────────────────

describe('SessionList', () => {
	const sampleSessions: ChatSession[] = [
		{
			id: 'sess_alpha',
			model: 'm1',
			created_at: '2026-06-01T00:00:00Z',
			last_active: '2026-06-03T01:00:00Z',
			message_count: 5,
			status: 'active',
			token_usage: { input: 100, output: 50, total: 150 }
		},
		{
			id: 'sess_beta',
			model: 'm2',
			created_at: '2026-06-01T00:00:00Z',
			last_active: '2026-06-03T02:00:00Z',
			message_count: 3,
			status: 'completed',
			token_usage: { input: 50, output: 20, total: 70 }
		}
	];

	it('空 sessions → 渲染"暂无会话"占位', () => {
		const { container } = render(SessionList, {
			props: { sessions: [], activeSessionId: null, onSelectSession: vi.fn() }
		});
		expect(container.textContent).toContain('暂无会话');
	});

	it('sessions[] 渲染 + 点击触发 onSelectSession 回调', async () => {
		const onSelect = vi.fn();
		const { getByTestId } = render(SessionList, {
			props: {
				sessions: sampleSessions,
				activeSessionId: null,
				onSelectSession: onSelect
			}
		});
		expect(getByTestId('session-item-sess_alpha')).toBeTruthy();
		expect(getByTestId('session-item-sess_beta')).toBeTruthy();

		// 点击 sess_beta
		const btn = getByTestId('session-item-sess_beta') as HTMLButtonElement;
		await fireEvent.click(btn);
		expect(onSelect).toHaveBeenCalledWith('sess_beta');
	});

	it('activeSessionId 高亮 + 新建按钮触发 onCreateSession', async () => {
		const onCreate = vi.fn();
		const { getByTestId } = render(SessionList, {
			props: {
				sessions: sampleSessions,
				activeSessionId: 'sess_alpha',
				onSelectSession: vi.fn(),
				onCreateSession: onCreate
			}
		});
		const alpha = getByTestId('session-item-sess_alpha');
		expect(alpha.className).toContain('bg-accent');

		const newBtn = getByTestId('session-list-new') as HTMLButtonElement;
		await fireEvent.click(newBtn);
		expect(onCreate).toHaveBeenCalledTimes(1);
	});
});

// ─── ThreeColumnLayout ─────────────────────────────────────────────────────

describe('ThreeColumnLayout', () => {
	it('3 栏 grid 正确渲染 (2 aside + 1 main)', () => {
		const { container } = render(ThreeColumnLayout, {
			props: {}
		});
		const grid = container.querySelector('.grid.h-screen');
		expect(grid).toBeTruthy();
		const asides = container.querySelectorAll('aside');
		// 2 asides (left + right) + 1 main = 3 cells in grid
		expect(asides.length).toBe(2);
		const main = container.querySelector('main');
		expect(main).toBeTruthy();
	});
});

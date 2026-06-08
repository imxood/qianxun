// qianxun-desktop/src/lib/stores/chat.svelte.ts
// Chat 流 (消息 + mock 流式输出)

import { sessionStore } from './session.svelte';
import { subSessionStore } from './sub_session.svelte';
import { planStore } from './plan.svelte';
import { uiStore } from './ui.svelte';
import { streamMock } from '$lib/utils/stream';
import type { Message, PlanContract } from '$lib/types/entity';

function createChatStore() {
	// 当前流式光标
	const streaming = $state({ message_id: null as string | null });

	function genId() {
		return `msg_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
	}

	async function send(session_id: string, userMessage: string) {
		const now = new Date().toISOString();
		// 1. 追加 user 消息
		const userMsg: Message = {
			id: genId(),
			session_id,
			sub_session_id: null,
			role: 'user',
			content: userMessage,
			created_at: now,
		};
		sessionStore.appendMessage(session_id, userMsg);

		// 2. 更新 session title (首条消息)
		const session = sessionStore.get(session_id);
		if (session && session.title === '新会话') {
			session.title = userMessage.slice(0, 30) + (userMessage.length > 30 ? '…' : '');
		}

		// 3. 模拟主 Agent 思考
		await sleep(800);

		// 4. 决定要不要拉 Plan (简单 mock: 消息含 "JWT" / "认证" / "实现" / "重构" 关键词就拉)
		const needsPlan = /jwt|认证|登录|实现|重构|调研|写测试|修 bug/i.test(userMessage);
		if (needsPlan && !planStore.bySession(session_id).some((p) => p.status === 'Running')) {
			const contract: PlanContract = {
				name: userMessage.slice(0, 20),
				description: userMessage,
				timeout_ms: 1800000,
				tasks: [
					{
						id: 'task_a',
						title: '分析需求',
						prompt: userMessage,
						assigned_to: 'coder',
						verified_by: 'code-reviewer',
						verify_prompt: '',
						depends_on: [],
						timeout_ms: 600000,
					},
					{
						id: 'task_b',
						title: '实现',
						prompt: '基于分析结果实现',
						assigned_to: 'coder',
						verified_by: 'code-reviewer',
						verify_prompt: '',
						depends_on: ['task_a'],
						timeout_ms: 900000,
					},
					{
						id: 'task_c',
						title: '测试',
						prompt: '写测试',
						assigned_to: 'tester',
						verified_by: 'code-reviewer',
						verify_prompt: '',
						depends_on: ['task_b'],
						timeout_ms: 900000,
					},
				],
			};
			const plan = planStore.create({ session_id, contract });
			// 追加一个 assistant 消息 (带 plan_ref)
			const planMsg: Message = {
				id: genId(),
				session_id,
				sub_session_id: null,
				role: 'assistant',
				content: '',
				plan_ref: plan.id,
				created_at: new Date().toISOString(),
			};
			sessionStore.appendMessage(session_id, planMsg);
			return;
		}

		// 5. 普通响应: 走流式 mock
		const assistantMsg: Message = {
			id: genId(),
			session_id,
			sub_session_id: null,
			role: 'assistant',
			content: '',
			created_at: new Date().toISOString(),
			streaming: true,
		};
		sessionStore.appendMessage(session_id, assistantMsg);
		streaming.message_id = assistantMsg.id;

		await streamMock(assistantMsg, userMessage, (chunk) => {
			// 更新 content
			const list = sessionStore.getMessages(session_id);
			const m = list.find((x) => x.id === assistantMsg.id);
			if (m) m.content = chunk;
		});

		assistantMsg.streaming = false;
		streaming.message_id = null;
	}

	// 追问: sub_session 状态下发消息
	// - sub.status === 'Active'  → 标记 'task' (原始任务流), 走流式响应
	// - sub.status !== 'Active'  → 标记 'followup' (追问), 走流式响应, 不触发执行
	async function sendToSubSession(sub_id: string, userMessage: string) {
		const sub = subSessionStore.get(sub_id);
		if (!sub) return;
		const isFollowup = !subSessionStore.isActive(sub);
		const now = new Date().toISOString();

		// 1. 追加 user 消息
		const userMsg: Message = {
			id: genId(),
			session_id: null,
			sub_session_id: sub_id,
			role: 'user',
			content: userMessage,
			kind: isFollowup ? 'followup' : 'task',
			created_at: now,
		};
		subSessionStore.appendMessage(sub_id, userMsg);

		// 2. 模拟思考
		await sleep(isFollowup ? 400 : 600);

		// 3. 流式响应
		const assistantMsg: Message = {
			id: genId(),
			session_id: null,
			sub_session_id: sub_id,
			role: 'assistant',
			content: '',
			kind: isFollowup ? 'followup' : 'task',
			created_at: new Date().toISOString(),
			streaming: true,
		};
		subSessionStore.appendMessage(sub_id, assistantMsg);
		streaming.message_id = assistantMsg.id;

		// 追问的回应更短、更简洁 (mock 阶段用 isFollowup 区分)
		await streamMock(assistantMsg, userMessage, (chunk) => {
			const list = subSessionStore.messagesOf(sub_id);
			const m = list.find((x) => x.id === assistantMsg.id);
			if (m) m.content = chunk;
		});

		assistantMsg.streaming = false;
		streaming.message_id = null;
	}

	function sleep(ms: number) {
		return new Promise((r) => setTimeout(r, ms));
	}

	return {
		get streaming() {
			return streaming;
		},
		send,
		sendToSubSession,
	};
}

export const chatStore = createChatStore();

// qianxun-desktop/src/lib/stores/chat.svelte.ts
// Chat 流 (消息 + 真后端 invoke)
//
// Stage 4a (sub-task #4): 切真后端 invoke.
//   - 删 streamMock (mock 阶段 helper) + sleep(800) 等待
//   - send() 调 invoke('send_message') 拿 SendResponse, 监听 'session_event' 流式事件
//   - sendToSubSession() 暂 noop (后端 RuntimeApi 没 sub_session 消息方法, 留后续 sub-task)
//   - Plan 触发: 保留前端关键词判断 (跟 mock 阶段一致), 调 planStore.create() 走 invoke
//
// 流式架构 (1 个全局 listener, N 个 in-flight stream):
//   - onSessionEvent() 全局注册一次, 按 session_id 分发到对应的 MessageStreamState
//   - 每个 send() 创建独立 stream state, 用 chat-stream.ts::applyEvent 处理事件
//   - stream state 直接改 sessionStore 里 message.content (Svelte 5 响应式)
//
// 业务约束:
//   - LLM 应该自己决定是否创建 plan (通过 tool call), 但当前后端 RuntimeApi 没 plan 决策
//     → 保留前端关键词判断, TODO 后续移到后端
//   - sendToSubSession 后端没对应方法, 暂 noop + 弹 error toast
//
// 关联:
//   - $lib/ipc/runtime.ts (sendMessage / onSessionEvent invoke)
//   - $lib/stores/chat-stream.ts (SseEvent → Message 状态机)
//   - $lib/stores/plan.svelte.ts (plan 触发后调 planStore.create)

import { sendMessage, onSessionEvent, type SessionEventPayload } from '$lib/ipc/runtime';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { newStreamState, applyEvent, type MessageStreamState } from './chat-stream';
import { sessionStore } from './session.svelte';
import { subSessionStore } from './sub_session.svelte';
import { planStore } from './plan.svelte';
import { uiStore } from './ui.svelte';
import type { Message, PlanContract } from '$lib/types/entity';

function createChatStore() {
	// 当前流式光标
	const streaming = $state({ message_id: null as string | null });

	// Per-session 流式 state (key = session_id, value = MessageStreamState)
	const streams = new Map<string, MessageStreamState>();

	// 全局 listener unlisten handle
	let unlisten: UnlistenFn | null = null;
	let listenerInitialized = false;

	/// 初始化全局 session_event listener. 重复调用安全.
	/// 调用方: +page.svelte / +layout.svelte 的 onMount.
	async function init() {
		if (listenerInitialized) return;
		listenerInitialized = true;
		unlisten = await onSessionEvent(handleSessionEvent);
	}

	function handleSessionEvent(payload: SessionEventPayload) {
		const state = streams.get(payload.session_id);
		if (!state) return; // 没有 in-flight stream, 忽略
		applyEvent(state, payload.event);
		// 收尾清理
		if (state.finished) {
			finalizeStream(payload.session_id, state);
		}
	}

	function finalizeStream(sessionId: string, state: MessageStreamState) {
		// 同步到 sessionStore 的 message (content / streaming = false)
		const list = sessionStore.getMessages(sessionId);
		const m = list.find((x) => x.id === state.messageId);
		if (m) {
			m.content = state.content;
			m.streaming = false;
		}
		streams.delete(sessionId);
		if (streaming.message_id === state.messageId) {
			streaming.message_id = null;
		}
	}

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
		if (session && session.title === session.id.slice(0, 20)) {
			// 仅当 title 还是兜底 (id slice) 才覆盖, 避免覆盖已自定义的 title
			session.title = userMessage.slice(0, 30) + (userMessage.length > 30 ? '…' : '');
		}

		// 3. Plan 触发判断 (前端关键词, TODO 后续移到后端 RuntimeApi 决策)
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
			const plan = await planStore.create({ session_id, contract });
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

		// 4. 普通响应: 调 sendMessage invoke + 起流式 state
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

		// 5. 创 stream state, onUpdate 同步到 sessionStore.message.content
		const state = newStreamState(assistantMsg.id, () => {
			const list = sessionStore.getMessages(session_id);
			const m = list.find((x) => x.id === assistantMsg.id);
			if (m) m.content = state.content;
		});
		streams.set(session_id, state);

		// 6. 调 invoke
		try {
			await sendMessage(session_id, {
				messages: [{ role: 'user', content: userMessage }],
			});
			// 立即返, 流走 session_event 异步推
		} catch (e) {
			// 失败时本地标记
			state.content = `[错误] ${(e as Error).message ?? String(e)}`;
			state.finished = true;
			finalizeStream(session_id, state);
		}
	}

	/// sub_session 追问 (后端 RuntimeApi 暂不支持, 留 TODO).
	/// 当前: noop + 弹 error toast, 不抛 panic.
	async function sendToSubSession(sub_id: string, _userMessage: string) {
		const sub = subSessionStore.get(sub_id);
		if (!sub) {
			uiStore.pushToast({
				kind: 'warn',
				title: '找不到子会话, 可能已被清理',
				timeout_ms: 3000,
			});
			return;
		}
		// TODO: 等后端 RuntimeApi 加 send_to_sub_session 方法
		uiStore.pushToast({
			kind: 'info',
			title: 'sub_session 追问功能待后端 RuntimeApi 支持 (后续 sub-task)',
			timeout_ms: 3000,
		});
	}

	return {
		get streaming() {
			return streaming;
		},
		init,
		send,
		sendToSubSession,
		/// 测试专用: 重置内部状态 + 调 unlisten. 业务代码不应该调.
		async __resetForTesting() {
			if (unlisten) {
				unlisten();
				unlisten = null;
			}
			listenerInitialized = false;
			streams.clear();
			streaming.message_id = null;
		},
	};
}

export const chatStore = createChatStore();

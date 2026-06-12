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

import {
	sendMessage,
	sendMessageToSubSession,
	onSessionEvent,
	type SessionEventPayload,
} from '$lib/ipc/runtime';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { newStreamState, applyEvent, type MessageStreamState } from './chat-stream';
import { sessionStore } from './session.svelte';
import { subSessionStore } from './sub_session.svelte';
import { planStore } from './plan.svelte';
import { uiStore } from './ui.svelte';
import { reportError } from '$lib/errors';
import type { Message, PlanContract } from '$lib/types/entity';

function createChatStore() {
	// 当前流式光标
	const streaming = $state({ message_id: null as string | null });

	// Per-session 流式 state (key = session_id, value = MessageStreamState)
	const streams = new Map<string, MessageStreamState>();

	// 2026-06-12 (Phase D.8): 最近一次 user 消息, 给 resend() 用.
	// key = session_id, value = message content. 切 session 不影响, 各自记.
	const lastUserMessage = new Map<string, string>();

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

	async function send(session_id: string | null, userMessage: string) {
		const now = new Date().toISOString();

		// 0. 2026-06-09 lazy create: 如果 session_id === null (用户在 'new' view 发送),
		//    先调 sessionStore.create 拿后端真 ID, 再切到该 session view.
		if (session_id === null) {
			const view = uiStore.activeView;
			const project_id = view.kind === 'new' ? view.project_id : null;
			try {
				const newSession = await sessionStore.create({ project_id });
				session_id = newSession.id;
				uiStore.switchToSession(newSession.id);
			} catch (e) {
				// sessionStore.create 内部已 toast 错误, 这里直接 return (避免后续 sendMessage invoke 用 null)
				console.warn('[chatStore] lazy create failed:', e);
				return;
			}
		}

		// 此时 session_id 必定是后端真 ID (不可能为 null)
		const sid = session_id;

		// 1. 追加 user 消息
		const userMsg: Message = {
			id: genId(),
			session_id: sid,
			sub_session_id: null,
			role: 'user',
			content: userMessage,
			created_at: now,
		};
		sessionStore.appendMessage(sid, userMsg);

		// 2. 更新 session title (首条消息)
		const session = sessionStore.get(sid);
		if (session && session.title === session.id.slice(0, 20)) {
			// 仅当 title 还是兜底 (id slice) 才覆盖, 避免覆盖已自定义的 title
			session.title = userMessage.slice(0, 30) + (userMessage.length > 30 ? '…' : '');
		}

		// 3. Plan 触发判断 (前端关键词, TODO 后续移到后端 RuntimeApi 决策)
		const needsPlan = /jwt|认证|登录|实现|重构|调研|写测试|修 bug/i.test(userMessage);
		if (needsPlan && !planStore.bySession(sid).some((p) => p.status === 'Running')) {
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
			const plan = await planStore.create({ session_id: sid, contract });
			// 追加一个 assistant 消息 (带 plan_ref)
			const planMsg: Message = {
				id: genId(),
				session_id: sid,
				sub_session_id: null,
				role: 'assistant',
				content: '',
				plan_ref: plan.id,
				created_at: new Date().toISOString(),
			};
			sessionStore.appendMessage(sid, planMsg);
			return;
		}

		// 4. 记入 lastUserMessage, 供 resend() 用
		lastUserMessage.set(sid, userMessage);

		// 5. 普通响应: 调 sendMessage invoke + 起流式 state
		const assistantMsg: Message = {
			id: genId(),
			session_id: sid,
			sub_session_id: null,
			role: 'assistant',
			content: '',
			created_at: new Date().toISOString(),
			streaming: true,
		};
		sessionStore.appendMessage(sid, assistantMsg);
		streaming.message_id = assistantMsg.id;

		// 5. 创 stream state, onUpdate 同步到 sessionStore.message.content
		const state = newStreamState(assistantMsg.id, () => {
			const list = sessionStore.getMessages(sid);
			const m = list.find((x) => x.id === assistantMsg.id);
			if (m) m.content = state.content;
		});
		streams.set(sid, state);

		// 6. 调 invoke
		try {
			await sendMessage(sid, {
				messages: [{ role: 'user', content: userMessage }],
			});
			// 立即返, 流走 session_event 异步推
		} catch (e) {
			// 失败时本地标记 + 弹 toast
			const err = e instanceof Error ? e : new Error(String(e));
			state.content = `[错误] ${err.message}`;
			state.finished = true;
			finalizeStream(sid, state);
			// 2026-06-09: invoke reject 走不到 chat-stream error 分支 (那时 listener 都还没收到任何事件),
			// 必须在 catch 块主动 toast.
			reportError(err, {
				source: 'chatStore.send',
				toast: '发送失败',
				context: { session_id: sid },
			});
		}
	}

	/// sub_session 追问 (4a-2 P0-2 收尾: 走真实后端 invoke).
	///
	/// 流程 (跟 send 同款, 但走 send_to_sub_session command):
	/// 1. 解析 `sub_session_id → parent_session_id` (P0 阶段前端解析; P1 阶段后端解析)
	/// 2. 追加 user 消息 (sub_session_id 标记) + assistant 消息占位
	/// 3. 调 `sendMessageToSubSession(parent_session_id, req)` 走真后端 invoke
	/// 4. 流走 session_event 异步推 (跟 send 共用 listener)
	async function sendToSubSession(sub_id: string, userMessage: string) {
		const sub = subSessionStore.get(sub_id);
		if (!sub) {
			uiStore.pushToast({
				kind: 'warn',
				title: '找不到子会话, 可能已被清理',
				timeout_ms: 3000,
			});
			return;
		}

		// 0. 解析 parent_session_id (P0 阶段前端解析; P1 后端接 sub_session 持久化时去掉这步)
		const parent_sid = sub.parent_session_id;

		// 1. 追加 user 消息 (sub_session_id 标记)
		const userMsg: Message = {
			id: genId(),
			session_id: parent_sid,
			sub_session_id: sub_id,
			role: 'user',
			content: userMessage,
			kind: 'followup',
			created_at: new Date().toISOString(),
		};
		sessionStore.appendMessage(parent_sid, userMsg);

		// 2. 追加 assistant 消息占位
		const assistantMsg: Message = {
			id: genId(),
			session_id: parent_sid,
			sub_session_id: sub_id,
			role: 'assistant',
			content: '',
			created_at: new Date().toISOString(),
			streaming: true,
		};
		sessionStore.appendMessage(parent_sid, assistantMsg);
		streaming.message_id = assistantMsg.id;

		// 3. stream state (key 用 parent_sid, 跟 onSessionEvent 路由一致)
		const state = newStreamState(assistantMsg.id, () => {
			const list = sessionStore.getMessages(parent_sid);
			const m = list.find((x) => x.id === assistantMsg.id);
			if (m) m.content = state.content;
		});
		streams.set(parent_sid, state);

		// 4. 调 invoke (sub_session 追问专用入口)
		try {
			await sendMessageToSubSession(parent_sid, {
				messages: [{ role: 'user', content: userMessage }],
			});
		} catch (e) {
			const err = e instanceof Error ? e : new Error(String(e));
			state.content = `[错误] ${err.message}`;
			state.finished = true;
			finalizeStream(parent_sid, state);
			reportError(err, {
				source: 'chatStore.sendToSubSession',
				toast: 'sub_session 追问失败',
				context: { sub_session_id: sub_id, parent_session_id: parent_sid },
			});
		}
	}

	return {
		get streaming() {
			return streaming;
		},
		init,
		send,
		sendToSubSession,
		/// 2026-06-12 (Phase D.8): 重发最近一次 user 消息. UI 流式错误时给 "重试" 按钮用.
		/// 没有最近消息时弹 toast 提示, 不抛.
		async resend(sid: string): Promise<void> {
			const last = lastUserMessage.get(sid);
			if (!last) {
				reportError(new Error('没有可重发的消息'), {
					source: 'chatStore.resend',
					toast: '当前会话没有最近消息可重发',
				});
				return;
			}
			await send(sid, last);
		},
		/// 测试专用: 重置内部状态 + 调 unlisten. 业务代码不应该调.
		async __resetForTesting() {
			if (unlisten) {
				unlisten();
				unlisten = null;
			}
			listenerInitialized = false;
			streams.clear();
			streaming.message_id = null;
			lastUserMessage.clear();
		},
	};
}

export const chatStore = createChatStore();

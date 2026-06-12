// qianxun-desktop/src/lib/stores/sub_session.svelte.ts
// SubSession store
//
// 2026-06-12 收尾: 真接后端 RuntimeApi. E2E Round 1 反馈 "plan 列表点击打开子会话
// 无法打开" 根因 = 后端没建 sub_session 实体, byPlan() 永远 []. 本次补:
//   1. loadAll 调 listSubSessions 拉全量 (init 一次性)
//   2. 订阅 sub_session_event, 实时 upsert 实体 (execute_one_task 启动 / 完成时 emit)
//   3. open(id) → uiStore.switchToSubSession, 跟之前同模式, 这次 byPlan 拿得到
//
// 关联:
//   - $lib/ipc/runtime.ts::listSubSessions / onSubSessionEvent / subscribeSubSessionEvents
//   - qianxun-runtime/src/api/sub_sessions.rs (4 个 RuntimeApi 方法 + SseEvent::SubSessionUpdate)
//   - $lib/types/entity.ts::SubSession (前端实体, 跟后端 SubSessionInfo 字段不完全一致)

import { uiStore } from './ui.svelte';
import type { SubSession, SubSessionStatus, Message } from '$lib/types/entity';
import {
	listSubSessions,
	subscribeSubSessionEvents,
	onSubSessionEvent,
	type SubSessionInfo,
} from '$lib/ipc/runtime';

/// 后端 SubSessionInfo → 前端 SubSession. task_id → plan_task_id, status 改 PascalCase.
function toEntity(info: SubSessionInfo): SubSession {
	return {
		id: info.id,
		plan_id: info.plan_id,
		plan_task_id: info.task_id,
		parent_session_id: info.parent_session_id,
		role: info.role,
		status: info.status === 'active'
			? 'Active'
			: info.status === 'done'
				? 'Done'
				: info.status === 'failed'
					? 'Failed'
					: 'Aborted',
		messages: [],
		output: info.output,
		started_at: info.started_at,
		ended_at: info.ended_at,
	};
}

function createSubSessionStore() {
	const subSessions = $state<SubSession[]>([]);
	let initialized = $state(false);
	let loading = $state(false);
	let lastError = $state<string | null>(null);
	let _unlisten: (() => void) | null = null;

	const activeSubSession = $derived.by(() => {
		const view = uiStore.activeView;
		return view.kind === 'sub_session'
			? subSessions.find((s) => s.id === view.sub_session_id) ?? null
			: null;
	});

	/// 启动时调一次. 拉全量 + 订阅 realtime, 跟 plan.svelte.ts::init 同模式.
	async function loadAll() {
		if (initialized || loading) return;
		loading = true;
		lastError = null;
		try {
			// 1. 拉全量 (空数组 web fallback, 桌面端走 Tauri invoke)
			const list = await listSubSessions();
			subSessions.length = 0;
			subSessions.push(...list.map(toEntity));
			// 2. 启动后端 broadcast bus (幂等, 多次调 OK)
			await subscribeSubSessionEvents();
			// 3. 订阅 sub_session_event, 收到 SubSessionUpdate 时 upsert
			_unlisten = await onSubSessionEvent((payload) => {
				if (payload.type === 'sub_session_update') {
					try {
						const info = JSON.parse(payload.sub_session_json) as SubSessionInfo;
						upsert(toEntity(info));
					} catch (e) {
						console.warn('[subSessionStore] parse sub_session_json failed:', e);
					}
				}
			});
			initialized = true;
		} catch (e) {
			lastError = (e as Error).message ?? String(e);
		} finally {
			loading = false;
		}
	}

	/// 内部: upsert 实体 (init 拉全量 + realtime 事件共用). 已存在则更新, 不存在则 push.
	function upsert(s: SubSession): void {
		const idx = subSessions.findIndex((x) => x.id === s.id);
		if (idx >= 0) {
			// 保留已有 messages (实时事件不带 messages, 全量拉时 messages=[])
			const prev = subSessions[idx]!;
			subSessions[idx] = { ...s, messages: prev.messages };
		} else {
			subSessions.push(s);
		}
	}

	/// 内部: plan execution 推新 sub_session 时调 (后续 sub-task 接).
	/// 当前 sub-task 范围: 保留 API 但不被外部调.
	function add(opts: Omit<SubSession, 'messages' | 'output'>): SubSession {
		const sub: SubSession = { ...opts, messages: [], output: null };
		subSessions.push(sub);
		return sub;
	}

	return {
		get all() {
			return subSessions;
		},
		get initialized() {
			return initialized;
		},
		get loading() {
			return loading;
		},
		get lastError() {
			return lastError;
		},
		get(id: string): SubSession | undefined {
			return subSessions.find((s) => s.id === id);
		},
		byPlan(planId: string): SubSession[] {
			return subSessions.filter((s) => s.plan_id === planId);
		},
		byParent(parentId: string): SubSession[] {
			return subSessions.filter((s) => s.parent_session_id === parentId);
		},
		get active(): SubSession | null {
			return activeSubSession;
		},
		countByPlan(planId: string): number {
			return subSessions.filter((s) => s.plan_id === planId).length;
		},
		countByPlanStatus(planId: string, status: SubSessionStatus): number {
			return subSessions.filter((s) => s.plan_id === planId && s.status === status).length;
		},
		messagesOf(id: string): Message[] {
			return subSessions.find((s) => s.id === id)?.messages ?? [];
		},
		appendMessage(subId: string, msg: Message) {
			const s = subSessions.find((x) => x.id === subId);
			if (s) s.messages.push(msg);
		},
		open(id: string) {
			const s = subSessions.find((x) => x.id === id);
			if (s) {
				uiStore.switchToSubSession(id, s.parent_session_id);
			}
		},
		terminate(planId: string, index: number, status: SubSessionStatus = 'Done') {
			const list = subSessions.filter((s) => s.plan_id === planId);
			if (list[index]) {
				list[index].status = status;
				list[index].ended_at = new Date().toISOString();
			}
		},
		// 任务是否处于"执行中" — Active 才算执行; 其他都视为"已完成/已结束/可追问"
		isActive(s: SubSession): boolean {
			return s.status === 'Active';
		},
		isReadOnly(s: SubSession): boolean {
			return s.status === 'Done' || s.status === 'Failed' || s.status === 'Aborted';
		},
		canSend(s: SubSession): boolean {
			// 任何状态都可以发消息 (追问模式), 仅 'ReadOnly' 状态完全冻结
			return s.status !== 'ReadOnly';
		},
		loadAll,
		add,
		/// 测试专用: 重置内部状态. 业务代码不应该调.
		__resetForTesting() {
			subSessions.length = 0;
			initialized = false;
			loading = false;
			lastError = null;
			if (_unlisten) {
				_unlisten();
				_unlisten = null;
			}
		},
	};
}

export const subSessionStore = createSubSessionStore();

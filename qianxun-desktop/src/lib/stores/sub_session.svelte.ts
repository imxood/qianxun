// qianxun-desktop/src/lib/stores/sub_session.svelte.ts
// SubSession store
//
// Stage 4a (sub-task #4): 删 buildSeed 依赖.
//   - SubSession 是 plan 执行的子任务状态, 后端 RuntimeApi 暂没完整 plan execution 接口
//     (sub-task #3 范围之外, 后续 sub-task 接)
//   - 当前: 空数组, 等 plan_update 事件 / 后端 push 真实 sub_session 时填充
//   - API 表面保持不变, sendToSubSession 等方法仍可用, 业务容错
//
// 关联:
//   - $lib/ipc/runtime.ts (后续 sub-task 接 createPlan + plan_update 事件)
//   - qianxun-runtime/src/api/types.rs (PlanInfo DTO)
//   - docs/30_子项目规划/04b-tauri-runtime-integration.md §"Sub-task 5/6"

import { uiStore } from './ui.svelte';
import type { SubSession, SubSessionStatus, Message } from '$lib/types/entity';

function createSubSessionStore() {
	const subSessions = $state<SubSession[]>([]);
	let initialized = $state(false);
	let loading = $state(false);
	let lastError = $state<string | null>(null);

	const activeSubSession = $derived.by(() => {
		const view = uiStore.activeView;
		return view.kind === 'sub_session'
			? subSessions.find((s) => s.id === view.sub_session_id) ?? null
			: null;
	});

	/// 启动时调. 当前 noop (后端暂没 list_sub_sessions RuntimeApi).
	/// 真实 sub_session 由 plan execution 推过来, 后续 sub-task 加 'plan_update' 事件监听.
	async function loadAll() {
		if (initialized || loading) return;
		loading = true;
		lastError = null;
		try {
			// TODO: 等后端 RuntimeApi 加 list_sub_sessions 或 plan_update 事件
			initialized = true;
		} catch (e) {
			lastError = (e as Error).message ?? String(e);
		} finally {
			loading = false;
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
		},
	};
}

export const subSessionStore = createSubSessionStore();

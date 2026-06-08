// qianxun-desktop/src/lib/stores/sub_session.svelte.ts
// SubSession store

import { buildSeed } from '$lib/mock/seed';
import { uiStore } from './ui.svelte';
import type { SubSession, SubSessionStatus, Message } from '$lib/types/entity';

function createSubSessionStore() {
	const subSessions = $state<SubSession[]>(buildSeed().sub_sessions);

	const activeSubSession = $derived(
		uiStore.activeView.kind === 'sub_session'
			? subSessions.find((s) => s.id === uiStore.activeView.sub_session_id) ?? null
			: null,
	);

	return {
		get all() {
			return subSessions;
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
	};
}

export const subSessionStore = createSubSessionStore();

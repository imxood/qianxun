// qianxun-desktop/src/lib/stores/plan.svelte.ts
// Plan store (含 task 自动调度 mock, 不持久化)

import { buildSeed } from '$lib/mock/seed';
import { subSessionStore } from './sub_session.svelte';
import { uiStore } from './ui.svelte';
import type { Plan, ChangedFile } from '$lib/types/entity';

function createPlanStore() {
	const seed = buildSeed();
	const plans = $state<Plan[]>(seed.plans);
	const changedFiles = $state<Record<string, ChangedFile[]>>({
		[seed.plans[0].id]: seed.changed_files,
	});

	// 自动调度: 启动时如果 plan_jwt_42 是 Running, 5s 后第一次 tick
	let schedulerHandles: Record<string, ReturnType<typeof setTimeout>> = {};
	for (const plan of plans) {
		if (plan.status === 'Running' && !schedulerHandles[plan.id]) {
			scheduleAutoComplete(plan.id);
		}
	}

	function scheduleAutoComplete(planId: string) {
		if (schedulerHandles[planId]) return;
		const tick = () => {
			const plan = plans.find((p) => p.id === planId);
			if (!plan || plan.status !== 'Running') return;

			const total = plan.contract.tasks.length;
			const currentDone = plan.result?.tasks_completed ?? subSessionStore.countByPlanStatus(plan.id, 'Done');
			const nextDone = currentDone + 1;

			if (nextDone > total) {
				plan.status = 'Done';
				plan.ended_at = new Date().toISOString();
				plan.result = {
					summary: `全部 ${total} 个子任务完成`,
					tasks_completed: total,
					tasks_total: total,
					deliverables: ['已完成所有 task'],
				};
				if (schedulerHandles[planId]) clearTimeout(schedulerHandles[planId]);
				schedulerHandles[planId] = null as unknown as ReturnType<typeof setTimeout>;
				return;
			}

			if (nextDone <= subSessionStore.countByPlan(planId)) {
				subSessionStore.terminate(planId, nextDone - 1, 'Done');
			}

			schedulerHandles[planId] = setTimeout(tick, 15000);
		};
		schedulerHandles[planId] = setTimeout(tick, 5000);
	}

	const activePlan = $derived(
		uiStore.activeView.kind === 'session'
			? plans.find(
					(p) => p.session_id === uiStore.activeView.session_id && p.status === 'Running',
				) ?? null
			: null,
	);

	return {
		get all() {
			return plans;
		},
		get(id: string): Plan | undefined {
			return plans.find((p) => p.id === id);
		},
		bySession(sessionId: string): Plan[] {
			return plans.filter((p) => p.session_id === sessionId);
		},
		get active(): Plan | null {
			return activePlan;
		},
		getChangedFiles(planId: string): ChangedFile[] {
			return changedFiles[planId] ?? [];
		},
		create(opts: { session_id: string; contract: Plan['contract'] }): Plan {
			const now = new Date().toISOString();
			const id = `plan_${now.replace(/[-:T.Z]/g, '').slice(0, 17)}_${Math.random().toString(36).slice(2, 6)}`;
			const newPlan: Plan = {
				id,
				session_id: opts.session_id,
				contract: opts.contract,
				status: 'Running',
				started_at: now,
				ended_at: null,
				result: null,
				attachments: [],
			};
			plans.push(newPlan);
			scheduleAutoComplete(id);
			return newPlan;
		},
		cancel(planId: string) {
			const p = plans.find((x) => x.id === planId);
			if (p && p.status === 'Running') {
				p.status = 'Aborted';
				p.ended_at = new Date().toISOString();
				if (schedulerHandles[planId]) clearTimeout(schedulerHandles[planId]);
			}
		},
		progressOf(plan: Plan): { done: number; total: number } {
			const total = plan.contract.tasks.length;
			const done = plan.result?.tasks_completed ?? subSessionStore.countByPlanStatus(plan.id, 'Done');
			return { done, total };
		},
	};
}

export const planStore = createPlanStore();

// qianxun-desktop/src/lib/stores/plan.svelte.ts
// Plan store
//
// Stage 4a (sub-task #4): 切真后端 invoke.
//   - 删 scheduleAutoComplete setTimeout (mock 阶段 auto-completion 调度)
//   - create() 调 invoke('create_plan') 拿真实 PlanInfo
//   - cancel() 调 invoke('cancel_session') (后端 RuntimeApi 暂没 plan.cancel 单独方法)
//   - progressOf() 改纯函数 (没后端实时进度, 只能看 contract.tasks.length vs result.tasks_completed)
//
// 业务约束:
//   - 后端 SseEvent 暂没 plan_update variant (qianxun-runtime/src/sse.rs 12 variant 不含 plan_update)
//     → plan 进度跟踪等后续 sub-task 加 (需要后端加 plan_update emit + Tauri command)
//   - 取消走 cancel_session 而非 plan.cancel: 后端 RuntimeApi 5 方法没 plan.cancel,
//     业务上取消 session 即可终止 session 上的 plan (跟现有 daemon 一致)
//
// 关联:
//   - $lib/ipc/runtime.ts (createPlan / cancelSession invoke)
//   - qianxun-runtime/src/api/types.rs (PlanInfo DTO)
//   - docs/30_子项目规划/04b-tauri-runtime-integration.md §"Sub-task 5"

import { createPlan, cancelPlan, type PlanInfo as IpcPlanInfo, type PlanTaskResult } from '$lib/ipc/runtime';
import { subSessionStore } from './sub_session.svelte';
import { uiStore } from './ui.svelte';
import type { Plan, PlanStatus as EntityPlanStatus, ChangedFile } from '$lib/types/entity';

/// IpcPlanInfo (后端 DTO, lowercase status) → Plan (前端 entity, PascalCase status) 转换.
///
/// Phase D 收尾: 后端 PlanInfo 带 contract + task_results, 前端 1:1 透传.
/// 之前是前端构造空 contract, 现在直接拿后端实数据.
function ipcPlanToEntity(p: IpcPlanInfo): Plan {
	// status 5 态 lowercase → PascalCase 映射
	const statusMap: Record<string, Plan['status']> = {
		pending: 'Pending',
		running: 'Running',
		done: 'Done',
		failed: 'Failed',
		aborted: 'Aborted',
	};
	// contract 兜底 (旧 mock 路径可能没 contract 字段)
	const contract = p.contract ?? {
		name: p.name,
		description: '',
		tasks: [],
		timeout_ms: 0,
	};
	// task_results 兜底 (旧 mock 路径可能没 task_results)
	const taskResults: PlanTaskResult[] = p.task_results ?? [];
	// 派生 result: 统计完成 task 数, 没结果时 null
	const tasksTotal = contract.tasks.length;
	const tasksCompleted = taskResults.filter(
		(r) => r.status === 'done' || r.status === 'aborted',
	).length;
	return {
		id: p.id,
		session_id: p.session_id,
		contract,
		status: statusMap[p.status] ?? 'Pending',
		started_at: p.started_at,
		ended_at: p.ended_at,
		result: p.ended_at
			? {
					summary: '',
					tasks_completed: tasksCompleted,
					tasks_total: tasksTotal,
					deliverables: [],
				}
			: null,
		attachments: [],
	};
}

function createPlanStore() {
	const plans = $state<Plan[]>([]);
	const changedFiles = $state<Record<string, ChangedFile[]>>({});
	let lastError = $state<string | null>(null);

	const activePlan = $derived.by(() => {
		const view = uiStore.activeView;
		return view.kind === 'session'
			? plans.find((p) => p.session_id === view.session_id && p.status === 'Running') ?? null
			: null;
	});

	/// 创 plan (调 invoke).
	/// planStatus 自动为 'Running' (后端 create_plan 一定返 running).
	/// 失败: 弹 toast, 抛错.
	/// contract 用 caller 传入的 (含 tasks), 后端 PlanInfo 不返 contract.
	async function create(opts: {
		session_id: string;
		contract: Plan['contract'];
	}): Promise<Plan> {
		try {
			const ipc = await createPlan({
				session_id: opts.session_id,
				name: opts.contract.name,
				description: opts.contract.description,
				timeout_ms: opts.contract.timeout_ms,
			});
			const plan: Plan = {
				...ipcPlanToEntity(ipc),
				contract: opts.contract,
			};
			plans.push(plan);
			return plan;
		} catch (e) {
			const msg = (e as Error).message ?? String(e);
			lastError = msg;
			uiStore.pushToast({
				kind: 'error',
				title: `创建 plan 失败: ${msg}`,
				timeout_ms: 5000,
			});
			throw e;
		}
	}

	/// 取消 plan (调 invoke cancel_plan, Phase D 收尾).
	/// 之前走 cancel_session (间接取消 plan 绑定的 session), 现在用 plan 级别
	/// 取消接口, 后端 RuntimeApi::cancel_plan 直接改 plan.status = Aborted.
	async function cancel(planId: string): Promise<void> {
		const p = plans.find((x) => x.id === planId);
		if (!p || p.status !== 'Running') return;
		try {
			await cancelPlan(planId);
			// 本地立即更新 UI (后端 cancel_plan 同步返, 不需要等回调)
			p.status = 'Aborted';
			p.ended_at = new Date().toISOString();
		} catch (e) {
			const msg = (e as Error).message ?? String(e);
			lastError = msg;
			uiStore.pushToast({
				kind: 'error',
				title: `取消 plan 失败: ${msg}`,
				timeout_ms: 5000,
			});
		}
	}

	/// 进度计算: contract.tasks.length 是总数, 已完成 = result.tasks_completed 兜底
	/// (后续 sub-task 接 plan_update 事件实时更新 result).
	function progressOf(plan: Plan): { done: number; total: number } {
		const total = plan.contract.tasks.length;
		const done = plan.result?.tasks_completed ?? 0;
		return { done, total };
	}

	return {
		get all() {
			return plans;
		},
		get lastError() {
			return lastError;
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
		create,
		cancel,
		progressOf,
		/// 测试专用: 重置内部状态. 业务代码不应该调.
		__resetForTesting() {
			plans.length = 0;
			for (const k of Object.keys(changedFiles)) delete changedFiles[k];
			lastError = null;
		},
	};
}

export const planStore = createPlanStore();

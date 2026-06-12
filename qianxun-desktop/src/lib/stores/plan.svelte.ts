// qianxun-desktop/src/lib/stores/plan.svelte.ts
// Plan store
//
// Stage 4a (sub-task #4): 切真后端 invoke.
//   - 删 scheduleAutoComplete setTimeout (mock 阶段 auto-completion 调度)
//   - create() 调 invoke('create_plan') 拿真实 PlanInfo
//   - cancel() 调 invoke('cancel_session') (后端 RuntimeApi 暂没 plan.cancel 单独方法)
//   - progressOf() 改纯函数 (没后端实时进度, 只能看 contract.tasks.length vs result.tasks_completed)
//
// P1-3 收尾 (2026-06-12): 接 `plan_event` Tauri 实时事件.
//   - init() 在 app 启动时调, 启后端 subscribePlanEvents 长连接 + listen "plan_event"
//   - 收到 plan_update 事件 → 按 plan_id 找本地 plan, 更新 status / task_results
//   - 之前要等 listPlans() 拉或轮询, 现在实时推 (毫秒级)
//
// 业务约束:
//   - 后端 SseEvent 现 16 variant 含 plan_update (P1-3 收尾加)
//   - 取消走 cancel_plan (P0 收尾加)
//
// 关联:
//   - $lib/ipc/runtime.ts (createPlan / cancelPlan / onPlanEvent / subscribePlanEvents)
//   - qianxun-runtime/src/api/types.rs (PlanInfo DTO)
//   - qianxun-runtime/src/api/plans.rs (SQLite 持久化 + broadcast bus 收尾)
//   - docs/30_子项目规划/04b-tauri-runtime-integration.md §"Sub-task 5"

import {
	createPlan,
	cancelPlan,
	onPlanEvent,
	subscribePlanEvents,
	type PlanInfo as IpcPlanInfo,
	type PlanTaskResult,
} from '$lib/ipc/runtime';
import { subSessionStore } from './sub_session.svelte';
import { uiStore } from './ui.svelte';
import { reportError } from '$lib/errors';
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
	///
	/// 2026-06-12 (批次 1.3): 失败回滚 — 错误统一走 reportError (不再独立设 lastError,
	/// 跟 sessionStore 错误路径合并); plans.push 在 throw 之前, 失败时不残留半状态.
	/// planMsg (带 plan_ref 的 assistant 消息) 由调用方 (chat.svelte.ts: send) 在拿到 plan
	/// 引用后追加, 失败时不追加, 避免半态 UI 残留.
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
				tasks: opts.contract.tasks,
			});
			const plan: Plan = {
				...ipcPlanToEntity(ipc),
				contract: opts.contract,
			};
			plans.push(plan);
			return plan;
		} catch (e) {
			reportError(e, {
				source: 'planStore.create',
				toast: '创建 plan 失败',
				context: { session_id: opts.session_id, plan_name: opts.contract.name },
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
			lastError = e instanceof Error ? e.message : String(e);
			reportError(e, {
				source: 'planStore.cancel',
				toast: '取消 plan 失败',
				context: { plan_id: planId },
			});
		}
	}

	/// 进度计算: contract.tasks.length 是总数, 已完成 = result.tasks_completed 兜底
	/// (P1-3 收尾: 实时 event 更新 result.tasks_completed).
	function progressOf(plan: Plan): { done: number; total: number } {
		const total = plan.contract.tasks.length;
		const done = plan.result?.tasks_completed ?? 0;
		return { done, total };
	}

	/// P1-3 收尾: 处理后端 plan_event (SseEvent::PlanUpdate).
	/// 收到 plan_update → 按 plan_id 找本地 plan, 更新 status / ended_at / task_results 派生 result.
	/// status 5 态 lowercase → PascalCase 映射 (跟 ipcPlanToEntity 共用).
	function handlePlanEvent(
		planId: string,
		status: string,
		taskResultsJson: string | null,
	): void {
		const plan = plans.find((p) => p.id === planId);
		if (!plan) return; // 未知 plan_id 忽略 (可能 web fallback / 旧 plan)
		const statusMap: Record<string, Plan['status']> = {
			pending: 'Pending',
			running: 'Running',
			done: 'Done',
			failed: 'Failed',
			aborted: 'Aborted',
		};
		const newStatus = statusMap[status] ?? plan.status;
		plan.status = newStatus;
		// 终态 (Done / Failed / Aborted) 写 ended_at, 兼容后端没传 (e.g. Running)
		if (newStatus === 'Done' || newStatus === 'Failed' || newStatus === 'Aborted') {
			plan.ended_at = plan.ended_at ?? new Date().toISOString();
		}
		// task_results_json 反序列化, 算 tasks_completed 更新 plan.result
		if (taskResultsJson) {
			try {
				const taskResults: PlanTaskResult[] = JSON.parse(taskResultsJson);
				const tasksCompleted = taskResults.filter(
					(r) => r.status === 'done' || r.status === 'aborted',
				).length;
				const tasksTotal = plan.contract.tasks.length;
				plan.result = {
					summary: plan.result?.summary ?? '',
					tasks_completed: tasksCompleted,
					tasks_total: tasksTotal,
					deliverables: plan.result?.deliverables ?? [],
				};
			} catch (e) {
				// 静默: parse 失败不该打扰用户, 但留痕
				reportError(e, { source: 'planStore.handlePlanEvent.parse', context: { plan_id: planId } });
			}
		}
	}

	/// P1-3 收尾: 启 plan 事件订阅 (在 app 启动时调一次).
	/// 1. 调 subscribePlanEvents() 让 Tauri 后端 spawn 长连接任务
	/// 2. listen "plan_event" Tauri 事件, 转 handlePlanEvent 处理
	/// 3. 返 unlisten 函数 (app 关闭时调)
	async function init(): Promise<() => void> {
		try {
			await subscribePlanEvents();
		} catch (e) {
			// 静默: 实时事件非关键, 失败降级到手动刷新
			reportError(e, { source: 'planStore.init.subscribe' });
			return () => {};
		}
		const unlisten = await onPlanEvent((event) => {
			if (event.type !== 'plan_update') return;
			handlePlanEvent(event.plan_id, event.status, event.task_results_json);
		});
		return unlisten;
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
		init,
		/// 测试专用: 重置内部状态. 业务代码不应该调.
		__resetForTesting() {
			plans.length = 0;
			for (const k of Object.keys(changedFiles)) delete changedFiles[k];
			lastError = null;
		},
	};
}

export const planStore = createPlanStore();

// ───────────────────────────────────────────────────────────────────────────
// PlanStore — Stage 4a (sub-task #4) 切真后端测试 + Phase D 收尾 cancel_plan
//
// 测试覆盖 (mock $lib/ipc/runtime):
//   1. create() 调 invoke('create_plan') + 转 PlanInfo → Plan + 推进 plans[]
//   2. create() 失败弹 error toast + lastError
//   3. cancel() 调 invoke('cancel_plan') + 本地 status = 'Aborted'  (Phase D)
//   4. cancel() 仅 Running 状态能取消
//   5. progressOf() 用 contract.tasks.length 算 total
//   6. bySession / get / active 派生保持兼容
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

const createPlanMock = vi.fn();
const cancelPlanMock = vi.fn();
const subscribePlanEventsMock = vi.fn();
const onPlanEventMock = vi.fn();

vi.mock("$lib/ipc/runtime", () => ({
	createPlan: (...args: unknown[]) => createPlanMock(...args),
	cancelPlan: (...args: unknown[]) => cancelPlanMock(...args),
	subscribePlanEvents: (...args: unknown[]) => subscribePlanEventsMock(...args),
	onPlanEvent: (...args: unknown[]) => onPlanEventMock(...args),
}));

import { planStore } from "$lib/stores/plan.svelte";
import { uiStore } from "$lib/stores/ui.svelte";
import type { PlanContract } from "$lib/types/entity";

const FAKE_CONTRACT: PlanContract = {
	name: "JWT 鉴权",
	description: "实现 JWT 鉴权",
	timeout_ms: 1800000,
	tasks: [
		{ id: "task_a", title: "分析", prompt: "p", assigned_to: "coder", verified_by: null, verify_prompt: null, depends_on: [], timeout_ms: 600000 },
		{ id: "task_b", title: "实现", prompt: "p", assigned_to: "coder", verified_by: null, verify_prompt: null, depends_on: ["task_a"], timeout_ms: 900000 },
		{ id: "task_c", title: "测试", prompt: "p", assigned_to: "tester", verified_by: null, verify_prompt: null, depends_on: ["task_b"], timeout_ms: 900000 },
	],
};

function resetPlanStore() {
	planStore.__resetForTesting();
}

beforeEach(() => {
	createPlanMock.mockReset();
	cancelPlanMock.mockReset();
	subscribePlanEventsMock.mockReset();
	onPlanEventMock.mockReset();
	resetPlanStore();
	uiStore.setActiveView({ kind: "empty" });
});

describe("PlanStore (Stage 4a sub-task #4 切 invoke)", () => {
	it("create_calls_invoke_and_converts_response: PlanInfo → Plan (lowercase status → PascalCase)", async () => {
		createPlanMock.mockResolvedValueOnce({
			id: "plan_jwt_001",
			session_id: "sess_001",
			name: "JWT 鉴权",
			status: "running",
			started_at: "2026-06-08T00:00:00Z",
			ended_at: null,
		});

		const plan = await planStore.create({
			session_id: "sess_001",
			contract: FAKE_CONTRACT,
		});

		expect(createPlanMock).toHaveBeenCalledWith({
			session_id: "sess_001",
			name: "JWT 鉴权",
			description: "实现 JWT 鉴权",
			timeout_ms: 1800000,
			tasks: FAKE_CONTRACT.tasks,
		});
		expect(plan.id).toBe("plan_jwt_001");
		expect(plan.status).toBe("Running"); // lowercase → PascalCase
		expect(plan.session_id).toBe("sess_001");
		expect(planStore.all).toHaveLength(1);
	});

	it("create_failure_shows_error_toast: 失败处理", async () => {
		// 2026-06-12 (批次 1.3): 错误路径合并 — 走 reportError (含 toast) 单一路, 不再独立设 lastError.
		// plans 也不残留 (push 在 throw 之前).
		createPlanMock.mockRejectedValueOnce(new Error("internal error: store error"));

		await expect(
			planStore.create({ session_id: "sess_001", contract: FAKE_CONTRACT }),
		).rejects.toThrow("internal error: store error");
		// 不再独立设 lastError (语义分离: lastError 留 sessionStore 级错误, plan 业务错误走 reportError)
		expect(planStore.lastError).toBeNull();
		// toast 仍弹 (reportError 走的)
		expect(uiStore.toasts.length).toBeGreaterThan(0);
		// plans 没残留半状态 (push 在 throw 之前)
		expect(planStore.all).toHaveLength(0);
	});

	it("cancel_running_plan_calls_invoke_and_marks_aborted: 取消走 cancel_plan (Phase D 收尾)", async () => {
		createPlanMock.mockResolvedValueOnce({
			id: "plan_001",
			session_id: "sess_001",
			name: "test",
			status: "running",
			started_at: new Date().toISOString(),
			ended_at: null,
		});
		cancelPlanMock.mockResolvedValueOnce(undefined);

		const plan = await planStore.create({ session_id: "sess_001", contract: FAKE_CONTRACT });
		expect(plan.status).toBe("Running");

		await planStore.cancel("plan_001");
		// Phase D 收尾: 调 cancel_plan(plan_id), 不是 cancel_session(session_id)
		expect(cancelPlanMock).toHaveBeenCalledWith("plan_001");
		expect(planStore.get("plan_001")?.status).toBe("Aborted");
		expect(planStore.get("plan_001")?.ended_at).not.toBeNull();
	});

	it("cancel_non_running_plan_is_noop: 非 Running 状态忽略", async () => {
		createPlanMock.mockResolvedValueOnce({
			id: "plan_done",
			session_id: "sess_001",
			name: "done",
			status: "done",
			started_at: "2026-06-08T00:00:00Z",
			ended_at: "2026-06-08T01:00:00Z",
		});
		const plan = await planStore.create({ session_id: "sess_001", contract: FAKE_CONTRACT });
		// Plan 已经是 Done 状态 (ipcPlanToEntity 转换: done → Done)
		expect(plan.status).toBe("Done");

		await planStore.cancel("plan_done");
		expect(cancelPlanMock).not.toHaveBeenCalled();
	});

	it("progressOf_uses_contract_tasks_length: 纯函数计算进度", async () => {
		createPlanMock.mockResolvedValueOnce({
			id: "plan_p",
			session_id: "sess_p",
			name: "p",
			status: "running",
			started_at: new Date().toISOString(),
			ended_at: null,
		});
		const plan = await planStore.create({ session_id: "sess_p", contract: FAKE_CONTRACT });
		const progress = planStore.progressOf(plan);
		expect(progress.total).toBe(3);
		expect(progress.done).toBe(0); // result null, 兜底 0
	});

	it("bySession_filters_by_session_id: 派生方法兼容", async () => {
		createPlanMock.mockResolvedValueOnce({
			id: "plan_a",
			session_id: "sess_a",
			name: "a",
			status: "running",
			started_at: new Date().toISOString(),
			ended_at: null,
		});
		createPlanMock.mockResolvedValueOnce({
			id: "plan_b",
			session_id: "sess_b",
			name: "b",
			status: "running",
			started_at: new Date().toISOString(),
			ended_at: null,
		});
		await planStore.create({ session_id: "sess_a", contract: FAKE_CONTRACT });
		await planStore.create({ session_id: "sess_b", contract: FAKE_CONTRACT });

		expect(planStore.bySession("sess_a")).toHaveLength(1);
		expect(planStore.bySession("sess_b")).toHaveLength(1);
	});

	it("active_plan_derived_from_active_view: uiStore 驱动", async () => {
		createPlanMock.mockResolvedValueOnce({
			id: "plan_active",
			session_id: "sess_active",
			name: "active",
			status: "running",
			started_at: new Date().toISOString(),
			ended_at: null,
		});
		await planStore.create({ session_id: "sess_active", contract: FAKE_CONTRACT });

		// 没切到该 session 时 active 应为 null
		expect(planStore.active).toBeNull();

		// 切到该 session
		uiStore.setActiveView({ kind: "session", session_id: "sess_active" });
		expect(planStore.active?.id).toBe("plan_active");

		// 切到其他 session
		uiStore.setActiveView({ kind: "session", session_id: "sess_other" });
		expect(planStore.active).toBeNull();
	});

	// 回归: planStore.init() 必须在 app 启动时调一次, 否则 PlanUpdate 事件断链.
	// 之前 +page.svelte 启动序列只调了 chatStore.init, UI 永远收不到 plan_event.
	it("init_registers_plan_event_listener_and_subscribes_backend: P1-3 事件链", async () => {
		const unlisten = vi.fn();
		subscribePlanEventsMock.mockResolvedValueOnce(undefined);
		onPlanEventMock.mockResolvedValueOnce(unlisten);

		const returned = await planStore.init();

		// 1. 调了后端 subscribePlanEvents 启 broadcast 任务
		expect(subscribePlanEventsMock).toHaveBeenCalledTimes(1);
		// 2. 调了 onPlanEvent 注册 Tauri 'plan_event' listener
		expect(onPlanEventMock).toHaveBeenCalledTimes(1);
		// 3. 返非空 unlisten (给 caller 用来 unregister)
		expect(returned).toBe(unlisten);
	});
});

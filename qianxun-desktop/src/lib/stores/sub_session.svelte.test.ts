// ───────────────────────────────────────────────────────────────────────────
// SubSessionStore — Stage 4a (sub-task #4) 测试
//
// 测试覆盖:
//   1. loadAll 当前 noop (后端没 list_sub_sessions RuntimeApi)
//   2. API 表面 (isActive / isReadOnly / canSend / byPlan / countByPlan) 保持兼容
//   3. 空数据时业务方法不 panic
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

const listSubSessionsMock = vi.fn();
vi.mock("$lib/ipc/runtime", () => ({
	listSubSessions: (...args: unknown[]) => listSubSessionsMock(...args),
}));

import { subSessionStore } from "$lib/stores/sub_session.svelte";
import { uiStore } from "$lib/stores/ui.svelte";
import type { SubSession } from "$lib/types/entity";

const FAKE_SUB: SubSession = {
	id: "sub_001",
	plan_id: "plan_jwt",
	plan_task_id: "task_a",
	parent_session_id: "sess_jwt_auth",
	role: "coder",
	status: "Active",
	messages: [],
	output: null,
	started_at: "2026-06-08T00:00:00Z",
	ended_at: null,
};

describe("SubSessionStore (Stage 4a sub-task #4)", () => {
	beforeEach(() => {
		listSubSessionsMock.mockReset();
		subSessionStore.all.length = 0;
		(subSessionStore as unknown as { initialized: boolean }).initialized = false;
		uiStore.setActiveView({ kind: "empty" });
	});

	it("loadAll_is_currently_noop: 后端 RuntimeApi 暂没 list_sub_sessions", async () => {
		await subSessionStore.loadAll();
		expect(listSubSessionsMock).not.toHaveBeenCalled();
		expect(subSessionStore.initialized).toBe(true);
	});

	it("byPlan_returns_empty_for_empty_list: 空数据查询安全", () => {
		expect(subSessionStore.byPlan("plan_xxx")).toEqual([]);
		expect(subSessionStore.byParent("sess_xxx")).toEqual([]);
		expect(subSessionStore.countByPlan("plan_xxx")).toBe(0);
		expect(subSessionStore.countByPlanStatus("plan_xxx", "Done")).toBe(0);
	});

	it("isActive_isReadOnly_canSend_logic_preserved: 状态判定 API 兼容", () => {
		const active: SubSession = { ...FAKE_SUB, status: "Active" };
		const done: SubSession = { ...FAKE_SUB, status: "Done" };
		const readOnly: SubSession = { ...FAKE_SUB, status: "ReadOnly" };

		expect(subSessionStore.isActive(active)).toBe(true);
		expect(subSessionStore.isActive(done)).toBe(false);

		expect(subSessionStore.isReadOnly(active)).toBe(false);
		expect(subSessionStore.isReadOnly(done)).toBe(true);

		expect(subSessionStore.canSend(active)).toBe(true);
		expect(subSessionStore.canSend(done)).toBe(true); // done 也能追问
		expect(subSessionStore.canSend(readOnly)).toBe(false);
	});

	it("add_appends_sub_session_for_future_wiring: 内部 add() 供后续 plan 事件 push", () => {
		const sub = subSessionStore.add({
			id: "sub_new",
			plan_id: "plan_xyz",
			plan_task_id: "task_1",
			parent_session_id: "sess_xyz",
			role: "coder",
			status: "Active",
			started_at: new Date().toISOString(),
			ended_at: null,
		});
		expect(sub.messages).toEqual([]);
		expect(subSessionStore.all).toHaveLength(1);
		expect(subSessionStore.get("sub_new")).toBeDefined();
	});

	it("appendMessage_pushes_to_sub_session: 业务方法兼容", () => {
		const sub = subSessionStore.add({
			id: "sub_msg",
			plan_id: "plan_m",
			plan_task_id: "task_m",
			parent_session_id: "sess_m",
			role: "coder",
			status: "Active",
			started_at: new Date().toISOString(),
			ended_at: null,
		});
		subSessionStore.appendMessage("sub_msg", {
			id: "msg_1",
			session_id: null,
			sub_session_id: "sub_msg",
			role: "user",
			content: "hi",
			created_at: new Date().toISOString(),
		});
		expect(subSessionStore.messagesOf("sub_msg")).toHaveLength(1);
	});
});

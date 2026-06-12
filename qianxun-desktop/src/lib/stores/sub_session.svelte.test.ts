// ───────────────────────────────────────────────────────────────────────────
// SubSessionStore — 2026-06-12 收尾测试
//
// 测试覆盖:
//   1. loadAll 调 listSubSessions + 订阅 sub_session_event (E2E Round 1 修复后真接)
//   2. realtime SubSessionUpdate 事件 upsert 实体 (保留已有 messages)
//   3. byPlan 拿得到的 sub_session 可 open (之前永远 [] 死链)
//   4. API 表面 (isActive / isReadOnly / canSend / byPlan / countByPlan) 保持兼容
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

const listSubSessionsMock = vi.fn();
const subscribeSubSessionEventsMock = vi.fn();
const onSubSessionEventMock = vi.fn();
let _capturedHandler: ((payload: unknown) => void) | null = null;

vi.mock("$lib/ipc/runtime", () => ({
	listSubSessions: (...args: unknown[]) => listSubSessionsMock(...args),
	subscribeSubSessionEvents: (...args: unknown[]) =>
		subscribeSubSessionEventsMock(...args),
	onSubSessionEvent: (...args: unknown[]) => {
		_capturedHandler = args[0] as (payload: unknown) => void;
		return onSubSessionEventMock(...args);
	},
}));

import { subSessionStore } from "$lib/stores/sub_session.svelte";
import { uiStore } from "$lib/stores/ui.svelte";
import type { SubSession } from "$lib/types/entity";

const FAKE_INFO = {
	id: "sub_001",
	plan_id: "plan_jwt",
	parent_session_id: "sess_jwt_auth",
	task_id: "task_a",
	role: "coder",
	status: "active" as const,
	started_at: "2026-06-08T00:00:00Z",
	ended_at: null,
	output: null,
};

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

describe("SubSessionStore (2026-06-12 收尾)", () => {
function resetSubSessionStore() {
	subSessionStore.__resetForTesting();
	_capturedHandler = null;
}

beforeEach(() => {
	listSubSessionsMock.mockReset();
	subscribeSubSessionEventsMock.mockReset().mockResolvedValue(undefined);
	onSubSessionEventMock.mockReset().mockResolvedValue(() => {});
	resetSubSessionStore();
	uiStore.setActiveView({ kind: "empty" });
});

	it("loadAll_fetches_and_subscribes: 真接后端 listSubSessions + subscribe", async () => {
		listSubSessionsMock.mockResolvedValue([FAKE_INFO]);
		await subSessionStore.loadAll();
		expect(listSubSessionsMock).toHaveBeenCalledOnce();
		expect(subscribeSubSessionEventsMock).toHaveBeenCalledOnce();
		expect(onSubSessionEventMock).toHaveBeenCalledOnce();
		expect(subSessionStore.initialized).toBe(true);
		expect(subSessionStore.all).toHaveLength(1);
		// 后端 snake_case + status active → 前端 PascalCase Active
		expect(subSessionStore.all[0]!.status).toBe("Active");
		expect(subSessionStore.all[0]!.plan_task_id).toBe("task_a");
	});

	it("loadAll_empty_list_keeps_initialized_true: 空 store 也标 ready", async () => {
		listSubSessionsMock.mockResolvedValue([]);
		await subSessionStore.loadAll();
		expect(subSessionStore.initialized).toBe(true);
		expect(subSessionStore.all).toEqual([]);
	});

	it("realtime_sub_session_update_upserts: E2E Round 1 根因修复, 事件来时插入", async () => {
		listSubSessionsMock.mockResolvedValue([]);
		await subSessionStore.loadAll();
		// 收到 SubSessionUpdate 事件 (后端 emit, execute_one_task 启动时触发)
		expect(_capturedHandler).toBeTruthy();
		_capturedHandler!({
			type: "sub_session_update",
			sub_session_id: "sub_new",
			plan_id: "plan_x",
			task_id: "task_1",
			status: "active",
			sub_session_json: JSON.stringify({ ...FAKE_INFO, id: "sub_new", plan_id: "plan_x", task_id: "task_1" }),
			updated_at: Date.now(),
		});
		expect(subSessionStore.all).toHaveLength(1);
		expect(subSessionStore.get("sub_new")!.status).toBe("Active");
	});

	it("realtime_update_preserves_existing_messages: 事件 JSON 不带 messages, 不应清空", async () => {
		listSubSessionsMock.mockResolvedValue([FAKE_INFO]);
		await subSessionStore.loadAll();
		// 模拟业务追加 messages
		subSessionStore.appendMessage("sub_001", {
			id: "msg_1",
			session_id: null,
			sub_session_id: "sub_001",
			role: "assistant",
			content: "hello",
			created_at: new Date().toISOString(),
		});
		// 收到 update 事件 (后端 task 完成时, status 变 done)
		_capturedHandler!({
			type: "sub_session_update",
			sub_session_id: "sub_001",
			plan_id: "plan_jwt",
			task_id: "task_a",
			status: "done",
			sub_session_json: JSON.stringify({ ...FAKE_INFO, status: "done" }),
			updated_at: Date.now(),
		});
		const got = subSessionStore.get("sub_001")!;
		expect(got.status).toBe("Done");
		// messages 不能丢
		expect(got.messages).toHaveLength(1);
	});

	it("open_routes_to_uiStore_with_real_sub: E2E 死链修复, byPlan 拿得到就能跳", async () => {
		listSubSessionsMock.mockResolvedValue([FAKE_INFO]);
		await subSessionStore.loadAll();
		const subs = subSessionStore.byPlan("plan_jwt");
		expect(subs).toHaveLength(1);
		subSessionStore.open(subs[0]!.id);
		expect(uiStore.activeView.kind).toBe("sub_session");
		if (uiStore.activeView.kind === "sub_session") {
			expect(uiStore.activeView.sub_session_id).toBe("sub_001");
			expect(uiStore.activeView.parent_session_id).toBe("sess_jwt_auth");
		}
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

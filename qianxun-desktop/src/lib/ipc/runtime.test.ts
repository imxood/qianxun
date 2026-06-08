// ───────────────────────────────────────────────────────────────────────────
// Runtime IPC 客户端 — web fallback 路径测试
//
// 跟 bridge.test.ts 同样思路: jsdom 默认无 __TAURI_INTERNALS__, isTauri() 返 false,
// 单元测试只测 web fallback. Tauri 容器路径依赖真实 Rust 后端, 留 E2E 覆盖.
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi, beforeEach } from "vitest";

// 全局 mock @tauri-apps/api/core + event, 阻止 vi.mock 不到时 invoke 真发请求
vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

import { listSessions, sendMessage, createPlan, cancelSession, loadSession } from "./runtime";
import { RuntimeApiError } from "./runtime";

describe("runtime ipc (web fallback path)", () => {
	beforeEach(() => {
		// 清理 @tauri-apps/api/core.invoke 状态
		vi.clearAllMocks();
		// 重置模块缓存, 让 isTauri() 重新求值 (虽然 jsdom 默认 false, 但保持跟 bridge.test.ts 一致)
		vi.resetModules();
	});

	it("listSessions_web_fallback: 返空 sessions + filter 回显", async () => {
		const r = await listSessions("active");
		expect(r.sessions).toEqual([]);
		expect(r.total).toBe(0);
		expect(r.active_in_memory).toBe(0);
		expect(r.paused_in_memory).toBe(0);
		expect(r.filter).toBe("active");
	});

	it("listSessions_web_fallback: filter='all' 也工作", async () => {
		const r = await listSessions("all");
		expect(r.filter).toBe("all");
		expect(r.sessions).toEqual([]);
	});

	it("sendMessage_web_fallback: 返 streaming SendResponse, 不调 invoke", async () => {
		const r = await sendMessage("sess_xxx", {
			messages: [{ role: "user", content: "hi" }],
		});
		expect(r.session_id).toBe("sess_xxx");
		expect(r.status).toBe("streaming");
	});

	it("createPlan_web_fallback: 返 mock PlanInfo (id 用 mock_ 前缀)", async () => {
		const r = await createPlan({
			session_id: "sess_xxx",
			name: "test plan",
			description: "desc",
		});
		expect(r.id.startsWith("mock_plan_")).toBe(true);
		expect(r.session_id).toBe("sess_xxx");
		expect(r.name).toBe("test plan");
		expect(r.status).toBe("running");
		expect(r.ended_at).toBeNull();
	});

	it("cancelSession_web_fallback: noop 不抛", async () => {
		await expect(cancelSession("sess_xxx")).resolves.toBeUndefined();
	});

	it("loadSession_web_fallback: 返 Stored 状态, 空 conversation", async () => {
		const r = await loadSession("sess_xxx");
		expect(r.session_id).toBe("sess_xxx");
		expect(r.exists_in_memory).toBe(false);
		expect(r.status).toBe("stored");
		expect(r.conversation_json).toBeNull();
		expect(r.message_count).toBe(0);
	});
});

describe("RuntimeApiError.parse", () => {
	it("'not found: xxx' → NotFound", () => {
		const e = RuntimeApiError.parse("not found: session sess_xxx not found");
		expect(e.code).toBe("NotFound");
		expect(e.message).toContain("not found");
	});

	it("'invalid request: xxx' → InvalidRequest", () => {
		const e = RuntimeApiError.parse("invalid request: empty messages");
		expect(e.code).toBe("InvalidRequest");
	});

	it("'internal error: xxx' → Internal", () => {
		const e = RuntimeApiError.parse("internal error: SQLite open failed");
		expect(e.code).toBe("Internal");
	});

	it("'unavailable: xxx' → Unavailable", () => {
		const e = RuntimeApiError.parse("unavailable: max sessions reached");
		expect(e.code).toBe("Unavailable");
	});

	it("未知前缀 → fallback Internal", () => {
		const e = RuntimeApiError.parse("some weird error");
		expect(e.code).toBe("Internal");
	});
});

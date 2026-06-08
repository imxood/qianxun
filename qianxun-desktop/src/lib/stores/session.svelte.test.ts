// ───────────────────────────────────────────────────────────────────────────
// SessionStore — Stage 4a (sub-task #4) 切真后端测试
//
// 测试覆盖 (mock $lib/ipc/runtime):
//   1. init() 调 listSessions 拉真实列表, 转换 SessionInfo → Session
//   2. init() 失败时 lastError 设置, sessions 保留空
//   3. init() 重复调用去重
//   4. refresh() 强制重拉 (绕开 initialized)
//   5. create() 客户端建占位, UI 能用
//   6. loadFullSession() 调 ipc loadSession, 更新本地 message_count
//   7. switchTo() 自动调 loadFullSession (fire-and-forget)
//   8. sessionInfoToSession 转换: status 映射 lowercase → PascalCase
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

const listSessionsMock = vi.fn();
const loadSessionMock = vi.fn();
vi.mock("$lib/ipc/runtime", () => ({
	listSessions: (...args: unknown[]) => listSessionsMock(...args),
	loadSession: (...args: unknown[]) => loadSessionMock(...args),
}));

import { sessionStore } from "$lib/stores/session.svelte";
import { uiStore } from "$lib/stores/ui.svelte";
import type { SessionInfo } from "$lib/ipc/runtime";

const FAKE_INFOS: SessionInfo[] = [
	{
		id: "sess_20260608_001_aaa",
		model: "deepseek-v4-flash",
		status: "active",
		created_at: "2026-06-08T00:00:00Z",
		last_active_at: "2026-06-08T01:00:00Z",
		message_count: 5,
	},
	{
		id: "sess_20260608_002_bbb",
		model: "deepseek-v4-flash",
		status: "paused",
		created_at: "2026-06-07T00:00:00Z",
		last_active_at: "2026-06-07T12:00:00Z",
		message_count: 12,
	},
	{
		id: "sess_20260608_003_ccc",
		model: "deepseek-v4-flash",
		status: "stored",
		created_at: "2026-06-06T00:00:00Z",
		last_active_at: "2026-06-06T00:00:00Z",
		message_count: 0,
	},
];

describe("SessionStore (Stage 4a sub-task #4 切 invoke)", () => {
	beforeEach(() => {
		listSessionsMock.mockReset();
		loadSessionMock.mockReset();
		// 重置 sessionStore 状态 (单例, 跨测试要清)
		sessionStore.all.length = 0;
		// 用类型 any 强穿私有字段 (测试场景可接受)
		(sessionStore as unknown as { initialized: boolean }).initialized = false;
		(sessionStore as unknown as { loading: boolean }).loading = false;
		(sessionStore as unknown as { lastError: string | null }).lastError = null;
		// ui 重置
		uiStore.setActiveView({ kind: "empty" });
	});

	it("init_pulls_from_runtime_and_converts_info_to_session: 拉真后端 + lowercase→PascalCase 状态映射", async () => {
		listSessionsMock.mockResolvedValueOnce({
			sessions: FAKE_INFOS,
			total: 3,
			filter: "all",
			active_in_memory: 1,
			paused_in_memory: 1,
		});

		await sessionStore.init();

		expect(listSessionsMock).toHaveBeenCalledWith("all");
		expect(sessionStore.all).toHaveLength(3);
		// active → Active
		expect(sessionStore.all[0]?.status).toBe("Active");
		// paused → Idle
		expect(sessionStore.all[1]?.status).toBe("Idle");
		// stored → Archived
		expect(sessionStore.all[2]?.status).toBe("Archived");
		// 兜底字段
		expect(sessionStore.all[0]?.provider).toBe("deepseek");
		expect(sessionStore.all[0]?.owner_id).toBe("u_1");
		expect(sessionStore.all[0]?.project_id).toBeNull();
		expect(sessionStore.all[0]?.title).toBe("sess_20260608_001_aaa");
		expect(sessionStore.initialized).toBe(true);
		expect(sessionStore.lastError).toBeNull();
	});

	it("init_failure_sets_lastError_and_keeps_sessions_empty: 失败容错", async () => {
		listSessionsMock.mockRejectedValueOnce(new Error("backend offline"));

		await sessionStore.init();

		expect(sessionStore.all).toHaveLength(0);
		expect(sessionStore.initialized).toBe(false);
		expect(sessionStore.lastError).toBe("backend offline");
	});

	it("init_is_idempotent: 重复调用只发一次 invoke", async () => {
		listSessionsMock.mockResolvedValue({
			sessions: FAKE_INFOS,
			total: 3,
			filter: "all",
			active_in_memory: 1,
			paused_in_memory: 1,
		});

		await sessionStore.init();
		await sessionStore.init();
		await sessionStore.init();

		expect(listSessionsMock).toHaveBeenCalledTimes(1);
	});

	it("refresh_forces_reinit: 绕过 initialized 检查", async () => {
		listSessionsMock.mockResolvedValueOnce({
			sessions: FAKE_INFOS,
			total: 3,
			filter: "all",
			active_in_memory: 1,
			paused_in_memory: 1,
		});
		await sessionStore.init();
		expect(listSessionsMock).toHaveBeenCalledTimes(1);

		listSessionsMock.mockResolvedValueOnce({
			sessions: [FAKE_INFOS[0]!],
			total: 1,
			filter: "all",
			active_in_memory: 1,
			paused_in_memory: 0,
		});
		await sessionStore.refresh();
		expect(listSessionsMock).toHaveBeenCalledTimes(2);
		expect(sessionStore.all).toHaveLength(1);
	});

	it("create_creates_client_side_placeholder: 后端暂没 create_session, 客户端建 + UI 能用", () => {
		const s = sessionStore.create({ project_id: null, title: "我的新会话" });
		expect(s.id.startsWith("sess_")).toBe(true);
		expect(s.title).toBe("我的新会话");
		expect(sessionStore.get(s.id)).toBeDefined();
		expect(sessionStore.getMessages(s.id)).toEqual([]);
	});

	it("loadFullSession_calls_ipc_and_updates_message_count: 切 session 时拉", async () => {
		listSessionsMock.mockResolvedValueOnce({
			sessions: FAKE_INFOS,
			total: 3,
			filter: "all",
			active_in_memory: 1,
			paused_in_memory: 1,
		});
		await sessionStore.init();

		loadSessionMock.mockResolvedValueOnce({
			session_id: "sess_20260608_001_aaa",
			exists_in_memory: true,
			status: "active",
			conversation_json: null,
			message_count: 99, // 后端返新计数
		});

		const state = await sessionStore.loadFullSession("sess_20260608_001_aaa");
		expect(state.message_count).toBe(99);
		expect(sessionStore.get("sess_20260608_001_aaa")?.message_count).toBe(99);
	});

	it("loadFullSession_propagates_error: 失败时 lastError + throw", async () => {
		loadSessionMock.mockRejectedValueOnce(new Error("not found: session sess_xxx not found"));

		await expect(sessionStore.loadFullSession("sess_xxx")).rejects.toThrow();
		expect(sessionStore.lastError).toContain("not found");
	});

	it("switchTo_triggers_loadFullSession_fire_and_forget: 切 session 时自动拉", async () => {
		listSessionsMock.mockResolvedValueOnce({
			sessions: FAKE_INFOS,
			total: 3,
			filter: "all",
			active_in_memory: 1,
			paused_in_memory: 1,
		});
		await sessionStore.init();

		loadSessionMock.mockResolvedValueOnce({
			session_id: "sess_20260608_001_aaa",
			exists_in_memory: true,
			status: "active",
			conversation_json: null,
			message_count: 7,
		});

		sessionStore.switchTo("sess_20260608_001_aaa");
		// switchTo 同步返, loadFullSession 异步 fire-and-forget
		// 给 microtask 一个 tick 让 promise 解析
		await new Promise((r) => setTimeout(r, 0));

		expect(loadSessionMock).toHaveBeenCalledWith("sess_20260608_001_aaa");
		expect(uiStore.activeView.kind).toBe("session");
	});
});

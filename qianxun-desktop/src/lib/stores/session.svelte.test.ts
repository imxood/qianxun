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
const createSessionMock = vi.fn();
vi.mock("$lib/ipc/runtime", () => ({
	listSessions: (...args: unknown[]) => listSessionsMock(...args),
	loadSession: (...args: unknown[]) => loadSessionMock(...args),
	createSession: (...args: unknown[]) => createSessionMock(...args),
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
// 重置 sessionStore 内部状态 (调 __resetForTesting 测试专用方法)
function resetSessionStore() {
	sessionStore.__resetForTesting();
}

beforeEach(() => {
	listSessionsMock.mockReset();
	loadSessionMock.mockReset();
	resetSessionStore();
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
		// title 兜底: id > 20 字符截断 + ellipsis (id 22 字符)
		expect(sessionStore.all[0]?.title).toBe("sess_20260608_001_aa…");
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

	it("create_calls_invoke_and_pushes_store: create() 调 createSession invoke 拿后端 ID", async () => {
		// mock createSession invoke 返后端生成的 sess_ ID
		(createSessionMock as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
			id: "sess_20260609_120000_123456",
			model: "deepseek-v4-flash",
			status: "active",
			created_at: new Date().toISOString(),
			last_active_at: new Date().toISOString(),
			message_count: 0,
		});
		const s = await sessionStore.create({ project_id: null, title: "我的新会话" });
		expect(createSessionMock).toHaveBeenCalledTimes(1);
		expect(s.id).toBe("sess_20260609_120000_123456");
		// title 来自 sessionInfoToSession 兜底 (id 长度 > 20 截断), 跟旧"client-only 占位"行为变化
		expect(s.title).toBe("sess_20260609_120000…");
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

		// 2026-06-12 (批次 1.2): 签名改 Promise<void>, 不再返 state; message_count 走 sessionStore.get()
		await sessionStore.loadFullSession("sess_20260608_001_aaa");
		expect(sessionStore.get("sess_20260608_001_aaa")?.message_count).toBe(99);
	});

	it("loadFullSession_no_throw_no_lastError_on_failure: 失败走 reportError 单一路", async () => {
		// 2026-06-12 (批次 1.2): 错误路径合并 — 不 throw, 不独立设 lastError.
		// 调用方 (switchTo) 不用 .catch(() => {}), UI 通过 loadingFull 派生显示骨架屏.
		loadSessionMock.mockRejectedValueOnce(new Error("not found: session sess_xxx not found"));

		await expect(sessionStore.loadFullSession("sess_xxx")).resolves.toBeUndefined();
		expect(sessionStore.lastError).toBeNull(); // 不再独立设 lastError
	});

	it("loadFullSession_sets_and_clears_loading_state: loading 标记 finally 必清", async () => {
		// 2026-06-12 (批次 1.2): loadingFull Set 反映 loadFullSession 进行中, finally 必清.
		loadSessionMock.mockImplementationOnce(
			() =>
				new Promise((resolve) =>
					setTimeout(
						() =>
							resolve({
								session_id: "sess_20260608_001_aaa",
								exists_in_memory: true,
								status: "active",
								conversation_json: null,
								message_count: 1,
							}),
						20,
					),
				),
		);

		const promise = sessionStore.loadFullSession("sess_20260608_001_aaa");
		// 进行中: 集合里应该有这个 id
		expect(sessionStore.loadingFull.has("sess_20260608_001_aaa")).toBe(true);
		await promise;
		// 完成后: 集合里应该清掉
		expect(sessionStore.loadingFull.has("sess_20260608_001_aaa")).toBe(false);
	});

	it("loadFullSession_clears_loading_state_even_on_failure: finally 必清 (失败路径)", async () => {
		// 2026-06-12 (批次 1.2): 失败路径也走 finally, 不残留 loading 标记.
		loadSessionMock.mockRejectedValueOnce(new Error("backend offline"));

		await sessionStore.loadFullSession("sess_xxx");
		expect(sessionStore.loadingFull.has("sess_xxx")).toBe(false);
	});

	it("loadFullSession_parses_conversation_json_and_idempotent: conversation_json 解析 + created_at 幂等", async () => {
		// 2026-06-12 (批次 1.1 + 1.4): conversation_json 解析走 parseConversationJsonl
		// (损坏行 skip, system header 宽松匹配), 二次 loadFullSession 保留原 created_at.
		const conversationJsonl = [
			'{"type": "system", "prompt": "You are helpful"}',
			'{"User": {"id": "m1", "content": [{"type": "text", "text": "hi"}]}}',
			'{"Assistant": {"id": "m2", "content": [{"type": "text", "text": "hello"}]}}',
		].join("\n");

		loadSessionMock.mockResolvedValueOnce({
			session_id: "sess_20260608_001_aaa",
			exists_in_memory: true,
			status: "active",
			conversation_json: conversationJsonl,
			message_count: 2,
		});
		await sessionStore.loadFullSession("sess_20260608_001_aaa");

		const msgs1 = sessionStore.getMessages("sess_20260608_001_aaa");
		expect(msgs1).toHaveLength(2);
		expect(msgs1[0]?.role).toBe("user");
		expect(msgs1[0]?.content).toBe("hi");
		expect(msgs1[1]?.role).toBe("assistant");
		expect(msgs1[1]?.content).toBe("hello");
		const originalCreatedAt1 = msgs1[0]?.created_at;

		// 二次 loadFullSession: 同样 conversation_json, 期望 created_at 不被刷新
		// (等 5ms 让 now 推进, 这样如果幂等没生效, created_at 会变化)
		await new Promise((r) => setTimeout(r, 5));
		loadSessionMock.mockResolvedValueOnce({
			session_id: "sess_20260608_001_aaa",
			exists_in_memory: true,
			status: "active",
			conversation_json: conversationJsonl,
			message_count: 2,
		});
		await sessionStore.loadFullSession("sess_20260608_001_aaa");

		const msgs2 = sessionStore.getMessages("sess_20260608_001_aaa");
		expect(msgs2[0]?.created_at).toBe(originalCreatedAt1); // 幂等保留
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

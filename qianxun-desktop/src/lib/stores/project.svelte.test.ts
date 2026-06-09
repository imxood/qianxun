// ───────────────────────────────────────────────────────────────────────────
// ProjectStore — 2026-06-09 改: 调 listSessions('all') 按 project_root 去重 derive Project[].
//
// 测试覆盖:
//   1. loadAll 调 listSessions, derive 出 project 列表
//   2. byId / get 返对应 project
//   3. loadAll 重复调用去重
//   4. project_root 为 null 的 session 不出现在 projects 里
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

const listSessionsMock = vi.fn();
vi.mock("$lib/ipc/runtime", () => ({
	listSessions: (...args: unknown[]) => listSessionsMock(...args),
}));

import { projectStore } from "$lib/stores/project.svelte";
import type { SessionInfo } from "$lib/ipc/runtime";

const fakeSessions: SessionInfo[] = [
	{
		id: "sess_1",
		model: "deepseek-v4-flash",
		status: "active",
		created_at: "2026-06-09T00:00:00Z",
		last_active_at: "2026-06-09T01:00:00Z",
		message_count: 3,
		project_root: "/home/maxu/qianxun",
	},
	{
		id: "sess_2",
		model: "deepseek-v4-flash",
		status: "active",
		created_at: "2026-06-08T00:00:00Z",
		last_active_at: "2026-06-09T02:00:00Z",
		message_count: 5,
		project_root: "/home/maxu/qianxun", // 同一项目, 应去重
	},
	{
		id: "sess_3",
		model: "deepseek-v4-flash",
		status: "active",
		created_at: "2026-06-07T00:00:00Z",
		last_active_at: "2026-06-08T12:00:00Z",
		message_count: 1,
		project_root: null, // 无项目, 应跳过
	},
	{
		id: "sess_4",
		model: "deepseek-v4-flash",
		status: "active",
		created_at: "2026-06-06T00:00:00Z",
		last_active_at: "2026-06-07T08:00:00Z",
		message_count: 2,
		project_root: "/tmp/another",
	},
];

describe("ProjectStore (2026-06-09 derive from sessions)", () => {
	function resetProjectStore() {
		projectStore.__resetForTesting();
	}

	beforeEach(() => {
		listSessionsMock.mockReset();
		resetProjectStore();
	});

	it("loadAll_calls_listSessions_and_derives_projects", async () => {
		listSessionsMock.mockResolvedValueOnce({
			sessions: fakeSessions,
			total: 4,
			filter: "all",
			active_in_memory: 4,
			paused_in_memory: 0,
		});
		await projectStore.loadAll();
		expect(listSessionsMock).toHaveBeenCalledTimes(1);
		expect(listSessionsMock).toHaveBeenCalledWith("all");
		expect(projectStore.initialized).toBe(true);
		expect(projectStore.all).toHaveLength(2); // 2 个去重项目 (sess_3 跳过)
	});

	it("derived_projects_dedup_by_project_root_and_count_sessions", async () => {
		listSessionsMock.mockResolvedValueOnce({
			sessions: fakeSessions,
			total: 4,
			filter: "all",
			active_in_memory: 4,
			paused_in_memory: 0,
		});
		await projectStore.loadAll();
		const qianxun = projectStore.byId("/home/maxu/qianxun");
		expect(qianxun).toBeDefined();
		expect(qianxun!.session_count).toBe(2); // sess_1 + sess_2
		expect(qianxun!.name).toBe("qianxun"); // path 末段
	});

	it("sorts_projects_by_last_active_at_desc", async () => {
		listSessionsMock.mockResolvedValueOnce({
			sessions: fakeSessions,
			total: 4,
			filter: "all",
			active_in_memory: 4,
			paused_in_memory: 0,
		});
		await projectStore.loadAll();
		// qianxun (2026-06-09T02:00) 比 /tmp/another (2026-06-07T08:00) 活跃 → qianxun 在前
		expect(projectStore.all[0].id).toBe("/home/maxu/qianxun");
		expect(projectStore.all[1].id).toBe("/tmp/another");
	});

	it("byId_and_get_return_undefined_for_unknown_id: 未知项目查询安全", () => {
		expect(projectStore.byId("/unknown/path")).toBeUndefined();
		expect(projectStore.get("/unknown/path")).toBeUndefined();
	});

	it("loadAll_idempotent: 重复调用只调 1 次 invoke", async () => {
		listSessionsMock.mockResolvedValue({
			sessions: [],
			total: 0,
			filter: "all",
			active_in_memory: 0,
			paused_in_memory: 0,
		});
		await projectStore.loadAll();
		await projectStore.loadAll();
		await projectStore.loadAll();
		expect(projectStore.initialized).toBe(true);
		expect(listSessionsMock).toHaveBeenCalledTimes(1);
	});
});

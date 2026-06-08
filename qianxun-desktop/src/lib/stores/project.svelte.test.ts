// ───────────────────────────────────────────────────────────────────────────
// ProjectStore — Stage 4a (sub-task #4) 测试
//
// 测试覆盖:
//   1. loadAll 当前 noop (RuntimeApi 暂没 list_projects)
//   2. byId / get 返 undefined for 空列表
//   3. loadAll 重复调用去重
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

const listProjectsMock = vi.fn();
vi.mock("$lib/ipc/runtime", () => ({
	listProjects: (...args: unknown[]) => listProjectsMock(...args),
}));

import { projectStore } from "$lib/stores/project.svelte";

describe("ProjectStore (Stage 4a sub-task #4)", () => {
	beforeEach(() => {
		listProjectsMock.mockReset();
		projectStore.all.length = 0;
		(projectStore as unknown as { initialized: boolean }).initialized = false;
		(projectStore as unknown as { loading: boolean }).loading = false;
	});

	it("loadAll_is_currently_noop: 后端 RuntimeApi 暂没 list_projects", async () => {
		await projectStore.loadAll();
		// 没 invoke, 没 listProjects mock 被调
		expect(listProjectsMock).not.toHaveBeenCalled();
		expect(projectStore.initialized).toBe(true);
		expect(projectStore.all).toHaveLength(0);
	});

	it("byId_returns_undefined_for_empty_list: 空状态查询安全", () => {
		expect(projectStore.byId("proj_xxx")).toBeUndefined();
		expect(projectStore.get("proj_xxx")).toBeUndefined();
	});

	it("loadAll_idempotent: 重复调用不重复触发", async () => {
		await projectStore.loadAll();
		await projectStore.loadAll();
		await projectStore.loadAll();
		expect(projectStore.initialized).toBe(true);
		// mock 也没被调, 1 次也没
		expect(listProjectsMock).not.toHaveBeenCalled();
	});
});

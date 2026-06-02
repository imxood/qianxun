// ───────────────────────────────────────────────────────────────────────────
// sqlite.ts — Stage 5 §11.2 项目/会话缓存层测试
// 与 docs/30_子项目规划/03-tauri-desktop.md §11.2 一致
//
// 测试目标: web 模式 (jsdom 无 __TAURI_INTERNALS__) cacheProjects → getCachedProjects
// 往返一致, 失败时返回 null 不抛.
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";
import type { Project, Session } from "$lib/types/ipc";

describe("sqlite.ts 缓存层 (Stage 5 §11.2, web 模式)", () => {
	beforeEach(() => {
		// 清空 localStorage
		localStorage.clear();
		// 隔离模块 (import 不会重新执行, 但 isTauri() 缓存了也不影响 — jsdom 没有 __TAURI_INTERNALS__)
		vi.resetModules();
	});

	it("cacheProjects → getCachedProjects 往返一致", async () => {
		const { cacheProjects, getCachedProjects } = await import("$lib/db/sqlite");

		const sample: Project[] = [
			{
				id: "proj_1",
				name: "qianxun",
				path: "E:/git/maxu/qianxun",
				owner_id: "u_1",
				team_id: "team_1",
				created_at: "2026-06-01T08:30:00Z",
			},
			{
				id: "proj_2",
				name: "qianxun-desktop",
				path: "E:/git/maxu/qianxun/qianxun-desktop",
				owner_id: "u_1",
				team_id: "team_1",
				created_at: "2026-06-01T10:00:00Z",
			},
		];

		// 写
		await cacheProjects(sample);
		expect(localStorage.getItem("qianxun.cache.projects")).toBeTruthy();

		// 读
		const got = await getCachedProjects();
		expect(got).toEqual(sample);
		expect(got).toHaveLength(2);
		expect(got?.[0]?.name).toBe("qianxun");
	});

	it("cacheSessions(projectId) → getCachedSessions(projectId) 按项目分键", async () => {
		const { cacheSessions, getCachedSessions } = await import("$lib/db/sqlite");

		const s1: Session[] = [
			{
				id: "sess_1",
				project_id: "proj_1",
				title: "Daemon 设计",
				model: "deepseek-v4-flash",
				status: "active",
				owner_id: "u_1",
				created_at: "2026-06-01T11:00:00Z",
				last_active_at: "2026-06-02T09:00:00Z",
				message_count: 12,
			},
		];
		const s2: Session[] = [
			{
				id: "sess_2",
				project_id: "proj_2",
				title: "Tauri Stage 1",
				model: "deepseek-v4-flash",
				status: "idle",
				owner_id: "u_1",
				created_at: "2026-06-02T00:00:00Z",
				last_active_at: "2026-06-02T10:00:00Z",
				message_count: 4,
			},
		];

		await cacheSessions("proj_1", s1);
		await cacheSessions("proj_2", s2);

		const got1 = await getCachedSessions("proj_1");
		const got2 = await getCachedSessions("proj_2");
		expect(got1).toEqual(s1);
		expect(got2).toEqual(s2);
		// 不同 key 不会串
		expect(got1?.[0]?.id).toBe("sess_1");
		expect(got2?.[0]?.id).toBe("sess_2");
	});

	it("无缓存时 getCached* 返回 null, 不抛", async () => {
		const { getCachedProjects, getCachedSessions } = await import("$lib/db/sqlite");

		expect(await getCachedProjects()).toBeNull();
		expect(await getCachedSessions("unknown")).toBeNull();
	});
});

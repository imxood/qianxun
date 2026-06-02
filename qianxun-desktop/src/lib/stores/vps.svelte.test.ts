// ───────────────────────────────────────────────────────────────────────────
// VpsStore — 测试
//   Stage 4 §10.4: VPS WS 健康检查 (URL 状态机 + normalize)
//   Stage 6c: 3 个写操作真接 fetch (用 vi.spyOn(global, 'fetch') mock)
//
// 与 docs/30_子项目规划/03-tauri-desktop.md §10.4 + §9.3 一致.
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { vpsStore } from "$lib/stores/vps.svelte";
import { settingsStore } from "$lib/stores/settings.svelte";

describe("VpsStore (Stage 4 §10.4 + Stage 6c 真接 fetch)", () => {
	beforeEach(() => {
		localStorage.removeItem("qianxun.vps.url");
		localStorage.removeItem("qianxun.vps.token");
		vpsStore.vpsUrl = "";
		vpsStore.connectionState = "offline";
		vpsStore.lastError = null;
		vpsStore.stopHealthCheck();
		// 配置 settingsStore 让 vpsFetch 能拿到 base URL + token
		settingsStore.setVpsUrl("https://vps.example.com");
		settingsStore.setVpsToken("test-token-abc");
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("默认 connectionState='offline' 且 vpsUrl 为空", () => {
		// 重置 settingsStore 避免污染
		settingsStore.setVpsUrl("");
		settingsStore.setVpsToken("");
		expect(vpsStore.connectionState).toBe("offline");
		expect(vpsStore.vpsUrl).toBe("");
		expect(vpsStore.isDegraded).toBe(false);
	});

	it("未配 URL 时 isDegraded=false (VPS 是可选的)", () => {
		settingsStore.setVpsUrl("");
		vpsStore.vpsUrl = "";
		expect(vpsStore.isDegraded).toBe(false);
	});

	it("配了 URL 但未连接时 isDegraded=true", () => {
		vpsStore.vpsUrl = "https://vps.example.com";
		vpsStore.connectionState = "offline";
		expect(vpsStore.isDegraded).toBe(true);
	});

	it("normalizeUrl 转换 http(s):// → ws(s):// 并补 /hub", () => {
		expect(vpsStore.normalizeUrl("https://vps.example.com")).toBe(
			"wss://vps.example.com/hub"
		);
		expect(vpsStore.normalizeUrl("http://localhost:3000/")).toBe(
			"ws://localhost:3000/hub"
		);
		expect(vpsStore.normalizeUrl("wss://vps.example.com/hub")).toBe(
			"wss://vps.example.com/hub"
		);
		expect(vpsStore.normalizeUrl("wss://vps.example.com/other")).toBe(
			"wss://vps.example.com/other/hub"
		);
	});

	it("setVpsUrl 持久化到 localStorage", () => {
		vpsStore.setVpsUrl("https://vps.example.com");
		expect(localStorage.getItem("qianxun.vps.url")).toBe("https://vps.example.com");
		expect(vpsStore.vpsUrl).toBe("https://vps.example.com");
	});

	// ─── Stage 6c: 真接 fetch 测试 ────────────────────────────────────────

	it("test_inviteMember_real_fetch_sends_POST_with_Bearer: 真发 POST /api/teams/:id/members, 带 Authorization Bearer", async () => {
		const fetchSpy = vi.spyOn(global, "fetch").mockResolvedValue(
			new Response(JSON.stringify({ ok: true }), {
				status: 200,
				headers: { "Content-Type": "application/json" },
			})
		);

		await vpsStore.inviteMember("team_1", "u_alice", "Alice", "admin");

		expect(fetchSpy).toHaveBeenCalledTimes(1);
		const [url, init] = fetchSpy.mock.calls[0]!;
		expect(url).toBe("https://vps.example.com/api/teams/team_1/members");
		expect(init!.method).toBe("POST");
		const headers = init!.headers as Record<string, string>;
		expect(headers["Authorization"]).toBe("Bearer test-token-abc");
		expect(headers["Content-Type"]).toBe("application/json");
		const body = JSON.parse(init!.body as string);
		expect(body).toEqual({
			user_id: "u_alice",
			display_name: "Alice",
			role: "admin",
		});
	});

	it("test_inviteMember_http_error_throws: 后端 4xx/5xx → 抛 Error with status", async () => {
		vi.spyOn(global, "fetch").mockResolvedValue(
			new Response("conflict", { status: 409, statusText: "Conflict" })
		);

		await expect(
			vpsStore.inviteMember("team_1", "u_alice", "Alice", "admin")
		).rejects.toThrow(/inviteMember failed: HTTP 409/);
	});

	it("test_changeRole_real_fetch_sends_PATCH: 真发 PATCH /api/teams/:id/members/:uid", async () => {
		const fetchSpy = vi.spyOn(global, "fetch").mockResolvedValue(
			new Response("{}", { status: 200 })
		);

		await vpsStore.changeRole("team_1", "u_bob", "developer");

		expect(fetchSpy).toHaveBeenCalledTimes(1);
		const [url, init] = fetchSpy.mock.calls[0]!;
		expect(url).toBe("https://vps.example.com/api/teams/team_1/members/u_bob");
		expect(init!.method).toBe("PATCH");
		const body = JSON.parse(init!.body as string);
		expect(body).toEqual({ role: "developer" });
	});

	it("test_assignProject_real_fetch_sends_POST: 真发 POST /api/projects/:id/assign", async () => {
		const fetchSpy = vi.spyOn(global, "fetch").mockResolvedValue(
			new Response("{}", { status: 200 })
		);

		await vpsStore.assignProject("proj_1", "u_carol");

		expect(fetchSpy).toHaveBeenCalledTimes(1);
		const [url, init] = fetchSpy.mock.calls[0]!;
		expect(url).toBe("https://vps.example.com/api/projects/proj_1/assign");
		expect(init!.method).toBe("POST");
		const body = JSON.parse(init!.body as string);
		expect(body).toEqual({ user_id: "u_carol" });
	});

	it("test_vpsUrl_missing_throws_no_fetch: settingsStore.vpsUrl 为空 → 抛错且不发 fetch", async () => {
		settingsStore.setVpsUrl("");
		const fetchSpy = vi.spyOn(global, "fetch");

		await expect(
			vpsStore.inviteMember("team_1", "u_alice", "Alice", "admin")
		).rejects.toThrow(/vpsUrl 未配置/);
		expect(fetchSpy).not.toHaveBeenCalled();
	});

	it("test_vpsToken_missing_still_fetches_with_empty_Bearer: token 为空 → 仍发请求, Authorization 头为空串", async () => {
		settingsStore.setVpsToken("");
		const fetchSpy = vi.spyOn(global, "fetch").mockResolvedValue(
			new Response("{}", { status: 200 })
		);

		await vpsStore.inviteMember("team_1", "u_alice", "Alice", "admin");

		expect(fetchSpy).toHaveBeenCalledTimes(1);
		const [, init] = fetchSpy.mock.calls[0]!;
		const headers = init!.headers as Record<string, string>;
		expect(headers["Authorization"]).toBe("");
	});
});

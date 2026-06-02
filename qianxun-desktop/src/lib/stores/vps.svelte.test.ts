// ───────────────────────────────────────────────────────────────────────────
// VpsStore — Stage 4 VPS 接入骨架测试
// 与 docs/30_子项目规划/03-tauri-desktop.md §10.4 一致
//
// Stage 6b: 加 1 个写操作 mock 测试 (inviteMember → 本地 teamMembers 状态更新).
// 真实 fetch 测试留 Stage 6c.
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach } from "vitest";
import { vpsStore } from "$lib/stores/vps.svelte";

describe("VpsStore (Stage 4 §10.4 + Stage 6b 写操作 mock)", () => {
	beforeEach(() => {
		localStorage.removeItem("qianxun.vps.url");
		vpsStore.vpsUrl = "";
		vpsStore.connectionState = "offline";
		vpsStore.lastError = null;
		vpsStore.stopHealthCheck();
		vpsStore.__resetMockState();
	});

	it("默认 connectionState='offline' 且 vpsUrl 为空", () => {
		expect(vpsStore.connectionState).toBe("offline");
		expect(vpsStore.vpsUrl).toBe("");
		expect(vpsStore.isDegraded).toBe(false);
		// 未配 URL 不算降级
	});

	it("未配 URL 时 isDegraded=false (VPS 是可选的)", () => {
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

	// ─── Stage 6b: 团队/项目写操作 mock ───────────────────────────────────

	it("test_inviteMember_member_stores_role_change: inviteMember 把新成员追加到 teamMembers 且 role 正确", async () => {
		// 准备: 已有 1 个 owner
		const teamId = "team_1";
		vpsStore.teamMembers[teamId] = [
			{
				user_id: "u_owner",
				display_name: "owner",
				role: "owner",
				joined_at: "2026-06-01T00:00:00Z",
			},
		];

		// 动作: 邀请一个新 admin 成员
		await vpsStore.inviteMember(teamId, "u_alice", "Alice", "admin");

		// 验证: 列表包含新成员, role 是 admin
		const list = vpsStore.teamMembers[teamId];
		expect(list).toHaveLength(2);
		const alice = list?.find((m) => m.user_id === "u_alice");
		expect(alice).toBeDefined();
		expect(alice?.role).toBe("admin");
		expect(alice?.display_name).toBe("Alice");
		// 原有 owner 保持不变
		expect(list?.[0]?.user_id).toBe("u_owner");
		expect(list?.[0]?.role).toBe("owner");
	});
});

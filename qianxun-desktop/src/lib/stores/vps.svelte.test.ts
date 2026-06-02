// ───────────────────────────────────────────────────────────────────────────
// VpsStore — Stage 4 VPS 接入骨架测试
// 与 docs/30_子项目规划/03-tauri-desktop.md §10.4 一致
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach } from "vitest";
import { vpsStore } from "$lib/stores/vps.svelte";

describe("VpsStore (Stage 4 §10.4)", () => {
	beforeEach(() => {
		localStorage.removeItem("qianxun.vps.url");
		vpsStore.vpsUrl = "";
		vpsStore.connectionState = "offline";
		vpsStore.lastError = null;
		vpsStore.stopHealthCheck();
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
});

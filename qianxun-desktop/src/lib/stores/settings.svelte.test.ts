// ───────────────────────────────────────────────────────────────────────────
// SettingsStore — Stage 5 §11 Settings 模型持久化测试
// 与 docs/30_子项目规划/03-tauri-desktop.md §11.1 一致
//
// 测试 1: setTheme() 改 theme 字段, $effect 自动写 localStorage, 回读校验
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

describe("SettingsStore (Stage 5 §11.1)", () => {
	beforeEach(() => {
		localStorage.removeItem("qianxun-settings");
		// 隔离模块缓存, 让 SettingsStore 重新读 localStorage
		vi.resetModules();
	});

	it("setTheme() 后 localStorage 含新 theme 值", async () => {
		const { settingsStore } = await import("$lib/stores/settings.svelte");

		// 初始默认值
		expect(settingsStore.theme).toBe("system");
		expect(settingsStore.locale).toBe("zh-CN");
		expect(settingsStore.daemonUrl).toBe("http://127.0.0.1:23900");
		expect(settingsStore.vpsUrl).toBe("");

		settingsStore.setTheme("dark");

		// 字段已更新
		expect(settingsStore.theme).toBe("dark");

		// $effect 同步写 localStorage (可能需要微任务让 effect 跑一次)
		await new Promise((r) => setTimeout(r, 10));
		const raw = localStorage.getItem("qianxun-settings");
		expect(raw).toBeTruthy();
		const parsed = JSON.parse(raw!);
		expect(parsed.theme).toBe("dark");
		// 其他字段保留
		expect(parsed.locale).toBe("zh-CN");
		expect(parsed.daemonUrl).toBe("http://127.0.0.1:23900");
	});

	it("4 字段 setXxx 后, 完整快照写入 localStorage", async () => {
		const { settingsStore } = await import("$lib/stores/settings.svelte");

		settingsStore.setTheme("light");
		settingsStore.setLocale("en");
		settingsStore.setDaemonUrl("http://10.0.0.1:9999");
		settingsStore.setVpsUrl("https://vps.example.com");

		await new Promise((r) => setTimeout(r, 10));
		const raw = localStorage.getItem("qianxun-settings");
		const parsed = JSON.parse(raw!);
		expect(parsed).toEqual({
			theme: "light",
			locale: "en",
			daemonUrl: "http://10.0.0.1:9999",
			vpsUrl: "https://vps.example.com",
		});
	});
});

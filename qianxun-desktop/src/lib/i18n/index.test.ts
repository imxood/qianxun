// ───────────────────────────────────────────────────────────────────────────
// i18n — Stage 4 起步 10 key 测试
// 与 docs/30_子项目规划/03-tauri-desktop.md §4.6 一致
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach } from "vitest";
import { t, setLocale, currentLocale } from "$lib/i18n";

describe("i18n (Stage 4 §4.6)", () => {
	beforeEach(() => {
		// 清理 localStorage (避免 setLocale 副作用污染)
		localStorage.removeItem("qianxun.locale");
	});

	it("zh-CN 下 t('app.title') 返回 '千寻'", () => {
		setLocale("zh-CN");
		expect(t("app.title")).toBe("千寻");
		expect(t("input.send")).toBe("发送");
		expect(t("retry")).toBe("立即重试");
	});

	it("en 下 t('app.title') 返回 'Qianxun'", () => {
		setLocale("en");
		expect(t("app.title")).toBe("Qianxun");
		expect(t("input.send")).toBe("Send");
		expect(t("retry")).toBe("Retry now");
	});

	it("currentLocale 是 writable store, setLocale 立即生效", () => {
		setLocale("en");
		let observed: string | undefined;
		const unsub = currentLocale.subscribe((v) => (observed = v));
		expect(observed).toBe("en");
		unsub();
	});

	it("10 个 key 都存在 (zh-CN + en 各 10 个)", () => {
		const expectedKeys = [
			"app.title",
			"connection.connected",
			"connection.degraded",
			"connection.reconnecting",
			"connection.offline",
			"input.placeholder",
			"input.send",
			"message.thinking",
			"retry",
			"error.network",
		];
		setLocale("zh-CN");
		for (const k of expectedKeys) {
			const v = t(k as Parameters<typeof t>[0]);
			expect(v, `key ${k} (zh-CN) should be non-empty`).toBeTruthy();
			expect(v, `key ${k} (zh-CN) should not equal the key name`).not.toBe(k);
		}
		setLocale("en");
		for (const k of expectedKeys) {
			const v = t(k as Parameters<typeof t>[0]);
			expect(v, `key ${k} (en) should be non-empty`).toBeTruthy();
			expect(v, `key ${k} (en) should not equal the key name`).not.toBe(k);
		}
	});
});

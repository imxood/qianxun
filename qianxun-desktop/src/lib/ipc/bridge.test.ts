// ───────────────────────────────────────────────────────────────────────────
// IPC Bridge — Stage 6a §11.3 stronghold 凭据加密测试
//
// 测试覆盖:
//   1. Web 模式下 setSecret → localStorage 含 base64 value
//   2. Web 模式下 getSecret 密码匹配 → 返回原 value
//   3. Web 模式下 getSecret 密码错 → 返回 null
//
// (Tauri 容器内路径依赖真实 Rust 后端, 留 E2E 测试覆盖; 单元测试只测 web fallback.)
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

describe("bridge (Stage 6a §11.3 stronghold secret + Stage 10b delete)", () => {
	beforeEach(() => {
		// 清空 web fallback 用的 secret-* key
		for (let i = localStorage.length - 1; i >= 0; i--) {
			const k = localStorage.key(i);
			if (k && k.startsWith("secret-")) localStorage.removeItem(k);
		}
		// 隔离模块缓存, 让 isTauri() 在 web mock 下重新求值
		vi.resetModules();
	});

	it("test_setSecret_web_mode_uses_localStorage: 非 Tauri 模式下, setSecret → localStorage 含 base64 value", async () => {
		// jsdom 默认没有 __TAURI_INTERNALS__, isTauri() 自然返回 false
		const { setSecret } = await import("$lib/ipc/bridge");

		await setSecret("deepseek-api-key", "sk-test-12345", "user-password-1");

		const stored = localStorage.getItem("secret-deepseek-api-key");
		expect(stored).toBeTruthy();
		// btoa('sk-test-12345') === 'c2stdGVzdC0xMjM0NQ=='
		expect(stored).toBe("c2stdGVzdC0xMjM0NQ==");

		const storedPwd = localStorage.getItem("secret-deepseek-api-key-pwd");
		expect(storedPwd).toBeTruthy();
		expect(storedPwd).toBe("dXNlci1wYXNzd29yZC0x");
	});

	it("getSecret_web_mode: 密码正确时回读原 value", async () => {
		const { setSecret, getSecret } = await import("$lib/ipc/bridge");

		await setSecret("openai-api-key", "sk-openai-xyz", "hunter2");
		const value = await getSecret("openai-api-key", "hunter2");
		expect(value).toBe("sk-openai-xyz");
	});

	it("getSecret_web_mode: 密码错时返回 null", async () => {
		const { setSecret, getSecret } = await import("$lib/ipc/bridge");

		await setSecret("vps-access-token", "vps-tok", "correct-pwd");
		const value = await getSecret("vps-access-token", "wrong-pwd");
		expect(value).toBeNull();
	});

	it("getSecret_web_mode: key 不存在时返回 null", async () => {
		const { getSecret } = await import("$lib/ipc/bridge");

		const value = await getSecret("never-set-key", "any-pwd");
		expect(value).toBeNull();
	});

	// ─── Stage 10b 新增: deleteSecret 端到端 (web fallback 路径) ───

	it("deleteSecret_web_mode_existing_key: set → delete (密码对) → 返 'deleted', get 返 null", async () => {
		const { setSecret, deleteSecret, getSecret } = await import("$lib/ipc/bridge");
		await setSecret("anthropic-api-key", "sk-ant-to-delete", "user-pwd");
		expect(await getSecret("anthropic-api-key", "user-pwd")).toBe("sk-ant-to-delete");

		// 2026-06-12 (批次 2.7): 改返 DeleteOutcome 字符串, 替代 boolean
		const outcome = await deleteSecret("anthropic-api-key", "user-pwd");
		expect(outcome).toBe("deleted");

		// 删除后 localStorage 应清干净
		expect(localStorage.getItem("secret-anthropic-api-key")).toBeNull();
		expect(localStorage.getItem("secret-anthropic-api-key-pwd")).toBeNull();
		// get 应返 null
		expect(await getSecret("anthropic-api-key", "user-pwd")).toBeNull();
	});

	it("deleteSecret_web_mode_wrong_password: 密码错时返 'key_missing' (web 端合并), 不删除", async () => {
		const { setSecret, deleteSecret, getSecret } = await import("$lib/ipc/bridge");
		await setSecret("vps-token-2", "vps-secret", "right-pwd");

		// 2026-06-12 (批次 2.7): web fallback 把密码错归类为 key_missing (业务方无感,
		// 跟"没存"等价, 不暴露 password 错误细节); Tauri 端会 throw 让上层处理.
		const outcome = await deleteSecret("vps-token-2", "wrong-pwd");
		expect(outcome).toBe("key_missing");

		// 密码错不删除, 仍能正确取
		expect(await getSecret("vps-token-2", "right-pwd")).toBe("vps-secret");
	});

	it("deleteSecret_web_mode_nonexistent_key: key 不存在返 'key_missing', 不 panic", async () => {
		const { deleteSecret } = await import("$lib/ipc/bridge");
		const outcome = await deleteSecret("never-existed-key", "any-pwd");
		expect(outcome).toBe("key_missing");
	});
});

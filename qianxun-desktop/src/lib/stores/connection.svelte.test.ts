// ───────────────────────────────────────────────────────────────────────────
// ConnectionStore — Stage 4 状态机 + 与 SessionStore 离线入队集成测试
// 与 docs/30_子项目规划/03-tauri-desktop.md §10.1 / §10.3 一致
//
// 测试 1: isDegraded (offline | degraded) 时 sessionStore.send() 把消息入
//         offlineQueue, 不调 streamPrompt
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

// 必须在 import session.svelte 之前 mock $lib/sse/client
const streamPromptMock = vi.fn();
vi.mock("$lib/sse/client", () => ({
	streamPrompt: (...args: unknown[]) => streamPromptMock(...args),
	SseError: class SseError extends Error {
		constructor(public code: string, message: string) {
			super(message);
			this.name = "SseError";
		}
	},
}));

import { sessionStore } from "$lib/stores/session.svelte";
import { connectionStore } from "$lib/stores/connection.svelte";

describe("ConnectionStore.isDegraded 驱动 SessionStore.send (Stage 4 §10.3)", () => {
	beforeEach(() => {
		sessionStore.offlineQueue = [];
		sessionStore.clearOfflineQueue();
		sessionStore.reset();
		streamPromptMock.mockReset();
		// 重置 connectionStore 到 connected
		connectionStore.daemonState = "connected";
		connectionStore.lastError = null;
		connectionStore.attempt = 0;
	});

	it("isDegraded=true (offline) 时 send 入队, isDegraded=false (connected) 时正常发", async () => {
		// ── 场景 1: daemon 不可达 → 入队 ──────────────────────────────────
		connectionStore.daemonState = "offline";
		connectionStore.lastError = { ts: Date.now(), message: "daemon 不可达" };
		expect(connectionStore.isDegraded).toBe(true);

		await sessionStore.send("第一句", "MiniMax-M3");
		expect(sessionStore.offlineQueue).toHaveLength(1);
		expect(sessionStore.offlineQueue[0]?.text).toBe("第一句");
		expect(sessionStore.offlineQueue[0]?.attempts).toBe(0);
		expect(streamPromptMock).not.toHaveBeenCalled();

		// ── 场景 2: daemon 恢复 → 走正常流 ─────────────────────────────────
		connectionStore.daemonState = "connected";
		expect(connectionStore.isDegraded).toBe(false);

		streamPromptMock.mockResolvedValueOnce(undefined);
		await sessionStore.send("第二句", "MiniMax-M3");
		// 入队那条仍在 (flushOfflineQueue 由 +page.svelte 的定时器触发, 此处不调)
		expect(sessionStore.offlineQueue).toHaveLength(1);
		// 第二句走真发
		expect(streamPromptMock).toHaveBeenCalledTimes(1);
		expect(streamPromptMock).toHaveBeenCalledWith(
			expect.objectContaining({
				daemonUrl: connectionStore.daemonUrl,
				messages: [{ type: "text", text: "第二句" }],
				model: "MiniMax-M3",
			})
		);
	});

	it("isDegraded=true (degraded) 时也入队", async () => {
		connectionStore.daemonState = "degraded";
		connectionStore.lastError = { ts: Date.now(), message: "健康检查 3 次失败" };
		expect(connectionStore.isDegraded).toBe(true);

		await sessionStore.send("degraded 模式", "MiniMax-M3");
		expect(sessionStore.offlineQueue).toHaveLength(1);
		expect(sessionStore.offlineQueue[0]?.text).toBe("degraded 模式");
		expect(streamPromptMock).not.toHaveBeenCalled();
	});
});

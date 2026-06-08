// ───────────────────────────────────────────────────────────────────────────
// ConnectionStore — Stage 4 状态机测试 (sub-task #4 后)
//
// Stage 4a (sub-task #4) 切真后端 (Tauri 2.0) 后, 原"daemon 不可达时
// 消息入队 + streamPrompt 兜底" 的 HTTP 路径测试已不适用 — Tauri 是 in-process IPC,
// 没有网络失败, 不需要 offline queue.
//
// 保留 ConnectionStore 状态机的 4 态 (offline / reconnecting / degraded / connected)
// 单元测试, 跟 daemon 远程 health 探活路径配合 (Stage 6a fetchDaemonHealth).
// 离线入队 / streamPrompt 集成测试, 留后续 sub-task 重新设计 (如果引入 multi-runtime
// 跨进程场景).
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

vi.mock("$lib/ipc/bridge", () => ({
	fetchDaemonHealth: vi.fn(),
}));

import { connectionStore } from "$lib/stores/connection.svelte";
import { fetchDaemonHealth } from "$lib/ipc/bridge";

const mockFetch = vi.mocked(fetchDaemonHealth);

describe("ConnectionStore (Stage 4a sub-task #4 切 invoke 后)", () => {
	beforeEach(() => {
		mockFetch.mockReset();
		connectionStore.daemonState = "offline";
		connectionStore.lastError = null;
		connectionStore.attempt = 0;
	});

	it("default_state_is_offline: 初始 offline", () => {
		expect(connectionStore.daemonState).toBe("offline");
		expect(connectionStore.isDegraded).toBe(true);
	});

	it("markError_increments_attempt_and_keeps_reconnecting: 失败重试中", () => {
		mockFetch.mockResolvedValue({
			status: "offline",
			version: "unknown",
			uptime_sec: 0,
			session_count: 0,
			mcp_online: 0,
			provider_status: {},
		});
		void connectionStore.startHealthCheck();
		expect(connectionStore.daemonState).toBe("reconnecting");
	});

	it("successful_health_sets_connected: health 返 connected → 切状态", async () => {
		mockFetch.mockResolvedValue({
			status: "connected",
			version: "0.1.0",
			uptime_sec: 10,
			session_count: 0,
			mcp_online: 0,
			provider_status: { deepseek: "ok" },
		});
		// 直接调 retry(), 跳过 startHealthCheck 的 setInterval
		connectionStore.retry();
		// 异步, 等 microtask
		await new Promise((r) => setTimeout(r, 10));
		expect(connectionStore.daemonState).toBe("connected");
		expect(connectionStore.attempt).toBe(0);
	});
});

// ───────────────────────────────────────────────────────────────────────────
// ConnectionStore — Daemon 连接状态机
// 与 docs/30_子项目规划/03-tauri-desktop.md §4.1.2 / §7.1 / §10.1 完全一致
//
// 状态机 (4 态):
//   'offline'      — 从未连上 / 显式断开
//   'reconnecting' — 正在尝试 (UI 显示重试中)
//   'degraded'     — 已连但 health 异常 (如 health 端点返回 down)
//   'connected'    — 完全健康
//
// Stage 1: 简单 ping '/health' 端点, 端点不存在时切到 'offline'.
// Stage 2 (当前): 走 $lib/ipc/bridge.ts → Tauri 2.0 invoke('daemon_health_fetch')
//                  (Tauri 环境) / 浏览器 fetch fallback (Web 环境).
// ───────────────────────────────────────────────────────────────────────────

import type { DaemonState, HealthStatus } from "$lib/types/ipc";
import { fetchDaemonHealth } from "$lib/ipc/bridge";

const DEFAULT_DAEMON_URL = "http://127.0.0.1:23900";
const HEALTH_CHECK_INTERVAL_MS = 10_000; // §4.1.2: 10s 周期
const REQUEST_TIMEOUT_MS = 3_000;

class ConnectionStore {
	daemonUrl = $state<string>(DEFAULT_DAEMON_URL);
	lastHealthCheck = $state<number>(0);
	daemonState = $state<DaemonState>("offline");
	attempt = $state<number>(0);
	lastError = $state<{ ts: number; message: string } | null>(null);

	/// 上次 health 端点返回的完整状态 (供 UI 显示 session_count / mcp_online 等)
	health = $state<HealthStatus | null>(null);

	// ─── 派生 ────────────────────────────────────────────────────────────────

	/// 是否显示降级 UI (degraded | offline)
	isDegraded = $derived(this.daemonState === "degraded" || this.daemonState === "offline");

	/// 上次错误时间 (格式化)
	lastErrorDisplay = $derived.by(() => {
		if (!this.lastError) return "从未连接";
		const ago = Math.floor((Date.now() - this.lastError.ts) / 1000);
		return `${ago}s 前: ${this.lastError.message}`;
	});

	// ─── 内部 ────────────────────────────────────────────────────────────────

	#timer: ReturnType<typeof setInterval> | null = null;
	#abortController: AbortController | null = null;

	// ─── 状态机方法 ──────────────────────────────────────────────────────────

	/// 启动 10s 周期 health check.
	/// Stage 1 行为: 第一次 ping 失败即切到 'offline' (因为 Daemon 未启动),
	/// 用户可以手动 retry() 触发重试.
	async startHealthCheck(): Promise<void> {
		// 避免重复启动
		if (this.#timer) return;
		this.daemonState = "reconnecting";
		this.attempt = 0;

		// 立即 ping 一次, 然后每 10s 周期
		await this.#ping();
		this.#timer = setInterval(() => this.#ping(), HEALTH_CHECK_INTERVAL_MS);
	}

	/// 立即重试, 不等下一个 tick.
	retry(): void {
		this.attempt = 0;
		this.daemonState = "reconnecting";
		void this.#ping();
	}

	/// 显式停止 health check (卸载/暂停时使用)
	stopHealthCheck(): void {
		if (this.#timer) {
			clearInterval(this.#timer);
			this.#timer = null;
		}
		this.#abortController?.abort();
		this.#abortController = null;
		this.daemonState = "offline";
	}

	// ─── 内部 ping 实现 ──────────────────────────────────────────────────────

	async #ping(): Promise<void> {
		this.attempt += 1;
		this.#abortController?.abort();
		this.#abortController = new AbortController();
		const timer = setTimeout(() => this.#abortController?.abort(), REQUEST_TIMEOUT_MS);

		try {
			// Stage 2: 走 IPC bridge (Tauri: invoke 真实后端 / Web: 浏览器 fetch).
			const data = await fetchDaemonHealth(this.daemonUrl);
			this.#abortController?.abort(); // 拿到结果后立即取消超时

			// 兼容: 后端返回的 status 可能是 4 态之一, 统一收口.
			const next: DaemonState =
				data.status === "connected" || data.status === "reconnecting" || data.status === "degraded"
					? data.status
					: "offline";

			// bridge 在网络错误时已返回 status="offline", 这里不区分网络/HTTP,
			// 直接用返回的 status 即可. 如果是 offline 且本轮是首 ping, 走 reconnecting;
			// 累计 ≥3 次失败切 degraded (保留 Stage 1 的语义).
			if (next === "offline") {
				this.#markError("daemon 不可达");
			} else {
				this.health = { ...data, status: next };
				this.daemonState = next;
				this.lastHealthCheck = Date.now();
				this.lastError = null;
				this.attempt = 0;
			}
		} catch (e) {
			const err = e as Error;
			// AbortError 不算错误 (用户主动取消或 timer 触发)
			if (err.name === "AbortError") {
				this.#markError("请求超时 (3s)");
			} else {
				this.#markError(err.message || "网络错误");
			}
		} finally {
			clearTimeout(timer);
		}
	}

	#markError(message: string): void {
		this.lastError = { ts: Date.now(), message };
		this.health = null;
		// ≥3 次失败视为 degraded
		if (this.attempt >= 3) {
			this.daemonState = "degraded";
		} else {
			this.daemonState = "reconnecting";
		}
	}
}

export const connectionStore = new ConnectionStore();

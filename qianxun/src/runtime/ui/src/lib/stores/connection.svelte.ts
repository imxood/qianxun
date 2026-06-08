// Stage 9c — Connection Store (临时 stub, 待 responsive sibling agent 实现完整)
// 跟 qianxun-desktop/src/lib/stores/connection.svelte.ts 对齐 (4 态状态机)
//
// 当前: 简化版 — 暴露 daemonReachable / lastError + 兄弟 agent 期望的方法
// (checkReachable / startHealthLoop / markUnreachable / markReachable) 让
// Sidebar / TopBar / +layout 能编译.
//
// 完整版 (healthCheck interval + 3-6-12-30s 退避 + degraded/reconnecting 多态)
// 留 sibling 接管.

import { authStore } from './auth.svelte';

export interface HealthCheckResult {
	ok: boolean;
	status?: 'ok' | 'degraded' | 'down';
	version?: string;
	latencyMs?: number;
	error?: string;
}

class ConnectionStore {
	#daemonReachable = $state(true);
	#lastError = $state<string | null>(null);
	#initialized = $state(false);

	get daemonReachable(): boolean {
		return this.#daemonReachable;
	}

	get lastError(): string | null {
		return this.#lastError;
	}

	get initialized(): boolean {
		return this.#initialized;
	}

	async checkReachable(): Promise<HealthCheckResult> {
		const start = Date.now();
		try {
			const headers: Record<string, string> = { Accept: 'application/json' };
			if (authStore.token) {
				headers['Authorization'] = `Bearer ${authStore.token}`;
			}
			const r = await fetch('/v1/system/health', { headers });
			const latency = Date.now() - start;
			if (r.ok) {
				const body = (await r.json().catch(() => ({}))) as {
					status?: string;
					version?: string;
				};
				this.#daemonReachable = true;
				this.#lastError = null;
				return {
					ok: true,
					status: (body.status as 'ok' | 'degraded' | 'down') ?? 'ok',
					version: body.version,
					latencyMs: latency
				};
			}
			const msg = `HTTP ${r.status}`;
			this.#daemonReachable = false;
			this.#lastError = msg;
			return { ok: false, error: msg, latencyMs: latency };
		} catch (e) {
			const msg = e instanceof Error ? e.message : String(e);
			this.#daemonReachable = false;
			this.#lastError = msg;
			return { ok: false, error: msg, latencyMs: Date.now() - start };
		}
	}

	/// Stage 9c 简化: 用 setInterval 而非 sibling 期望的 startHealthLoop 闭包.
	/// 返回 stop 函数 (sibling 兼容, onDestroy 调).
	startHealthLoop(intervalMs = 10_000): () => void {
		void this.checkReachable();
		const id = setInterval(() => {
			void this.checkReachable();
		}, intervalMs);
		return () => clearInterval(id);
	}

	markUnreachable(message?: string): void {
		this.#daemonReachable = false;
		this.#lastError = message ?? null;
	}

	markReachable(): void {
		this.#daemonReachable = true;
		this.#lastError = null;
	}
}

export const connectionStore = new ConnectionStore();

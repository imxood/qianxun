// Stage 7a §6.1: token 状态 — 存 localStorage, 401 时清空 + 触发重弹.
// 跟 qianxun-desktop 完全独立 (Tauri 用 IPC 注入, Web 用 localStorage).

import { browser } from '$app/environment';

const TOKEN_KEY = 'qianxun_admin_token';

class AuthStore {
	#token = $state<string | null>(null);
	#initialized = $state(false);

	/** 当前 JWT (or null when unauthenticated) */
	get token(): string | null {
		return this.#token;
	}

	/** 是否已登录 (有 token) */
	get isAuthenticated(): boolean {
		return this.#token != null && this.#token.length > 0;
	}

	/** 是否已从 localStorage 读过一次 (用于判断要不要弹 token 框) */
	get initialized(): boolean {
		return this.#initialized;
	}

	/**
	 * 从 localStorage 初始化. 在 +layout.svelte onMount 时调一次.
	 * SSR 环境 (browser=false) 直接跳过.
	 */
	init(): void {
		if (this.#initialized) return;
		if (!browser) return;
		try {
			const v = localStorage.getItem(TOKEN_KEY);
			this.#token = v && v.length > 0 ? v : null;
		} catch {
			// localStorage 可能被禁用 (隐私模式) — 静默降级到内存
			this.#token = null;
		}
		this.#initialized = true;
	}

	/**
	 * 写入 token 到 localStorage + 内存.
	 * @throws 当 localStorage 不可用时静默吞掉 (内存里仍有 token, 但刷新会丢)
	 */
	setToken(token: string): void {
		this.#token = token;
		if (browser) {
			try {
				localStorage.setItem(TOKEN_KEY, token);
			} catch {
				/* ignore */
			}
		}
	}

	/** 清空 token. 401 时调. */
	clear(): void {
		this.#token = null;
		if (browser) {
			try {
				localStorage.removeItem(TOKEN_KEY);
			} catch {
				/* ignore */
			}
		}
	}
}

export const authStore = new AuthStore();

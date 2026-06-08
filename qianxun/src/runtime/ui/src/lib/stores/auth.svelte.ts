// Stage 10a — 密码登录 store (替代 Stage 7a 的纯 JWT token store)
//
// 设计:
// - 用户输入密码 → POST /v1/auth/login → 拿 {token, exp, ...}
// - token + exp 存 localStorage
// - 启动时检查 exp, 已过期自动清空 + 触发重弹
// - 调用方 (auth api) 在 401 时调 clear() 触发弹框
// - 不直接打 /v1/* API — 调 api/auth.ts (login / changePassword / logout)

import { browser } from '$app/environment';
import { login as apiLogin, logout as apiLogout } from '$lib/api/auth';

const TOKEN_KEY = 'qianxun_admin_token';
const EXP_KEY = 'qianxun_admin_token_exp';
const SUB_KEY = 'qianxun_admin_sub';

class AuthStore {
	#token = $state<string | null>(null);
	#expiresAt = $state<number | null>(null); // unix seconds
	#sub = $state<string | null>(null);
	#initialized = $state(false);

	/** 当前 JWT (or null when unauthenticated) */
	get token(): string | null {
		return this.#token;
	}

	/** JWT 过期 unix timestamp (秒). null = 未登录. */
	get expiresAt(): number | null {
		return this.#expiresAt;
	}

	/** 当前用户 sub (e.g. "admin") */
	get sub(): string | null {
		return this.#sub;
	}

	/** 是否已登录 (有 token 且未过期) */
	get isAuthenticated(): boolean {
		if (this.#token == null || this.#token.length === 0) return false;
		if (this.#expiresAt == null) return true; // legacy: 兼容旧 localStorage
		return Date.now() / 1000 < this.#expiresAt;
	}

	/** 是否已从 localStorage 读过一次 (用于判断要不要弹 token 框) */
	get initialized(): boolean {
		return this.#initialized;
	}

	/**
	 * 从 localStorage 初始化. 在 +layout.svelte onMount 时调一次.
	 * SSR 环境 (browser=false) 直接跳过.
	 *
	 * 顺带做: 检查 token 过期, 过期则自动清空.
	 */
	init(): void {
		if (this.#initialized) return;
		if (!browser) return;
		try {
			const t = localStorage.getItem(TOKEN_KEY);
			const expStr = localStorage.getItem(EXP_KEY);
			const sub = localStorage.getItem(SUB_KEY);
			this.#token = t && t.length > 0 ? t : null;
			this.#expiresAt = expStr ? parseInt(expStr, 10) || null : null;
			this.#sub = sub || null;
		} catch {
			this.#token = null;
			this.#expiresAt = null;
			this.#sub = null;
		}
		this.#initialized = true;

		// 启动时检查过期 — 过期立即清, 让 layout 重弹密码框
		if (this.#token && this.#expiresAt != null) {
			if (Date.now() / 1000 >= this.#expiresAt) {
				console.info('[auth] token expired at', new Date(this.#expiresAt * 1000).toISOString());
				this.clear();
			}
		}
	}

	/**
	 * Stage 10a — 密码登录. 调 POST /v1/auth/login.
	 * 成功 → 写 token + exp + sub 到 localStorage.
	 * 失败 → throw (callers handle).
	 */
	async login(password: string): Promise<void> {
		const resp = await apiLogin(password);
		this.setSession(resp.token, resp.exp, resp.sub);
	}

	/**
	 * Stage 10a — 登出. 调 POST /v1/auth/logout (server-side stateless hook)
	 * 然后清 localStorage.
	 */
	async logout(): Promise<void> {
		try {
			await apiLogout();
		} catch {
			// 即便服务端失败, 客户端也清 — 反正 token 是 stateless
		}
		this.clear();
	}

	/**
	 * 写入 token + 过期时间到 localStorage + 内存.
	 * `expiresAt` 是 unix seconds. `sub` 是 claims.sub (e.g. "admin").
	 */
	setSession(token: string, expiresAt: number, sub: string): void {
		this.#token = token;
		this.#expiresAt = expiresAt;
		this.#sub = sub;
		if (browser) {
			try {
				localStorage.setItem(TOKEN_KEY, token);
				localStorage.setItem(EXP_KEY, String(expiresAt));
				localStorage.setItem(SUB_KEY, sub);
			} catch {
				/* ignore */
			}
		}
	}

	/**
	 * 写入 token (向后兼容 Stage 7a 路径 — TokenDialog 调).
	 * 用当前 iat + 默认 24h 计算 expiresAt (假设 token 24h 有效).
	 */
	setToken(token: string): void {
		const expiresAt = Math.floor(Date.now() / 1000) + 24 * 60 * 60;
		this.setSession(token, expiresAt, 'admin');
	}

	/** 清空 token. 401 时调 / 用户主动 logout. */
	clear(): void {
		this.#token = null;
		this.#expiresAt = null;
		this.#sub = null;
		// clear() 之后, 允许 init() 重新读 localStorage (testability + 重新登录)
		this.#initialized = false;
		if (browser) {
			try {
				localStorage.removeItem(TOKEN_KEY);
				localStorage.removeItem(EXP_KEY);
				localStorage.removeItem(SUB_KEY);
			} catch {
				/* ignore */
			}
		}
	}
}

export const authStore = new AuthStore();

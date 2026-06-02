// Stage 7a §6 — fetchWithAuth(url, opts) 封装
//
// 行为:
// 1. 读 authStore.token
// 2. 有 token → 注入 Authorization: Bearer <token>
// 3. 401 响应 → 清空 token + 派发 'qianxun:auth:failed' 事件 (由 +layout 监听弹 token 框)
// 4. 非 2xx → 抛 ApiError, 包含 status + body
// 5. 2xx → 解析 body (JSON or text), 返回
//
// 在 SSR 端 (browser=false) 不发请求, 抛错.

import { authStore } from '$lib/stores/auth.svelte';

const API_BASE = ''; // 同源 (Vite dev: 5174 → proxy → 23900; prod: 23900)

export class ApiError extends Error {
	constructor(
		public status: number,
		public statusText: string,
		public body: unknown
	) {
		super(`API ${status} ${statusText}: ${typeof body === 'string' ? body : JSON.stringify(body)}`);
		this.name = 'ApiError';
	}
}

export type FetchWithAuthInit = Omit<RequestInit, 'body' | 'headers'> & {
	body?: unknown; // 自动 JSON.stringify
	headers?: Record<string, string>;
	signal?: AbortSignal;
};

export class AuthRequiredError extends Error {
	constructor() {
		super('Authentication required');
		this.name = 'AuthRequiredError';
	}
}

export async function fetchWithAuth<T = unknown>(
	url: string,
	init: FetchWithAuthInit = {}
): Promise<T> {
	if (typeof window === 'undefined') {
		throw new Error('fetchWithAuth is browser-only');
	}
	const headers: Record<string, string> = { Accept: 'application/json', ...(init.headers ?? {}) };
	const token = authStore.token;
	if (token) {
		headers['Authorization'] = `Bearer ${token}`;
	}

	// 拆解 init, 单独处理 body (init.body 是 unknown, RequestInit.body 是 BodyInit)
	const { body: rawBody, headers: _omit, ...restInit } = init;
	const fetchInit: RequestInit = {
		...restInit,
		headers
	};
	if (rawBody !== undefined) {
		if (rawBody == null) {
			fetchInit.body = null;
		} else if (typeof rawBody === 'string' || rawBody instanceof FormData) {
			fetchInit.body = rawBody as BodyInit;
		} else {
			fetchInit.body = JSON.stringify(rawBody);
			if (!headers['Content-Type']) {
				headers['Content-Type'] = 'application/json';
			}
		}
	}

	const fullUrl = url.startsWith('http') ? url : `${API_BASE}${url}`;
	const res = await fetch(fullUrl, fetchInit);

	if (res.status === 401) {
		// 401 → 清 token, 通知 +layout 弹框
		authStore.clear();
		window.dispatchEvent(new CustomEvent('qianxun:auth:failed'));
		throw new AuthRequiredError();
	}

	if (!res.ok) {
		// 尝试解析 error body
		const ct = res.headers.get('content-type') ?? '';
		const body: unknown = ct.includes('application/json')
			? await res.json().catch(() => null)
			: await res.text().catch(() => null);
		throw new ApiError(res.status, res.statusText, body);
	}

	// 2xx — 解析
	const ct = res.headers.get('content-type') ?? '';
	if (ct.includes('application/json')) {
		return (await res.json()) as T;
	}
	return (await res.text()) as unknown as T;
}

/** 便捷 GET, 路径以 / 开头 */
export function apiGet<T = unknown>(path: string, init?: FetchWithAuthInit): Promise<T> {
	return fetchWithAuth<T>(path, { ...(init ?? {}), method: 'GET' });
}

/** 便捷 POST JSON */
export function apiPost<T = unknown>(path: string, body?: unknown): Promise<T> {
	return fetchWithAuth<T>(path, { method: 'POST', body });
}

/** 便捷 PUT JSON */
export function apiPut<T = unknown>(path: string, body?: unknown): Promise<T> {
	return fetchWithAuth<T>(path, { method: 'PUT', body });
}

/** 便捷 DELETE */
export function apiDelete<T = unknown>(path: string): Promise<T> {
	return fetchWithAuth<T>(path, { method: 'DELETE' });
}

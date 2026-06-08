// qianxun-desktop/src/lib/api/client.ts
// Phase 4a-1: HTTP client 跟 daemon 通信
//
// 设计:
// - 跟 qianxun/src/daemon/ui/src/lib/api/client.ts 保持结构一致 (fetchWithAuth + ApiError + 401 处理)
// - 去 authStore 依赖: desktop 当前没认证, 直接读 env (PUBLIC_QIANXUN_DAEMON_URL) 配 base URL
// - 401 处理保留: 真 daemon 上线后可能要 token, 现在抛 AuthRequiredError
//
// 跟 docs/daemon-design.md v1.0 / _shared-contract v2 §3.1 路由对齐
// (注: 实际 daemon 还在 v0.2 路径 /v1/chat/session/*, 4a-2 统一迁移到 v1.0 /v1/sessions/*)

const DEFAULT_DAEMON_URL = 'http://127.0.0.1:23900';

// 用函数读, 不缓存: import.meta.env 在模块加载时被 snapshot, vi.stubEnv 后续
// 改 env 不会更新模块级常量. 函数每次读确保拿到当前 env.
export function getDaemonUrl(): string {
	return (import.meta.env?.PUBLIC_QIANXUN_DAEMON_URL as string | undefined) || DEFAULT_DAEMON_URL;
}

export class ApiError extends Error {
	constructor(
		public status: number,
		public statusText: string,
		public body: unknown
	) {
		super(
			`API ${status} ${statusText}: ${typeof body === 'string' ? body : JSON.stringify(body)}`
		);
		this.name = 'ApiError';
	}
}

export class AuthRequiredError extends Error {
	constructor() {
		super('Authentication required');
		this.name = 'AuthRequiredError';
	}
}

export type FetchInit = Omit<RequestInit, 'body' | 'headers'> & {
	body?: unknown;
	headers?: Record<string, string>;
	signal?: AbortSignal;
};

export async function fetchWithAuth<T = unknown>(url: string, init: FetchInit = {}): Promise<T> {
	if (typeof window === 'undefined') {
		throw new Error('fetchWithAuth is browser-only');
	}

	const headers: Record<string, string> = { Accept: 'application/json', ...(init.headers ?? {}) };

	const { body: rawBody, headers: _omit, ...restInit } = init;
	const fetchInit: RequestInit = { ...restInit, headers };
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

	const fullUrl = url.startsWith('http') ? url : `${getDaemonUrl()}${url}`;
	const res = await fetch(fullUrl, fetchInit);

	if (res.status === 401) {
		throw new AuthRequiredError();
	}

	if (!res.ok) {
		const ct = res.headers.get('content-type') ?? '';
		const body: unknown = ct.includes('application/json')
			? await res.json().catch(() => null)
			: await res.text().catch(() => null);
		throw new ApiError(res.status, res.statusText, body);
	}

	const ct = res.headers.get('content-type') ?? '';
	if (ct.includes('application/json')) {
		return (await res.json()) as T;
	}
	return (await res.text()) as unknown as T;
}

export function apiGet<T = unknown>(path: string, init?: FetchInit): Promise<T> {
	return fetchWithAuth<T>(path, { ...(init ?? {}), method: 'GET' });
}

export function apiPost<T = unknown>(path: string, body?: unknown): Promise<T> {
	return fetchWithAuth<T>(path, { method: 'POST', body });
}

export function apiPut<T = unknown>(path: string, body?: unknown): Promise<T> {
	return fetchWithAuth<T>(path, { method: 'PUT', body });
}

export function apiDelete<T = unknown>(path: string): Promise<T> {
	return fetchWithAuth<T>(path, { method: 'DELETE' });
}

export { DEFAULT_DAEMON_URL };
// Backwards compat alias (was previously a string constant; tests may import it)
export { getDaemonUrl as DAEMON_URL };

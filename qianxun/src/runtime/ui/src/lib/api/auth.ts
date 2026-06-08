// Stage 10a — Auth API client (密码登录 + 修改密码 + 登出)
//
// 跟 Stage 7a 的 `api/settings.ts::rotateAdminToken` 配套:
// - login() 公开 endpoint (不走 fetchWithAuth, 不带 Authorization)
// - changePassword() / logout() 走 fetchWithAuth (需已登录)

import { ApiError, apiPost } from './client';

/** POST /v1/auth/login 响应. */
export interface LoginResponse {
	token: string;
	/** unix seconds */
	exp: number;
	sub: string;
	expires_in: number;
}

/** POST /v1/auth/change-password 请求体. */
export interface ChangePasswordRequest {
	old_password: string;
	new_password: string;
}

/** POST /v1/auth/change-password 响应. */
export interface ChangePasswordResponse {
	status: string;
	message: string;
}

/** POST /v1/auth/logout 响应. */
export interface LogoutResponse {
	status: string;
	message: string;
}

/**
 * 密码登录. **不** 走 fetchWithAuth (这个 endpoint 本身公开, 不需要带 token).
 * 后端: bcrypt::verify, 通过后签发 24h JWT.
 *
 * @throws ApiError 401 (invalid_credentials) / 400 (invalid_request)
 * @throws Error 网络错误 / 解析错误
 */
export async function login(password: string): Promise<LoginResponse> {
	const url = '/v1/auth/login';
	const res = await fetch(url, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
		body: JSON.stringify({ password })
	});
	if (!res.ok) {
		const ct = res.headers.get('content-type') ?? '';
		const body: unknown = ct.includes('application/json')
			? await res.json().catch(() => null)
			: await res.text().catch(() => null);
		throw new ApiError(res.status, res.statusText, body);
	}
	return (await res.json()) as LoginResponse;
}

/**
 * 修改密码. 走 fetchWithAuth (需已登录).
 * 后端: 验证 old_password, 写新 hash 到 admin.cred (不 rotate token).
 */
export async function changePassword(
	oldPassword: string,
	newPassword: string
): Promise<ChangePasswordResponse> {
	return apiPost<ChangePasswordResponse>('/v1/auth/change-password', {
		old_password: oldPassword,
		new_password: newPassword
	});
}

/**
 * 登出. 走 fetchWithAuth (需已登录).
 * 后端 stateless — 仅返 200. 客户端必须清 localStorage.
 * `authStore.logout()` 已经在 store 内部调 clear(), 这里只是 fire-and-forget.
 */
export async function logout(): Promise<LogoutResponse> {
	return apiPost<LogoutResponse>('/v1/auth/logout', {});
}

// Stage 9c — Settings API client
//
// 包含 token rotate endpoint — POST /v1/system/admin/rotate-token
// daemon 端用现有 HS256 secret (env var QIANXUN_JWT_SECRET) 重新签发一个
// 24h 过期的 admin JWT 并返回. 旧 token 仍能在 daemon 内存里校验通过
// (因为 secret 没换), 简化版的 rotate; 真"secret rotation"需要换 JWT
// secret — Stage 9c 暂不实现, 留后续.

import { apiPost } from './client';
import type { TokenRotateResponse } from '$lib/types/api';

/**
 * 重新生成 admin token (HS256, exp = now + 24h, sub = "admin").
 *
 * 行为:
 * - 旧 token 仍然能用 (daemon 没换 secret), 但前端会立刻用新 token 替换
 *   localStorage, 旧 token 在浏览器侧"作废" (从 UX 角度)
 * - 后端不会强制失效旧 token (Stage 9c 简化方案)
 *
 * @throws AuthRequiredError 如果当前未登录
 * @throws ApiError 其他 HTTP 错误
 */
export async function rotateAdminToken(): Promise<TokenRotateResponse> {
	return apiPost<TokenRotateResponse>('/v1/system/admin/rotate-token');
}

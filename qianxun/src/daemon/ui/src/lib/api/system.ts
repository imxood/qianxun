// System API client
// 公开 endpoint (无 auth), 用于状态指示 / 健康检查

import { apiGet } from './client';
import type { SystemHealth, SystemStatus } from '$lib/types/api';

export async function getHealth(): Promise<SystemHealth> {
	return apiGet<SystemHealth>('/v1/system/health');
}

export async function getStatus(): Promise<SystemStatus> {
	return apiGet<SystemStatus>('/v1/system/status');
}

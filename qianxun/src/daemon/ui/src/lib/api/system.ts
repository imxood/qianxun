// System API client (Stage 7a health/status + Stage 7b metrics/logs)
// 公开 endpoint (无 auth), 用于状态指示 / 健康检查
// metrics + logs 是 auth-protected (跟随 daemon 鉴权)

import { apiGet } from './client';
import type { SystemHealth, SystemLogsResponse, SystemMetrics, SystemStatus } from '$lib/types/api';

export async function getHealth(): Promise<SystemHealth> {
	return apiGet<SystemHealth>('/v1/system/health');
}

export async function getStatus(): Promise<SystemStatus> {
	return apiGet<SystemStatus>('/v1/system/status');
}

/** Stage 7b: CPU/内存/连接数/sessions 摘要 */
export async function getMetrics(): Promise<SystemMetrics> {
	return apiGet<SystemMetrics>('/v1/system/metrics');
}

/** Stage 7b: 日志 tail (最近 N 行) */
export async function getLogs(lines = 100): Promise<SystemLogsResponse> {
	return apiGet<SystemLogsResponse>(`/v1/system/logs?lines=${encodeURIComponent(lines)}`);
}

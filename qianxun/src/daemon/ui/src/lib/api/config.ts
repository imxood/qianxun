// Config API client — Stage 7b
// 对应 daemon /v1/config (GET 已有, PUT 新增)

import { apiGet, apiPut } from './client';
import type { ConfigUpdateResponse, ResolvedConfigView } from '$lib/types/api';

export interface ConfigResponse {
	config: ResolvedConfigView;
}

export async function getConfig(): Promise<ResolvedConfigView> {
	const r = await apiGet<ConfigResponse>('/v1/config');
	return r.config;
}

export async function putConfig(
	patch: Partial<ResolvedConfigView>
): Promise<ConfigUpdateResponse> {
	return apiPut<ConfigUpdateResponse>('/v1/config', patch);
}

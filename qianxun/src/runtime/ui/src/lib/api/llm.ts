// LLM provider API client
// 跟 docs/30_子项目规划/_shared-contract.md §3.1.1 endpoint 对应.

import { apiDelete, apiGet, apiPost, apiPut } from './client';
import type {
	LlmProviderConfig,
	LlmProviderSummary,
	ProviderTestResult
} from '$lib/types/api';

export interface LlmProvidersResponse {
	providers: LlmProviderSummary[];
}

export interface LlmProviderResponse {
	provider: Omit<LlmProviderConfig, 'api_key'> & { has_key: boolean; active: boolean };
}

export interface LlmStatusResponse {
	status: 'added' | 'updated' | 'deleted' | 'active';
}

export async function listProviders(): Promise<LlmProviderSummary[]> {
	const r = await apiGet<LlmProvidersResponse>('/v1/llm/providers');
	return r.providers ?? [];
}

export async function getProvider(id: string): Promise<LlmProviderResponse['provider']> {
	const r = await apiGet<LlmProviderResponse>(`/v1/llm/providers/${encodeURIComponent(id)}`);
	return r.provider;
}

export async function createProvider(cfg: LlmProviderConfig): Promise<void> {
	await apiPost<LlmStatusResponse>('/v1/llm/providers', cfg);
}

export async function updateProvider(id: string, cfg: LlmProviderConfig): Promise<void> {
	await apiPut<LlmStatusResponse>(`/v1/llm/providers/${encodeURIComponent(id)}`, cfg);
}

export async function deleteProvider(id: string): Promise<void> {
	await apiDelete<LlmStatusResponse>(`/v1/llm/providers/${encodeURIComponent(id)}`);
}

export async function activateProvider(id: string): Promise<void> {
	await apiPost<LlmStatusResponse>(`/v1/llm/providers/${encodeURIComponent(id)}/activate`);
}

export async function testProvider(id: string): Promise<ProviderTestResult> {
	return apiPost<ProviderTestResult>(
		`/v1/llm/providers/${encodeURIComponent(id)}/test`
	);
}

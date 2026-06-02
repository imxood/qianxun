// Memory API client — Stage 7b
// 对应 daemon /v1/memory/* (sessions / search / observations/{id} DELETE)

import { apiDelete, apiGet, apiPost } from './client';
import type {
	MemoryObservation,
	MemorySearchRequest,
	MemorySearchResponse,
	MemorySessionSummary,
	MemorySessionsResponse
} from '$lib/types/api';

export interface MemoryObservationsResponse {
	observations: MemoryObservation[];
}

export async function listMemorySessions(): Promise<MemorySessionSummary[]> {
	const r = await apiGet<MemorySessionsResponse>('/v1/memory/sessions');
	return r.sessions ?? [];
}

export async function searchMemory(req: MemorySearchRequest): Promise<MemorySearchResponse> {
	return apiPost<MemorySearchResponse>('/v1/memory/search', req);
}

/** 获取指定 session 下的 observations (Web UI 辅助 endpoint, 后端可选) */
export async function listObservations(
	sessionId: string
): Promise<MemoryObservation[]> {
	const r = await apiGet<MemoryObservationsResponse>(
		`/v1/memory/sessions/${encodeURIComponent(sessionId)}/observations`
	);
	return r.observations ?? [];
}

export async function deleteObservation(id: string): Promise<void> {
	await apiDelete<{ status: string }>(`/v1/memory/observations/${encodeURIComponent(id)}`);
}

/** 删整个 memory session (Stage 7b 简化) */
export async function deleteMemorySession(id: string): Promise<void> {
	await apiDelete<{ status: string }>(`/v1/memory/sessions/${encodeURIComponent(id)}`);
}

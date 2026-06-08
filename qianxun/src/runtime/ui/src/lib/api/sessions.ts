// Chat Sessions API client — Stage 7b
// 对应 daemon /v1/chat/sessions, /v1/chat/session/{id}/{cancel,pause,delete}

import { apiDelete, apiGet, apiPost } from './client';
import type {
	ChatSessionActionResponse,
	ChatSessionDetail,
	ChatSessionsResponse,
	ChatSessionSummary,
	SessionStatus
} from '$lib/types/api';

export interface ListSessionsParams {
	status?: SessionStatus | 'all';
}

export async function listChatSessions(
	params: ListSessionsParams = {}
): Promise<{ sessions: ChatSessionSummary[]; total: number }> {
	const qs = params.status && params.status !== 'all' ? `?status=${params.status}` : '';
	const r = await apiGet<ChatSessionsResponse>(`/v1/chat/sessions${qs}`);
	return { sessions: r.sessions ?? [], total: r.total ?? (r.sessions?.length ?? 0) };
}

export async function getChatSession(id: string): Promise<ChatSessionDetail> {
	return apiGet<ChatSessionDetail>(`/v1/chat/session/${encodeURIComponent(id)}`);
}

export async function cancelChatSession(id: string): Promise<ChatSessionActionResponse> {
	return apiPost<ChatSessionActionResponse>(
		`/v1/chat/session/${encodeURIComponent(id)}/cancel`
	);
}

export async function pauseChatSession(id: string): Promise<ChatSessionActionResponse> {
	return apiPost<ChatSessionActionResponse>(
		`/v1/chat/session/${encodeURIComponent(id)}/pause`
	);
}

export async function deleteChatSession(id: string): Promise<ChatSessionActionResponse> {
	return apiDelete<ChatSessionActionResponse>(
		`/v1/chat/session/${encodeURIComponent(id)}`
	);
}

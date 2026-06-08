// Stage 9c — Chat API client
// 跟 docs/30_子项目规划/_shared-contract.md §3.2 SSE 事件 schema 一致
// 跟 docs/30_子项目规划/01b-daemon-web-console.md §4.2 endpoint 一致
//
// 覆盖:
//   - POST   /v1/chat/session                  创建 session
//   - GET    /v1/chat/sessions                 列表
//   - DELETE /v1/chat/session/{id}             删除
//   - POST   /v1/chat/session/{id}/prompt      SSE 流 (返原始 ReadableStream 给 parser)
//   - POST   /v1/chat/session/{id}/cancel      取消正在跑的 prompt
//
// 设计:
//   - 普通 CRUD (create / list / delete / cancel) 走 fetchWithAuth (自动注入 Bearer, 错误转 ApiError)
//   - streamPrompt 走 raw fetch (不解析 body) + 手动注入 Bearer, caller 自己拿 Response.body.getReader()
//     喂给 lib/sse/parser.ts

import { fetchWithAuth } from './client';
import { authStore } from '$lib/stores/auth.svelte';
import type { ChatSession, ChatSessionCreated, ChatSessionList } from '$lib/types/chat';

// ─── 普通 CRUD (走 fetchWithAuth, 错误转 ApiError) ─────────────────────────

export async function createChatSession(): Promise<ChatSessionCreated> {
	return fetchWithAuth<ChatSessionCreated>('/v1/chat/session', { method: 'POST' });
}

export async function listChatSessionsAll(): Promise<ChatSession[]> {
	const r = await fetchWithAuth<ChatSessionList>('/v1/chat/sessions');
	return r.sessions ?? [];
}

export async function deleteChatSessionById(id: string): Promise<void> {
	await fetchWithAuth<unknown>(`/v1/chat/session/${encodeURIComponent(id)}`, {
		method: 'DELETE'
	});
}

export async function cancelChatSessionById(id: string): Promise<void> {
	await fetchWithAuth<unknown>(`/v1/chat/session/${encodeURIComponent(id)}/cancel`, {
		method: 'POST'
	});
}

// ─── SSE 流 (raw fetch, caller 自己读 body) ────────────────────────────────

/**
 * POST /v1/chat/session/{id}/prompt — 发起 SSE 流式 prompt.
 * 返原始 Response, caller 自己 .body.getReader() 解析.
 *
 * AbortSignal 透传 fetch — caller 调 abortController.abort() 即可取消流.
 *
 * 错误: HTTP 4xx/5xx 时 fetch 仍返 Response (status 非 2xx), 由 caller 决定
 * 是 throw 还是 yield error event. 这里不预 throw, 避免双层错误处理.
 */
export async function fetchPromptStream(
	sessionId: string,
	text: string,
	signal?: AbortSignal
): Promise<Response> {
	const headers: Record<string, string> = {
		'Content-Type': 'application/json',
		Accept: 'text/event-stream'
	};
	const token = authStore.token;
	if (token) {
		headers['Authorization'] = `Bearer ${token}`;
	}
	return fetch(`/v1/chat/session/${encodeURIComponent(sessionId)}/prompt`, {
		method: 'POST',
		headers,
		body: JSON.stringify({ messages: [{ role: 'user', content: text }] }),
		signal
	});
}

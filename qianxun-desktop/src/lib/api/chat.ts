// qianxun-desktop/src/lib/api/chat.ts
// Phase 4a-1: Chat API client
//
// 跟 qianxun/src/daemon/ui/src/lib/api/chat.ts 结构一致
// 覆盖:
//   - POST   /v1/chat/session                  创建 session
//   - GET    /v1/chat/sessions                 列表
//   - DELETE /v1/chat/session/{id}             删除
//   - POST   /v1/chat/session/{id}/prompt      SSE 流 (返原始 ReadableStream)
//   - POST   /v1/chat/session/{id}/cancel      取消正在跑的 prompt
//   - POST   /v1/chat/session/{id}/messages    追加消息 (sub_session 共享)
//
// 注: 路径是 v0.2 (/v1/chat/session/*), 跟真 daemon router.rs 一致
//     4a-2 切到 v1.0 (/v1/sessions/*) 时本文件再改

import { fetchWithAuth, getDaemonUrl } from './client';
import type { ChatSession, ChatSessionCreated, ChatSessionList } from './types';

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

export interface FetchPromptStreamOptions {
	text: string;
	/** sub_session 模式: 改路径 + body 多一个 sub_session_id 字段 */
	subSessionId?: string;
	signal?: AbortSignal;
}

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
	opts: FetchPromptStreamOptions
): Promise<Response> {
	const headers: Record<string, string> = {
		'Content-Type': 'application/json',
		Accept: 'text/event-stream'
	};

	// 主会话 vs sub_session 路径
	const path = opts.subSessionId
		? `/v1/chat/session/${encodeURIComponent(sessionId)}/sub_session/${encodeURIComponent(opts.subSessionId)}/prompt`
		: `/v1/chat/session/${encodeURIComponent(sessionId)}/prompt`;

	const body: Record<string, unknown> = {
		messages: [{ role: 'user', content: opts.text }]
	};
	if (opts.subSessionId) {
		body.sub_session_id = opts.subSessionId;
	}

	return fetch(`${getDaemonUrl()}${path}`, {
		method: 'POST',
		headers,
		body: JSON.stringify(body),
		signal: opts.signal
	});
}

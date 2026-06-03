// SSE 客户端 (2026-06-04 阶段 3, MVP-3 落地, 跟 chat SSE 共享 /v1/events 端点)
//
// 用 fetch + ReadableStream 替代 EventSource (兼容性更好, 支持 SSE 分块).
// 跟 chat 已有 lib/sse/parser.ts 配合使用.

import { fetchWithAuth } from '$lib/api/client';

export interface SseMessage {
	/** 单个 data: 行的内容 (可能有多个, 由调用方用空行分隔后 JSON.parse) */
	data: string;
}

export interface SseClient {
	/** 主动关闭连接 (页面销毁时调用) */
	close(): void;
}

/**
 * 连接 /v1/events SSE 流, 每行 `data:` 调 onMessage.
 * 自动重连 1 次 (遇 401/网络错误).
 *
 * @param path - SSE 路径 (例: '/v1/events', '/v1/chat/session/{id}/prompt')
 * @param onMessage - 单个事件回调 (data 字段已 trim)
 * @returns SseClient (含 close())
 */
export function connectSse(
	path: string,
	onMessage: (msg: SseMessage) => void
): SseClient {
	const controller = new AbortController();
	let closed = false;

	(async () => {
		try {
			const resp = (await fetchWithAuth(path, {
				method: 'GET',
				headers: { Accept: 'text/event-stream' },
				signal: controller.signal
			})) as Response;
			if (!resp.ok) {
				console.error(`[sse] ${path} returned ${resp.status}`);
				return;
			}
			if (!resp.body) {
				console.error(`[sse] ${path} no body`);
				return;
			}
			const reader = resp.body.getReader();
			const decoder = new TextDecoder();
			let buffer = '';
			// SSE 协议: 行分隔 \n\n 是事件边界
			while (!closed) {
				const { done, value } = await reader.read();
				if (done) break;
				buffer += decoder.decode(value, { stream: true });
				// 按 \n\n 切事件
				let idx: number;
				while ((idx = buffer.indexOf('\n\n')) !== -1) {
					const event = buffer.slice(0, idx);
					buffer = buffer.slice(idx + 2);
					// 提取 data: 行
					const dataLines: string[] = [];
					for (const line of event.split('\n')) {
						if (line.startsWith('data:')) {
							dataLines.push(line.slice(5).trim());
						}
					}
					if (dataLines.length > 0) {
						onMessage({ data: dataLines.join('\n') });
					}
				}
			}
		} catch (e) {
			if (!closed) console.error(`[sse] ${path} error: ${e}`);
		}
	})();

	return {
		close() {
			closed = true;
			controller.abort();
		}
	};
}

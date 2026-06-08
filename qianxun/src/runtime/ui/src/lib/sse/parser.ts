// ───────────────────────────────────────────────────────────────────────────
// 千寻 Web Console — SSE 流解析器 (Stage 9c)
// 跟 docs/30_子项目规划/_shared-contract.md §3.2 事件 schema 一致
// 跟 qianxun/src/daemon/sse.rs SseEvent 严格对齐 (12 种事件)
//
// 设计: 复制 qianxun-desktop/src/lib/sse/parser.ts 的解析逻辑 (chunk buffer
// + 多 data: 行拼接 + 空行 dispatch), 改 Svelte 5 / Web Console 风格.
//
// 12 个事件类型通过 JSON 的 `type` 字段分发 (内部 tag-style), daemon 在数据帧
// 直接发 `{"type": "text_delta", ...}` 的 JSON, 客户端 JSON.parse 后通过
// SseEvent 联合类型分支.
//
// RFC: text/event-stream
//   - 行分隔 \n, \r\n 都接受 (尾部 \r 去掉)
//   - data: <text> 多行拼接 (dataLines: string[] 累加)
//   - 空行 = dispatch 一个完整事件
//   - 以 ":" 开头的行 = 注释, 忽略
//   - event: / id: / retry: 字段当前不用 (daemon 用 JSON type 标识)
//   - 解析失败不中断流, 仅 console.warn
// ───────────────────────────────────────────────────────────────────────────

import type { SseEvent } from '$lib/types/chat';

/**
 * 解析器状态. 调用方维护 (避免函数式风格产生大量中间对象).
 */
export interface SseParserState {
	/// 未结束的行 (可能是不完整的最后一行)
	buffer: string;
	/// 当前累积的 data: 行 (一个事件可能跨多行)
	dataLines: string[];
}

/**
 * 创建一个新的解析器状态.
 */
export function newSseParserState(): SseParserState {
	return { buffer: '', dataLines: [] };
}

/**
 * 解析一条警告 (用于测试, 生产环境仅 console.warn).
 */
export interface SseParseWarning {
	kind: 'json' | 'unknown';
	raw: string;
	error: string;
}

/**
 * 把一段文本块喂给解析器, 返回完整事件 + 警告.
 *
 * 语义:
 * - 输入 chunk 是从 ReadableStream 读出的一个 Uint8Array 解码出的字符串
 * - 内部维护 buffer (跨 chunk 拼接) + dataLines (一个事件的多 data: 行)
 * - 遇到空行 → 把 dataLines join → JSON.parse → 推 events
 * - 解析失败 → 推 warnings (生产环境同步 console.warn, 测试可断言)
 */
export function processSseChunk(
	chunk: string,
	state: SseParserState
): { events: SseEvent[]; warnings: SseParseWarning[] } {
	const events: SseEvent[] = [];
	const warnings: SseParseWarning[] = [];

	// 把 chunk 接到 buffer, 按 \n 切
	state.buffer += chunk;
	const lines = state.buffer.split('\n');
	state.buffer = lines.pop() ?? '';

	for (const rawLine of lines) {
		// SSE wire 用 \r\n, 去掉尾部 \r
		const line = rawLine.endsWith('\r') ? rawLine.slice(0, -1) : rawLine;

		if (line.startsWith(':')) {
			// 注释行, 忽略
			continue;
		}
		if (line.startsWith('data:')) {
			const data = line.slice(5).trimStart();
			state.dataLines.push(data);
		} else if (line === '') {
			// 空行 = dispatch
			if (state.dataLines.length > 0) {
				const dataStr = state.dataLines.join('\n');
				state.dataLines = [];
				try {
					const event = JSON.parse(dataStr) as SseEvent;
					events.push(event);
				} catch (e) {
					const msg = (e as Error).message;
					console.warn('[SSE] parse error:', e, dataStr);
					warnings.push({ kind: 'json', raw: dataStr, error: msg });
				}
			}
		}
		// 其他 SSE 字段 (event: / id: / retry:) 暂忽略 —
		// Daemon 按 _shared-contract.md §3.2 约定, 类型在 JSON 的 type 字段.
	}

	return { events, warnings };
}

/**
 * 把一个 ReadableStream<Uint8Array> 解析成 SseEvent 序列 (AsyncGenerator).
 *
 * 用法:
 * ```ts
 * const response = await fetchPromptStream(sessionId, text);
 * for await (const ev of parseSseStream(response.body)) {
 *   // 处理 ev (12 种类型之一)
 * }
 * ```
 */
export async function* parseSseStream(
	stream: ReadableStream<Uint8Array> | null
): AsyncGenerator<SseEvent, void, undefined> {
	if (!stream) {
		throw new Error('Empty response body');
	}

	const reader = stream.getReader();
	const decoder = new TextDecoder('utf-8');
	const state = newSseParserState();

	try {
		while (true) {
			const { value, done } = await reader.read();
			if (done) break;

			const chunk = decoder.decode(value, { stream: true });
			const { events } = processSseChunk(chunk, state);
			for (const ev of events) {
				yield ev;
			}
		}
	} finally {
		try {
			reader.releaseLock();
		} catch {
			// ignore — reader may already be released by abort
		}
	}
}

/**
 * 测试 helper: 从一个 string 构造 ReadableStream<Uint8Array>.
 * 接受 (text: string) → ReadableStream, 把整段 text 一次性 enqueue 然后 close.
 */
export function stringToReadableStream(text: string): ReadableStream<Uint8Array> {
	const encoder = new TextEncoder();
	return new ReadableStream<Uint8Array>({
		start(controller) {
			controller.enqueue(encoder.encode(text));
			controller.close();
		}
	});
}

/**
 * 测试 helper: 从一个 string[] 构造 ReadableStream, 每次 enqueue 一段.
 * 用于验证 chunk 边界 (state.buffer 跨 chunk 拼接).
 */
export function chunksToReadableStream(chunks: string[]): ReadableStream<Uint8Array> {
	const encoder = new TextEncoder();
	return new ReadableStream<Uint8Array>({
		start(controller) {
			for (const c of chunks) {
				controller.enqueue(encoder.encode(c));
			}
			controller.close();
		}
	});
}

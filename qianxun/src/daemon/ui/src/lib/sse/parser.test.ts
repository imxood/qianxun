// ──────────────────────────────────────────────────────────────────────────
// Stage 9c — SSE parser 单元测试
// 跟 docs/30_子项目规划/_shared-contract.md §3.2 事件 schema 一致
// 跟 qianxun/src/daemon/sse.rs SseEvent 严格对齐 (12 种事件)
//
// 覆盖:
//   - 单事件 (text_delta)
//   - 多事件 (多 data: 行 + 空行 dispatch)
//   - 工具调用 (tool_use_complete)
//   - 流结束 (message_stop)
//   - 错误事件 (error)
//   - 跨 chunk 边界 (state.buffer 拼接)
//   - \r\n 换行处理
//   - 注释行 (:) 忽略
// ──────────────────────────────────────────────────────────────────────────

import { describe, it, expect } from 'vitest';
import {
	processSseChunk,
	newSseParserState,
	parseSseStream,
	stringToReadableStream,
	chunksToReadableStream
} from './parser';

describe('processSseChunk (SSE 增量解析)', () => {
	it('text_delta 单事件解析', () => {
		const state = newSseParserState();
		const r = processSseChunk('data: {"type":"text_delta","index":0,"text":"hello"}\n\n', state);
		expect(r.events).toHaveLength(1);
		expect(r.events[0]).toEqual({ type: 'text_delta', index: 0, text: 'hello' });
		expect(r.warnings).toHaveLength(0);
	});

	it('tool_use_complete 单事件解析', () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"tool_use_complete","index":1,"id":"t1","name":"read_file","arguments":{"path":"/tmp"}}\n\n',
			state
		);
		expect(r.events).toHaveLength(1);
		const ev = r.events[0] as Extract<typeof r.events[0], { type: 'tool_use_complete' }>;
		expect(ev.type).toBe('tool_use_complete');
		expect(ev.id).toBe('t1');
		expect(ev.name).toBe('read_file');
		expect(ev.arguments).toEqual({ path: '/tmp' });
	});

	it('多事件 dispatch (多 data: 行 + 空行)', () => {
		const state = newSseParserState();
		const chunk =
			'data: {"type":"message_start","session_id":"s1","model":"m","max_tokens":1024}\n\n' +
			'data: {"type":"content_block_start","index":0,"block_type":"text"}\n\n' +
			'data: {"type":"text_delta","index":0,"text":"hi"}\n\n' +
			'data: {"type":"message_stop"}\n\n';
		const r = processSseChunk(chunk, state);
		expect(r.events).toHaveLength(4);
		expect(r.events.map((e) => e.type)).toEqual([
			'message_start',
			'content_block_start',
			'text_delta',
			'message_stop'
		]);
	});

	it('message_stop (无字段) 解析', () => {
		const state = newSseParserState();
		const r = processSseChunk('data: {"type":"message_stop"}\n\n', state);
		expect(r.events).toHaveLength(1);
		expect(r.events[0]?.type).toBe('message_stop');
	});

	it('error 事件解析 (含 code + message)', () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"error","code":"rate_limit","message":"too many"}\n\n',
			state
		);
		expect(r.events).toHaveLength(1);
		const ev = r.events[0] as Extract<typeof r.events[0], { type: 'error' }>;
		expect(ev.type).toBe('error');
		expect(ev.code).toBe('rate_limit');
		expect(ev.message).toBe('too many');
	});

	it('解析失败 → warnings + 不中断流', () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: not-json-at-all\n\n' +
				'data: {"type":"text_delta","index":0,"text":"ok"}\n\n',
			state
		);
		expect(r.warnings).toHaveLength(1);
		expect(r.warnings[0]?.kind).toBe('json');
		expect(r.events).toHaveLength(1);
		expect(r.events[0]?.type).toBe('text_delta');
	});

	it('注释行 (: 开头) 忽略', () => {
		const state = newSseParserState();
		const r = processSseChunk(
			':this is a comment\ndata: {"type":"message_stop"}\n\n',
			state
		);
		expect(r.events).toHaveLength(1);
		expect(r.events[0]?.type).toBe('message_stop');
	});

	it('\\r\\n 换行 (SSE 标准 wire format)', () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"text_delta","index":0,"text":"x"}\r\n\r\n',
			state
		);
		expect(r.events).toHaveLength(1);
	});

	it('跨 chunk 边界 (state.buffer 拼接)', () => {
		const state = newSseParserState();
		// 第一 chunk: data: + 半个 JSON
		const r1 = processSseChunk('data: {"type":"text_delta",', state);
		expect(r1.events).toHaveLength(0);
		// 第二 chunk: 补全 JSON + 空行
		const r2 = processSseChunk('"index":0,"text":"abc"}\n\n', state);
		expect(r2.events).toHaveLength(1);
		expect((r2.events[0] as { text: string }).text).toBe('abc');
	});
});

describe('parseSseStream (ReadableStream 端到端)', () => {
	it('简单流: 1 个 message_stop', async () => {
		const stream = stringToReadableStream('data: {"type":"message_stop"}\n\n');
		const events: string[] = [];
		for await (const ev of parseSseStream(stream)) {
			events.push(ev.type);
		}
		expect(events).toEqual(['message_stop']);
	});

	it('text_delta 流 → 累积为完整文本', async () => {
		const stream = stringToReadableStream(
			'data: {"type":"text_delta","index":0,"text":"Hello"}\n\n' +
				'data: {"type":"text_delta","index":0,"text":", "}\n\n' +
				'data: {"type":"text_delta","index":0,"text":"world!"}\n\n' +
				'data: {"type":"message_stop"}\n\n'
		);
		let text = '';
		for await (const ev of parseSseStream(stream)) {
			if (ev.type === 'text_delta') text += ev.text;
		}
		expect(text).toBe('Hello, world!');
	});

	it('null stream → throw', async () => {
		const gen = parseSseStream(null);
		await expect(gen.next()).rejects.toThrow('Empty response body');
	});

	it('chunksToReadableStream: 跨 chunk 边界正确拼接', async () => {
		const stream = chunksToReadableStream([
			'data: {"type":"text_delta","index":0,',
			'"text":"chunked"}\n\ndata: {"type":"message_stop"}\n\n'
		]);
		const events: string[] = [];
		for await (const ev of parseSseStream(stream)) {
			events.push(ev.type);
		}
		expect(events).toEqual(['text_delta', 'message_stop']);
	});
});

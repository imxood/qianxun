// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — SSE 解析器单元测试
// 与 docs/30_子项目规划/_shared-contract.md §3.2 事件 schema 一致
// 与 qianxun-desktop/src/lib/sse/parser.ts SseParserState / processSseChunk 一致
//
// 覆盖 docs/30_子项目规划/03-tauri-desktop.md §8.2 列出的 12 个 SSE 事件
// 中 8 个高频路径 (message_start / content_block_start / text_delta /
// tool_use_delta+tool_use_complete / tool_result / message_delta+message_stop /
// usage / error). 剩余 4 个 (thinking_delta / content_block_stop) 由后续
// 端到端测试补 (依赖 ReadableStream 的多事件场景).
//
// 不测 ReadableStream 层的网络/客户端行为 (那是 client.ts 职责)
// 不测 network 调用 (mock 字符串)
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect } from "vitest";
import { newSseParserState, processSseChunk } from "./parser";
import type { SseEvent } from "$lib/types/ipc";

// 类型守卫帮助函数 (避免在测试里写大段 type narrowing)
function asEvent<T extends SseEvent["type"]>(
	ev: SseEvent | undefined,
	type: T
): Extract<SseEvent, { type: T }> | null {
	if (!ev) return null;
	if (ev.type !== type) return null;
	return ev as Extract<SseEvent, { type: T }>;
}

describe("processSseChunk — 12 个事件中的 8 个高频路径", () => {
	// 1. message_start — 标记一次 assistant 响应的开始
	it("test_parse_message_start_event: 解析 session_id / model / max_tokens", () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"message_start","session_id":"s_1","model":"x","max_tokens":16384}\n\n',
			state
		);
		expect(r.warnings).toHaveLength(0);
		expect(r.events).toHaveLength(1);
		const ev = asEvent(r.events[0], "message_start");
		expect(ev).not.toBeNull();
		expect(ev!.session_id).toBe("s_1");
		expect(ev!.model).toBe("x");
		expect(ev!.max_tokens).toBe(16384);
	});

	// 2. content_block_start — 文本块开始 (index=0, block_type="text")
	it("test_parse_content_block_start_text: 解析 index=0 / block_type='text'", () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"content_block_start","index":0,"block_type":"text"}\n\n',
			state
		);
		expect(r.warnings).toHaveLength(0);
		expect(r.events).toHaveLength(1);
		const ev = asEvent(r.events[0], "content_block_start");
		expect(ev).not.toBeNull();
		expect(ev!.index).toBe(0);
		expect(ev!.block_type).toBe("text");
	});

	// 3. text_delta — 文本增量 (流式输出的核心)
	it("test_parse_text_delta: 解析 text 字段", () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"text_delta","index":0,"text":"hello"}\n\n',
			state
		);
		expect(r.warnings).toHaveLength(0);
		expect(r.events).toHaveLength(1);
		const ev = asEvent(r.events[0], "text_delta");
		expect(ev).not.toBeNull();
		expect(ev!.index).toBe(0);
		expect(ev!.text).toBe("hello");
	});

	// 4. tool_use_delta (流式 args) + tool_use_complete (完整 args) — 工具调用
	it("test_parse_tool_use_delta_and_complete: 2 个事件都正确解析", () => {
		const state = newSseParserState();
		// tool_use_delta: 增量推送 JSON 字符串片段 (用 JSON.stringify 自动转义)
		const deltaPayload = JSON.stringify({
			type: "tool_use_delta",
			index: 2,
			id: "toolu_1",
			name: "read_file",
			arguments_json: JSON.stringify({ path: "/" })
		});
		const r1 = processSseChunk(`data: ${deltaPayload}\n\n`, state);
		// tool_use_complete: 一次性提供完整 arguments 对象
		const completePayload = JSON.stringify({
			type: "tool_use_complete",
			index: 2,
			id: "toolu_1",
			name: "read_file",
			arguments: { path: "/tmp" }
		});
		const r2 = processSseChunk(`data: ${completePayload}\n\n`, state);
		expect(r1.warnings).toHaveLength(0);
		expect(r1.events).toHaveLength(1);
		expect(r2.warnings).toHaveLength(0);
		expect(r2.events).toHaveLength(1);

		const delta = asEvent(r1.events[0], "tool_use_delta");
		expect(delta).not.toBeNull();
		expect(delta!.index).toBe(2);
		expect(delta!.id).toBe("toolu_1");
		expect(delta!.name).toBe("read_file");
		expect(delta!.arguments_json).toBe('{"path":"/"}');

		const complete = asEvent(r2.events[0], "tool_use_complete");
		expect(complete).not.toBeNull();
		expect(complete!.index).toBe(2);
		expect(complete!.id).toBe("toolu_1");
		expect(complete!.name).toBe("read_file");
		expect(complete!.arguments).toEqual({ path: "/tmp" });
	});

	// 5. tool_result — 工具执行结果 (tool_use_id 关联, elapsed_ms 性能)
	it("test_parse_tool_result: 解析 tool_use_id / content / is_error / elapsed_ms", () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"tool_result","tool_use_id":"t_1","content":"file content here","is_error":false,"elapsed_ms":234}\n\n',
			state
		);
		expect(r.warnings).toHaveLength(0);
		expect(r.events).toHaveLength(1);
		const ev = asEvent(r.events[0], "tool_result");
		expect(ev).not.toBeNull();
		expect(ev!.tool_use_id).toBe("t_1");
		expect(ev!.content).toBe("file content here");
		expect(ev!.is_error).toBe(false);
		expect(ev!.elapsed_ms).toBe(234);
	});

	// 6. message_delta (stop_reason) + message_stop — 流结束的两步
	it("test_parse_message_delta_and_stop: 顺序正确 + stop_reason 解析", () => {
		const state = newSseParserState();
		// 先发 message_delta (含 stop_reason), 再发 message_stop
		const r1 = processSseChunk(
			'data: {"type":"message_delta","stop_reason":"end_turn"}\n\n',
			state
		);
		const r2 = processSseChunk('data: {"type":"message_stop"}\n\n', state);
		expect(r1.warnings).toHaveLength(0);
		expect(r2.warnings).toHaveLength(0);

		// message_delta
		expect(r1.events).toHaveLength(1);
		const delta = asEvent(r1.events[0], "message_delta");
		expect(delta).not.toBeNull();
		expect(delta!.stop_reason).toBe("end_turn");

		// message_stop (无字段)
		expect(r2.events).toHaveLength(1);
		const stop = asEvent(r2.events[0], "message_stop");
		expect(stop).not.toBeNull();
		expect(stop!.type).toBe("message_stop");
	});

	// 7. usage — token 计数 (含 cache 字段)
	it("test_parse_usage_event: 解析 input/output/cache_* 字段", () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"usage","input_tokens":1234,"output_tokens":567,"cache_creation_input_tokens":100,"cache_read_input_tokens":50}\n\n',
			state
		);
		expect(r.warnings).toHaveLength(0);
		expect(r.events).toHaveLength(1);
		const ev = asEvent(r.events[0], "usage");
		expect(ev).not.toBeNull();
		expect(ev!.input_tokens).toBe(1234);
		expect(ev!.output_tokens).toBe(567);
		expect(ev!.cache_creation_input_tokens).toBe(100);
		expect(ev!.cache_read_input_tokens).toBe(50);
	});

	// 8. error — daemon 推送的错误事件 (4 种 code 之一)
	it("test_parse_error_event: 解析 code='rate_limit' + message", () => {
		const state = newSseParserState();
		const r = processSseChunk(
			'data: {"type":"error","code":"rate_limit","message":"too many requests, retry in 30s"}\n\n',
			state
		);
		expect(r.warnings).toHaveLength(0);
		expect(r.events).toHaveLength(1);
		const ev = asEvent(r.events[0], "error");
		expect(ev).not.toBeNull();
		expect(ev!.code).toBe("rate_limit");
		expect(ev!.message).toBe("too many requests, retry in 30s");
	});
});

// ───────────────────────────────────────────────────────────────────────────
// chat-stream 状态机测试
//
// 镜像 qianxun-runtime/src/sse.rs 测, 验证前端 12 种 event 路由正确.
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, vi } from "vitest";
import { newStreamState, applyEvent } from "./chat-stream";
import type { SseEventFromBackend } from "$lib/ipc/runtime";

function makeState() {
	const onUpdate = vi.fn();
	return { state: newStreamState("msg_1", onUpdate), onUpdate };
}

describe("chat-stream state machine (Stage 4a sub-task #4)", () => {
	it("text_delta_appends_to_content_when_block_is_text", () => {
		const { state, onUpdate } = makeState();
		state.currentBlock = "text";
		applyEvent(state, { type: "text_delta", index: 0, text: "hello " } as SseEventFromBackend);
		applyEvent(state, { type: "text_delta", index: 0, text: "world" } as SseEventFromBackend);
		expect(state.content).toBe("hello world");
		expect(onUpdate).toHaveBeenCalledTimes(2);
	});

	it("text_delta_ignored_when_block_is_none_or_thinking", () => {
		const { state, onUpdate } = makeState();
		// currentBlock === 'none' (default)
		applyEvent(state, { type: "text_delta", index: 0, text: "x" } as SseEventFromBackend);
		expect(state.content).toBe("");

		state.currentBlock = "thinking";
		applyEvent(state, { type: "text_delta", index: 0, text: "y" } as SseEventFromBackend);
		expect(state.content).toBe("");
		expect(onUpdate).not.toHaveBeenCalled();
	});

	it("content_block_start_switches_block_kind", () => {
		const { state } = makeState();
		applyEvent(state, {
			type: "content_block_start",
			index: 0,
			block_type: "thinking",
		} as SseEventFromBackend);
		expect(state.currentBlock).toBe("thinking");
		applyEvent(state, {
			type: "content_block_start",
			index: 1,
			block_type: "tool_use",
		} as SseEventFromBackend);
		expect(state.currentBlock).toBe("tool_use");
		applyEvent(state, {
			type: "content_block_start",
			index: 2,
			block_type: "text",
		} as SseEventFromBackend);
		expect(state.currentBlock).toBe("text");
	});

	it("content_block_stop_resets_to_none", () => {
		const { state } = makeState();
		state.currentBlock = "text";
		applyEvent(state, { type: "content_block_stop", index: 0 } as SseEventFromBackend);
		expect(state.currentBlock).toBe("none");
	});

	it("thinking_delta_appends_to_thinking", () => {
		const { state, onUpdate } = makeState();
		state.currentBlock = "thinking";
		applyEvent(state, { type: "thinking_delta", index: 0, text: "let me " } as SseEventFromBackend);
		applyEvent(state, { type: "thinking_delta", index: 0, text: "think" } as SseEventFromBackend);
		expect(state.thinking).toBe("let me think");
		expect(onUpdate).toHaveBeenCalledTimes(2);
	});

	it("tool_use_complete_pushes_tool_call_with_arguments", () => {
		const { state, onUpdate } = makeState();
		applyEvent(state, {
			type: "tool_use_complete",
			index: 0,
			id: "tc_1",
			name: "read_file",
			arguments: { path: "/tmp/x" },
		} as SseEventFromBackend);
		expect(state.toolCalls).toHaveLength(1);
		expect(state.toolCalls[0]?.id).toBe("tc_1");
		expect(state.toolCalls[0]?.name).toBe("read_file");
		expect(state.toolCalls[0]?.arguments).toEqual({ path: "/tmp/x" });
		expect(state.toolCalls[0]?.result).toBeUndefined();
		expect(onUpdate).toHaveBeenCalledTimes(1);
	});

	it("tool_result_attaches_to_matching_tool_call", () => {
		const { state, onUpdate } = makeState();
		applyEvent(state, {
			type: "tool_use_complete",
			index: 0,
			id: "tc_1",
			name: "read_file",
			arguments: { path: "/tmp/x" },
		} as SseEventFromBackend);
		onUpdate.mockClear();

		applyEvent(state, {
			type: "tool_result",
			tool_use_id: "tc_1",
			content: "file content here",
			is_error: false,
			elapsed_ms: 42,
		} as SseEventFromBackend);
		expect(state.toolCalls[0]?.result?.content).toBe("file content here");
		expect(state.toolCalls[0]?.result?.isError).toBe(false);
		expect(state.toolCalls[0]?.result?.elapsedMs).toBe(42);
		expect(onUpdate).toHaveBeenCalledTimes(1);
	});

	it("tool_result_for_unknown_id_is_ignored", () => {
		const { state, onUpdate } = makeState();
		applyEvent(state, {
			type: "tool_result",
			tool_use_id: "tc_unknown",
			content: "x",
			is_error: false,
			elapsed_ms: 1,
		} as SseEventFromBackend);
		expect(state.toolCalls).toHaveLength(0);
		expect(onUpdate).not.toHaveBeenCalled();
	});

	it("message_stop_marks_finished_and_triggers_update", () => {
		const { state, onUpdate } = makeState();
		applyEvent(state, { type: "message_stop" } as SseEventFromBackend);
		expect(state.finished).toBe(true);
		expect(onUpdate).toHaveBeenCalledTimes(1);
	});

	it("error_appends_message_and_marks_finished", () => {
		const { state, onUpdate } = makeState();
		state.content = "partial response";
		applyEvent(state, {
			type: "error",
			code: "rate_limit",
			message: "too many requests",
		} as SseEventFromBackend);
		expect(state.finished).toBe(true);
		expect(state.error).toEqual({ code: "rate_limit", message: "too many requests" });
		expect(state.content).toContain("partial response");
		expect(state.content).toContain("[错误: rate_limit]");
		expect(state.content).toContain("too many requests");
		expect(onUpdate).toHaveBeenCalledTimes(1);
	});

	it("full_text_flow: block_start → text_delta → block_stop → message_stop", () => {
		const { state, onUpdate } = makeState();
		applyEvent(state, {
			type: "content_block_start",
			index: 0,
			block_type: "text",
		} as SseEventFromBackend);
		applyEvent(state, { type: "text_delta", index: 0, text: "hi" } as SseEventFromBackend);
		applyEvent(state, { type: "text_delta", index: 0, text: " there" } as SseEventFromBackend);
		applyEvent(state, { type: "content_block_stop", index: 0 } as SseEventFromBackend);
		applyEvent(state, { type: "message_stop" } as SseEventFromBackend);

		expect(state.content).toBe("hi there");
		expect(state.finished).toBe(true);
		expect(state.currentBlock).toBe("none");
		// onUpdate 调了 2 次 (text_delta × 2) + 1 次 message_stop = 3
		expect(onUpdate).toHaveBeenCalledTimes(3);
	});

	it("usage_and_message_delta_are_ignored_silently", () => {
		const { state, onUpdate } = makeState();
		applyEvent(state, {
			type: "usage",
			input_tokens: 10,
			output_tokens: 20,
			cache_creation_input_tokens: 0,
			cache_read_input_tokens: 0,
		} as SseEventFromBackend);
		applyEvent(state, {
			type: "message_delta",
			stop_reason: "end_turn",
		} as SseEventFromBackend);
		expect(state.content).toBe("");
		expect(state.finished).toBe(false);
		expect(onUpdate).not.toHaveBeenCalled();
	});

	it("tool_use_delta_is_ignored_silently: 批式 tool 不走增量", () => {
		const { state, onUpdate } = makeState();
		applyEvent(state, {
			type: "tool_use_delta",
			index: 0,
			id: "tc_1",
			name: "x",
			arguments_json: "{}",
		} as SseEventFromBackend);
		expect(state.toolCalls).toHaveLength(0);
		expect(onUpdate).not.toHaveBeenCalled();
	});
});

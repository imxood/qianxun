// ───────────────────────────────────────────────────────────────────────────
// ChatStore — Stage 4a (sub-task #4) 切真后端测试
//
// 测试覆盖 (mock $lib/ipc/runtime):
//   1. init() 注册全局 session_event listener (不重复)
//   2. send() 调 sendMessage, 创建 user 消息 + assistant 占位消息
//   3. send() 触发 plan 关键词时, 调 planStore.create + 不走流式
//   4. 流式事件通过 listener 路由到 stream state, 同步到 message.content
//   5. message_stop 收尾, streaming = false
//   6. send() 失败时本地标记错误, 弹 toast
//   7. sendToSubSession: sub 不存在弹 warn toast
//   8. sendToSubSession: sub 存在时调 sendMessageToSubSession(parent_sid, ...)
//      (4a-2 P0-2 收尾: 走真后端 invoke, 不再 noop + TODO toast)
//   9. sendToSubSession 失败时本地标记错误
// ───────────────────────────────────────────────────────────────────────────

import { describe, it, expect, beforeEach, vi } from "vitest";

const sendMessageMock = vi.fn();
const sendMessageToSubSessionMock = vi.fn();
const onSessionEventMock = vi.fn();
const createPlanMock = vi.fn();
const cancelSessionMock = vi.fn();
const loadSessionMock = vi.fn();
const listSessionsMock = vi.fn();

vi.mock("$lib/ipc/runtime", () => ({
	sendMessage: (...args: unknown[]) => sendMessageMock(...args),
	sendMessageToSubSession: (...args: unknown[]) => sendMessageToSubSessionMock(...args),
	onSessionEvent: (...args: unknown[]) => onSessionEventMock(...args),
	createPlan: (...args: unknown[]) => createPlanMock(...args),
	cancelSession: (...args: unknown[]) => cancelSessionMock(...args),
	loadSession: (...args: unknown[]) => loadSessionMock(...args),
	listSessions: (...args: unknown[]) => listSessionsMock(...args),
}));

import { chatStore } from "$lib/stores/chat.svelte";
import { sessionStore } from "$lib/stores/session.svelte";
import { planStore } from "$lib/stores/plan.svelte";
import { subSessionStore } from "$lib/stores/sub_session.svelte";
import { uiStore } from "$lib/stores/ui.svelte";
import type { SessionEventPayload } from "$lib/ipc/runtime";

// 捕获 onSessionEvent 注册的 handler, 让我们手动推事件
let capturedHandler: ((p: SessionEventPayload) => void) | null = null;
const unlistenMock = vi.fn();

// 重置 store 状态 (调 __resetForTesting 测试专用方法)
async function resetStores() {
	await chatStore.__resetForTesting();
	sessionStore.__resetForTesting();
	planStore.__resetForTesting();
}

beforeEach(async () => {
	sendMessageMock.mockReset();
	sendMessageToSubSessionMock.mockReset();
	onSessionEventMock.mockReset();
	createPlanMock.mockReset();
	cancelSessionMock.mockReset();
	loadSessionMock.mockReset();
	listSessionsMock.mockReset();
	unlistenMock.mockReset();
	capturedHandler = null;

	// 默认 onSessionEvent 立即调 handler 一次, 把 handler 存到 capturedHandler
	onSessionEventMock.mockImplementation(async (handler: (p: SessionEventPayload) => void) => {
		capturedHandler = handler;
		return unlistenMock;
	});

	await resetStores();
	uiStore.setActiveView({ kind: "empty" });
	// 清 toast 避免测试间状态泄漏 (uiStore 无 __resetForTesting, 直接清数组)
	uiStore.toasts.length = 0;

	// 预置一个 session 给 chatStore 用
	listSessionsMock.mockResolvedValue({
		sessions: [
			{
				id: "sess_test_001",
				model: "deepseek-v4-flash",
				status: "active",
				created_at: "2026-06-08T00:00:00Z",
				last_active_at: "2026-06-08T00:00:00Z",
				message_count: 0,
			},
		],
		total: 1,
		filter: "all",
		active_in_memory: 1,
		paused_in_memory: 0,
	});
});

// helper: 拿到 session_test_001 当前的 assistant streaming 消息
function lastAssistant(sessionId: string) {
	const msgs = sessionStore.getMessages(sessionId);
	return msgs.findLast((m) => m.role === "assistant" && m.streaming);
}

describe("ChatStore (Stage 4a sub-task #4 切 invoke)", () => {
	it("init_registers_global_session_event_listener_once: 重复 init 不重复注册", async () => {
		await chatStore.init();
		await chatStore.init();
		await chatStore.init();
		expect(onSessionEventMock).toHaveBeenCalledTimes(1);
		expect(capturedHandler).toBeTypeOf("function");
	});

	it("send_appends_user_and_assistant_placeholder_and_calls_invoke", async () => {
		await sessionStore.init();
		sendMessageMock.mockResolvedValue({ session_id: "sess_test_001", status: "streaming" });

		await chatStore.send("sess_test_001", "hi");

		const msgs = sessionStore.getMessages("sess_test_001");
		expect(msgs).toHaveLength(2);
		expect(msgs[0]?.role).toBe("user");
		expect(msgs[0]?.content).toBe("hi");
		expect(msgs[1]?.role).toBe("assistant");
		expect(msgs[1]?.content).toBe("");
		expect(msgs[1]?.streaming).toBe(true);
		expect(sendMessageMock).toHaveBeenCalledWith("sess_test_001", {
			messages: [{ role: "user", content: "hi" }],
		});
	});

	it("send_routes_session_event_to_message_content: 流式事件 → message.content", async () => {
		await chatStore.init();
		await sessionStore.init();
		sendMessageMock.mockResolvedValue({ session_id: "sess_test_001", status: "streaming" });

		await chatStore.send("sess_test_001", "hi");
		const assistantBefore = lastAssistant("sess_test_001");
		expect(assistantBefore).toBeDefined();

		// 推流: message_start → text_delta × 2 → content_block_stop → message_stop
		capturedHandler!({ session_id: "sess_test_001", event: { type: "message_start", session_id: "sess_test_001", model: "deepseek-v4-flash", max_tokens: 16384 } });
		capturedHandler!({ session_id: "sess_test_001", event: { type: "content_block_start", index: 0, block_type: "text" } });
		capturedHandler!({ session_id: "sess_test_001", event: { type: "text_delta", index: 0, text: "hello " } });
		capturedHandler!({ session_id: "sess_test_001", event: { type: "text_delta", index: 0, text: "world" } });
		capturedHandler!({ session_id: "sess_test_001", event: { type: "content_block_stop", index: 0 } });
		capturedHandler!({ session_id: "sess_test_001", event: { type: "message_stop" } });

		// 给 microtask 一个 tick
		await new Promise((r) => setTimeout(r, 0));

		const msgs = sessionStore.getMessages("sess_test_001");
		const assistant = msgs.find((m) => m.role === "assistant");
		expect(assistant?.content).toBe("hello world");
		expect(assistant?.streaming).toBe(false);
	});

	it("send_with_plan_keyword_skips_streaming_and_calls_plan_create: 关键词触发 plan", async () => {
		await sessionStore.init();
		createPlanMock.mockResolvedValue({
			id: "plan_jwt_001",
			session_id: "sess_test_001",
			name: "JWT 鉴权",
			status: "running",
			started_at: new Date().toISOString(),
			ended_at: null,
		});

		await chatStore.send("sess_test_001", "请实现 JWT 鉴权");

		// 不应调 sendMessage
		expect(sendMessageMock).not.toHaveBeenCalled();
		// createPlan 必须带 session_id (回归: 之前用 sid 字段导致后端报 missing field)
		expect(createPlanMock).toHaveBeenCalledWith(
			expect.objectContaining({ session_id: "sess_test_001" }),
		);
		// 应追加一个 user + 1 个 assistant (带 plan_ref) 消息
		const msgs = sessionStore.getMessages("sess_test_001");
		expect(msgs).toHaveLength(2);
		expect(msgs[1]?.role).toBe("assistant");
		expect(msgs[1]?.plan_ref).toBe("plan_jwt_001");
	});

	it("send_failure_marks_error_message_locally: invoke 失败", async () => {
		await sessionStore.init();
		sendMessageMock.mockRejectedValueOnce(new Error("backend offline"));

		await chatStore.send("sess_test_001", "hi");

		const assistant = lastAssistant("sess_test_001");
		expect(assistant).toBeUndefined();
		const msgs = sessionStore.getMessages("sess_test_001");
		const lastAsst = msgs.find((m) => m.role === "assistant");
		expect(lastAsst?.content).toContain("[错误]");
		expect(lastAsst?.streaming).toBe(false);
	});

	it("sendToSubSession_when_no_sub_returns_silently: sub 不存在弹 toast", async () => {
		await chatStore.sendToSubSession("sub_nonexistent", "test");
		// 没 panic, uiStore 推了 warn toast
		expect(uiStore.toasts.length).toBeGreaterThan(0);
		expect(uiStore.toasts[0]?.kind).toBe("warn");
		// 不调 sendMessage 也不调 sendMessageToSubSession
		expect(sendMessageMock).not.toHaveBeenCalled();
		expect(sendMessageToSubSessionMock).not.toHaveBeenCalled();
	});

	it("sendToSubSession_when_sub_exists_routes_to_sendMessageToSubSession: 走真后端 invoke (4a-2 P0-2)", async () => {
		await sessionStore.init();
		sendMessageToSubSessionMock.mockResolvedValue({
			session_id: "sess_test_001",
			status: "streaming",
		});
		// 临时给 subSessionStore 加一个 sub
		vi.spyOn(subSessionStore, "get").mockReturnValueOnce({
			id: "sub_test",
			plan_id: "plan_test",
			plan_task_id: "task_1",
			parent_session_id: "sess_test_001",
			role: "coder",
			status: "Done",
			messages: [],
			output: null,
			started_at: new Date().toISOString(),
			ended_at: null,
		});

		await chatStore.sendToSubSession("sub_test", "追问一下");

		// 1. 解析 sub → parent_sid, 调 sendMessageToSubSession(parent_sid, ...)
		expect(sendMessageToSubSessionMock).toHaveBeenCalledTimes(1);
		expect(sendMessageToSubSessionMock).toHaveBeenCalledWith("sess_test_001", {
			messages: [{ role: "user", content: "追问一下" }],
		});
		// 2. 不应再调 sendMessage (避免走普通 session 路径)
		expect(sendMessageMock).not.toHaveBeenCalled();
		// 3. 追加 user + assistant 占位消息, 都标记 sub_session_id
		const msgs = sessionStore.getMessages("sess_test_001");
		expect(msgs).toHaveLength(2);
		expect(msgs[0]?.role).toBe("user");
		expect(msgs[0]?.sub_session_id).toBe("sub_test");
		expect(msgs[0]?.kind).toBe("followup");
		expect(msgs[1]?.role).toBe("assistant");
		expect(msgs[1]?.sub_session_id).toBe("sub_test");
		expect(msgs[1]?.streaming).toBe(true);
	});

	it("sendToSubSession_failure_marks_error_message: invoke 失败", async () => {
		await sessionStore.init();
		sendMessageToSubSessionMock.mockRejectedValueOnce(new Error("session not found"));
		vi.spyOn(subSessionStore, "get").mockReturnValueOnce({
			id: "sub_test",
			plan_id: "plan_test",
			plan_task_id: "task_1",
			parent_session_id: "sess_test_001",
			role: "coder",
			status: "Done",
			messages: [],
			output: null,
			started_at: new Date().toISOString(),
			ended_at: null,
		});

		await chatStore.sendToSubSession("sub_test", "追问");

		const msgs = sessionStore.getMessages("sess_test_001");
		const lastAsst = msgs.find((m) => m.role === "assistant");
		expect(lastAsst?.content).toContain("[错误]");
		expect(lastAsst?.streaming).toBe(false);
	});
});

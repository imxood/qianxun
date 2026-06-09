// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — Runtime IPC 客户端 (Stage 4a 集成骨架 #4)
//
// 封装 qianxun-runtime 5 个 RuntimeApi 命令的 invoke 调用, 跟 $lib/ipc/bridge.ts
// 同模式 (isTauri() 降级 + web dev fallback). 上层 store 不感知运行环境.
//
// 5 个命令 (1:1 对应 RuntimeApi trait 5 个方法):
//   - listSessions   list_sessions   → 列所有 session (filter 选 active/paused/stored/all)
//   - sendMessage    send_message    → 推 user 消息 + 起 agent loop (返 SendResponse, 流走 'session_event' 事件)
//   - createPlan     create_plan     → 在指定 session 上建 plan (Running 状态)
//   - cancelSession  cancel_session  → 取消正在跑的 session
//   - loadSession    load_session    → 加载 session 完整状态 (含 conversation snapshot)
//
// 不在本文件:
//   - 'session_event' 事件监听: 拆到 onSessionEvent() (单 listener 全局复用, 见 sub-task #4 设计)
//   - SseEvent → Message 状态机: 拆到 $lib/stores/chat-stream.ts
//   - 业务错误处理: 由 store 层包 RuntimeApiError, 弹 toast / 入队
//
// 关联:
//   - qianxun-runtime/src/api/trait_def.rs (后端 trait 定义)
//   - qianxun-runtime/src/api/types.rs (DTO 类型, snake_case JSON)
//   - qianxun-desktop/src-tauri/src/commands/runtime/* (Tauri thin adapter)
//   - docs/30_子项目规划/04b-tauri-runtime-integration.md §"数据流"
// ───────────────────────────────────────────────────────────────────────────

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn, type Event } from "@tauri-apps/api/event";
import { isTauri } from "./bridge";

// ─── DTO 类型 (跟 qianxun-runtime/src/api/types.rs 1:1) ─────────────────

/// Session 状态过滤.
export type SessionFilter = "active" | "paused" | "stored" | "all";

/// 单个 session 状态 (来自后端).
export type SessionStatus = "active" | "paused" | "stored";

/// list_sessions 单个元素.
export interface SessionInfo {
	id: string;
	model: string;
	status: SessionStatus;
	created_at: string;
	last_active_at: string;
	message_count: number;
	/// 2026-06-09 加: 工作目录根 (后端 SessionInfo.project_root).
	/// 前端 projectStore 用此字段去重 derive project 列表.
	project_root?: string;
}

/// create_session 请求. 跟后端 CreateSessionRequest 1:1 (snake_case JSON).
/// `model` 后端暂未使用, 跟 SendRequest.model 一致 (Stage 2 简化).
/// `project_root` 透传给 AgentLoopHost (工作目录关联).
export interface CreateSessionRequest {
	model?: string;
	project_root?: string;
}

/// Provider 配置 (跟后端 ProviderConfig 1:1).
/// 2026-06-09 加: 桌面端 Provider 设置 UI 写 config.json 用.
export interface ProviderConfig {
	api_key?: string;
	model?: string;
	base_url?: string;
	temperature?: number;
	max_tokens?: number;
}

/// update_active_provider 请求 (2026-06-09 加).
/// 行为: 后端写 ~/.qianxun/config.json, 不热替换. 调用方需提示用户重启 desktop.
export interface UpdateProviderRequest {
	active_provider: string;
	provider_config?: ProviderConfig;
}

/// list_sessions 响应.
export interface ListSessionsResponse {
	sessions: SessionInfo[];
	total: number;
	filter: string;
	active_in_memory: number;
	paused_in_memory: number;
}

/// send_message 单条消息.
export interface SendMessage {
	role: "user" | "assistant" | "system";
	content: string;
}

/// send_message 请求.
export interface SendRequest {
	messages: SendMessage[];
	model?: string;
}

/// send_message 响应.
export interface SendResponse {
	session_id: string;
	status: "streaming";
}

/// Plan 状态 (Phase D 收尾: 加 pending/failed 跟后端 5 态 1:1).
export type PlanStatus = "pending" | "running" | "done" | "failed" | "aborted";

/// 单个 task 规格 (跟 Svelte 端 PlanTaskSpec 字段对齐, 跟后端 PlanTaskSpec 1:1).
export interface PlanTaskSpec {
	id: string;
	title: string;
	prompt: string;
	assigned_to?: string;
	depends_on?: string[];
	timeout_ms?: number;
}

/// 单个 task 执行结果.
export interface PlanTaskResult {
	id: string;
	status: PlanStatus;
	output: string;
	error?: string | null;
	started_at?: string | null;
	ended_at?: string | null;
}

/// Plan contract.
export interface PlanContract {
	name: string;
	description?: string;
	tasks: PlanTaskSpec[];
	timeout_ms?: number;
}

/// create_plan 请求 (Phase D 收尾: tasks 字段).
export interface PlanInput {
	session_id: string;
	name: string;
	description?: string;
	timeout_ms?: number;
	tasks?: PlanTaskSpec[];
}

/// create_plan / list_plans 响应元素 (Phase D 收尾: 加 task_results / contract).
export interface PlanInfo {
	id: string;
	session_id: string;
	name: string;
	status: PlanStatus;
	started_at: string;
	ended_at: string | null;
	task_results?: PlanTaskResult[];
	contract?: PlanContract;
}

/// load_session 响应.
export interface SessionState {
	session_id: string;
	exists_in_memory: boolean;
	status: SessionStatus;
	conversation_json: string | null;
	message_count: number;
}

// ─── session_event payload (Tauri emit schema) ─────────────────────────

/// 后端 send_message 起 spawn task 后, 逐个 emit SseEvent 给前端.
/// payload = { session_id, event: SseEvent (snake_case 12 种类型, 跟 qianxun-runtime/src/sse.rs 1:1) }.
export interface SessionEventPayload {
	session_id: string;
	// 原始 SseEvent JSON, 跟 ipc/types/sse.ts 的 SseEvent union 1:1 (但用 snake_case)
	// 前端按 event.type 分发 (message_start / text_delta / message_stop / plan_update 等)
	event: SseEventFromBackend;
}

/// Tauri emit 用的 SseEvent (snake_case 字段, 跟后端 SseEvent serde tag 一致).
/// 12 种 type 跟 qianxun-runtime/src/sse.rs::SseEvent 1:1, 前端按 type 分发.
export type SseEventFromBackend =
	| { type: "message_start"; session_id: string; model: string; max_tokens: number }
	| { type: "content_block_start"; index: number; block_type: string }
	| { type: "text_delta"; index: number; text: string }
	| { type: "thinking_delta"; index: number; text: string }
	| {
			type: "tool_use_delta";
			index: number;
			id: string;
			name: string;
			arguments_json: string;
	  }
	| {
			type: "tool_use_complete";
			index: number;
			id: string;
			name: string;
			arguments: Record<string, unknown>;
	  }
	| {
			type: "tool_result";
			tool_use_id: string;
			content: string;
			is_error: boolean;
			elapsed_ms: number;
	  }
	| { type: "content_block_stop"; index: number }
	| {
			type: "usage";
			input_tokens: number;
			output_tokens: number;
			cache_creation_input_tokens: number;
			cache_read_input_tokens: number;
	  }
	| { type: "message_delta"; stop_reason: string }
	| { type: "message_stop" }
	| { type: "error"; code: string; message: string };

/// Tauri emit 事件名 (跟 src-tauri/src/commands/runtime/send.rs::SESSION_EVENT 一致).
export const SESSION_EVENT_NAME = "session_event";

// ─── RuntimeApiError (前端包装, 跟后端 RuntimeApiError 4 类 1:1) ────────

/// RuntimeApi 调用错误. 后端 RuntimeApiError 是 thiserror enum, Tauri layer map 成 String.
/// 前端重新 parse 出 code (前 3 字符) + message, 跟后端 NotFound/InvalidRequest/Conflict/Internal/Unavailable 对齐.
export class RuntimeApiError extends Error {
	constructor(
		public code: "NotFound" | "InvalidRequest" | "Conflict" | "Internal" | "Unavailable",
		message: string,
	) {
		super(`[${code}] ${message}`);
		this.name = "RuntimeApiError";
	}

	/// 从 Tauri 返回的 String 错误还原 RuntimeApiError.
	/// 后端 format 是 "not found: xxx" / "invalid request: xxx" / "conflict: xxx" / "internal error: xxx" / "unavailable: xxx".
	static parse(raw: string): RuntimeApiError {
		const lower = raw.toLowerCase();
		if (lower.startsWith("not found:")) return new RuntimeApiError("NotFound", raw);
		if (lower.startsWith("invalid request:"))
			return new RuntimeApiError("InvalidRequest", raw);
		if (lower.startsWith("conflict:"))
			return new RuntimeApiError("Conflict", raw);
		if (lower.startsWith("internal error:"))
			return new RuntimeApiError("Internal", raw);
		if (lower.startsWith("unavailable:"))
			return new RuntimeApiError("Unavailable", raw);
		return new RuntimeApiError("Internal", raw);
	}
}

// ─── 5 个 invoke 薄壳 ──────────────────────────────────────────────────

/// 列所有 session (filter 选 "active" / "paused" / "stored" / "all", 默认 "all").
/// Tauri 模式: invoke<list_sessions, ListSessionsResponse>('list_sessions', { filter }).
/// Web fallback: 返空 list (让 UI 显空状态, 不假装有数据).
export async function listSessions(filter: SessionFilter = "all"): Promise<ListSessionsResponse> {
	if (!isTauri()) {
		return webFallbackListSessions(filter);
	}
	try {
		return await invoke<ListSessionsResponse>("list_sessions", { filter });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 创建新 session. 后端生成 sess_ 格式 ID, 返 SessionInfo.
///
/// Tauri 模式: invoke<create_session, SessionInfo>('create_session', { request }).
/// Web fallback: 客户端造 ID (web 模式无后端, 跟旧 mock 阶段一致).
export async function createSession(request: CreateSessionRequest): Promise<SessionInfo> {
	if (!isTauri()) {
		return webFallbackCreateSession(request);
	}
	try {
		return await invoke<SessionInfo>("create_session", { request });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 推 user 消息 + 起 agent loop, 立即返 SendResponse. 流走 'session_event' 事件 (用 onSessionEvent listen).
/// Tauri 模式: invoke<send_message, SendResponse>('send_message', { sessionId, request }).
/// Web fallback: 模拟 'streaming' 状态 (前端 UI 仍能跑, 但不会真发流).
export async function sendMessage(
	sessionId: string,
	request: SendRequest,
): Promise<SendResponse> {
	if (!isTauri()) {
		return { session_id: sessionId, status: "streaming" };
	}
	try {
		return await invoke<SendResponse>("send_message", { sessionId, request });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 在指定 session 上建 plan (Running 状态).
/// Tauri 模式: invoke<create_plan, PlanInfo>('create_plan', { input }).
/// Web fallback: 返 mock PlanInfo (id 用 'mock_' 前缀, 让 UI 能识别).
export async function createPlan(input: PlanInput): Promise<PlanInfo> {
	if (!isTauri()) {
		return webFallbackCreatePlan(input);
	}
	try {
		return await invoke<PlanInfo>("create_plan", { input });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 取消正在跑的 session (软取消, agent_host 设 paused flag).
/// Tauri 模式: invoke<cancel_session, ()>('cancel_session', { sessionId }).
/// Web fallback: noop (mock 阶段没有可取消的 session).
export async function cancelSession(sessionId: string): Promise<void> {
	if (!isTauri()) {
		return;
	}
	try {
		await invoke<void>("cancel_session", { sessionId });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 加载 session 完整状态 (含 conversation snapshot).
/// Tauri 模式: invoke<load_session, SessionState>('load_session', { sessionId }).
/// Web fallback: 返 Stored 状态 + 空 conversation.
export async function loadSession(sessionId: string): Promise<SessionState> {
	if (!isTauri()) {
		return webFallbackLoadSession(sessionId);
	}
	try {
		return await invoke<SessionState>("load_session", { sessionId });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 删除 session (内存 + 持久化). 释放 max_sessions 槽位.
export async function deleteSession(sessionId: string): Promise<void> {
	if (!isTauri()) {
		return;
	}
	try {
		await invoke<void>("delete_session", { sessionId });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 暂停 session (拒绝新 send_message, 返 InvalidRequest).
export async function pauseSession(sessionId: string): Promise<void> {
	if (!isTauri()) {
		return;
	}
	try {
		await invoke<void>("pause_session", { sessionId });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 解除暂停.
export async function resumeSession(sessionId: string): Promise<void> {
	if (!isTauri()) {
		return;
	}
	try {
		await invoke<void>("resume_session", { sessionId });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 更新 active provider + 可选 provider config. 写 ~/.qianxun/config.json.
/// 调用方需提示用户重启 desktop (后端不热替换).
export async function updateActiveProvider(
	request: UpdateProviderRequest,
): Promise<void> {
	if (!isTauri()) {
		// Web 模式 noop
		return;
	}
	try {
		await invoke<void>("update_active_provider", { request });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

/// 取消 plan (Phase D 收尾: 后端 RuntimeApi 加 cancel_plan).
/// Tauri 模式: invoke<cancel_plan, ()>('cancel_plan', { planId }).
/// Web fallback: noop.
export async function cancelPlan(planId: string): Promise<void> {
	if (!isTauri()) {
		return;
	}
	try {
		await invoke<void>("cancel_plan", { planId });
	} catch (e) {
		throw RuntimeApiError.parse(String(e));
	}
}

// ─── session_event 监听 (全局唯一 listener) ─────────────────────────────

/// Listen 'session_event' Tauri 事件. caller 自己过滤 session_id.
/// Tauri 模式: 真 listen.
/// Web fallback: noop unlisten (web 模式无后端 emit).
export async function onSessionEvent(
	handler: (payload: SessionEventPayload) => void,
): Promise<UnlistenFn> {
	if (!isTauri()) {
		return () => {};
	}
	return await listen<SessionEventPayload>(SESSION_EVENT_NAME, (e: Event<SessionEventPayload>) =>
		handler(e.payload),
	);
}

// ─── Web fallback 内部 helper ───────────────────────────────────────────

function webFallbackListSessions(filter: SessionFilter): ListSessionsResponse {
	return {
		sessions: [],
		total: 0,
		filter,
		active_in_memory: 0,
		paused_in_memory: 0,
	};
}

function webFallbackCreatePlan(input: PlanInput): PlanInfo {
	const now = new Date().toISOString();
	return {
		id: `mock_plan_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
		session_id: input.session_id,
		name: input.name,
		status: "running",
		started_at: now,
		ended_at: null,
		task_results: (input.tasks ?? []).map((t) => ({
			id: t.id,
			status: "pending",
			output: "",
		})),
		contract: {
			name: input.name,
			description: input.description ?? "",
			tasks: input.tasks ?? [],
			timeout_ms: input.timeout_ms ?? 0,
		},
	};
}

function webFallbackLoadSession(sessionId: string): SessionState {
	return {
		session_id: sessionId,
		exists_in_memory: false,
		status: "stored",
		conversation_json: null,
		message_count: 0,
	};
}

/// Web fallback: 客户端造 ID (跟旧 mock 阶段一致, 让 UI 仍能跑).
function webFallbackCreateSession(request: CreateSessionRequest): SessionInfo {
	const now = new Date().toISOString();
	return {
		id: `sess_${now.replace(/[-:T.Z]/g, "").slice(0, 17)}_${Math.random().toString(36).slice(2, 8)}`,
		model: request.model ?? "deepseek-v4-flash",
		status: "active",
		created_at: now,
		last_active_at: now,
		message_count: 0,
	};
}

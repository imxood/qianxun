// Stage 9c — Chat 视图类型定义 (Web Console)
// 跟 docs/30_子项目规划/_shared-contract.md §3.2 SSE 事件 schema 一致
// 跟 docs/30_子项目规划/01b-daemon-web-console.md §10 Chat 视图 一致
//
// SSE 事件 JSON 格式: `data: <json>\n\n`, `type` 字段 (内部 tag) 分发 12 种事件.
// 这里只定义 Web Console 用到的子集 (跟 daemon 端 sse.rs 严格对齐).
//
// 重要: 跟 Tauri 桌面端 ipc.ts SseEvent 略有差异 (daemon 端字段是必填的).
// Web Console 的类型与 daemon 一致, 跟 Tauri 不互通 (走不同 store, 没问题).
//
// ContentBlock 是 1 个 message 内的 1 个块 (text / thinking / tool_use / tool_result).
// Message 由 ContentBlock[] 构成 (跟 daemon 的 Conversation 模型一致).

// ─── ContentBlock (跟 qianxun-core 完全对齐) ────────────────────────────────

export type ContentBlockType = 'text' | 'thinking' | 'tool_use' | 'tool_result' | 'image';

export interface TextBlock {
	type: 'text';
	text: string;
}

export interface ThinkingBlock {
	type: 'thinking';
	text: string;
}

export interface ToolUseBlock {
	type: 'tool_use';
	id: string;
	name: string;
	/** 解析后的参数对象 (尽力 JSON.parse tool_use_delta.arguments_json 累积) */
	input: Record<string, unknown>;
}

export interface ToolResultBlock {
	type: 'tool_result';
	tool_use_id: string;
	content: string;
	is_error: boolean;
	elapsed_ms: number;
}

export interface ImageBlock {
	type: 'image';
	/** 暂时不实现, 占位 */
	src?: string;
}

export type ContentBlock = TextBlock | ThinkingBlock | ToolUseBlock | ToolResultBlock | ImageBlock;

// ─── Message (Stage 9c: 简化版, 跟 Tauri 的 stage 3 一样) ─────────────────────

export type MessageRole = 'user' | 'assistant' | 'system';

export interface Message {
	id: string;
	session_id: string;
	role: MessageRole;
	/** 累积的 content blocks. user message 总是单 text block. */
	content: ContentBlock[];
	model?: string;
	usage?: { input: number; output: number };
	stop_reason?: string;
	created_at: string;
	/** 客户端本地状态 — 不会序列化到 daemon */
	streaming?: boolean;
	error?: string;
}

// ─── ChatSession 摘要 (跟 ChatSessionSummary 一致) ──────────────────────────

export type SessionStatus = 'active' | 'paused' | 'completed' | 'cancelled';

export interface ChatSession {
	id: string;
	model: string;
	created_at: string;
	last_active: string;
	message_count: number;
	status: SessionStatus;
	token_usage: { input: number; output: number; total: number };
}

export interface ChatSessionList {
	sessions: ChatSession[];
	total: number;
}

export interface ChatSessionCreated {
	session_id: string;
}

// ─── SSE 事件 (12 种, 跟 daemon sse.rs SseEvent 严格对齐) ────────────────────

export type SseBlockType = 'text' | 'thinking' | 'tool_use';

/**
 * 12 个 SSE 事件的 discriminated union.
 * JSON 格式: `{"type": "message_start", ...}`, 通过 `type` 字段分发.
 */
export type SseEvent =
	| {
			type: 'message_start';
			session_id: string;
			model: string;
			max_tokens: number;
	  }
	| { type: 'content_block_start'; index: number; block_type: SseBlockType }
	| { type: 'text_delta'; index: number; text: string }
	| { type: 'thinking_delta'; index: number; text: string }
	| {
			type: 'tool_use_delta';
			index: number;
			id: string;
			name: string;
			arguments_json: string;
	  }
	| {
			type: 'tool_use_complete';
			index: number;
			id: string;
			name: string;
			arguments: Record<string, unknown>;
	  }
	| {
			type: 'tool_result';
			tool_use_id: string;
			content: string;
			is_error: boolean;
			elapsed_ms: number;
	  }
	| { type: 'content_block_stop'; index: number }
	| {
			type: 'usage';
			input_tokens: number;
			output_tokens: number;
			cache_creation_input_tokens: number;
			cache_read_input_tokens: number;
	  }
	| { type: 'message_delta'; stop_reason: string }
	| { type: 'message_stop' }
	| { type: 'error'; code: string; message: string };

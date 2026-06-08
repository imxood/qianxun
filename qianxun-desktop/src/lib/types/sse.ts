// qianxun-desktop/src/lib/types/sse.ts
// 跟 docs/daemon-design.md v1.0 §3.5 + _shared-contract v2 §3.2 一致
// 12 事件 type union, 用 discriminator 区分

export type SseEvent =
	| { event: 'message_start'; data: { session_id: string; message_id: string } }
	| { event: 'text'; data: { text: string } }
	| { event: 'thinking'; data: { text: string } }
	| {
			event: 'tool_call';
			data: {
				id: string;
				name: string;
				arguments: Record<string, unknown>;
				plan_ref: string | null;
			};
	  }
	| {
			event: 'tool_result';
			data: {
				id: string;
				name: string;
				content: string;
				is_error: boolean;
				elapsed_ms: number;
			};
	  }
	| {
			event: 'plan_update';
			data: {
				plan_id: string;
				status: 'pending' | 'running' | 'done' | 'failed' | 'aborted';
				task_id: string | null;
				progress: { done: number; total: number };
			};
	  }
	| {
			event: 'sub_session_event';
			data: {
				sub_session_id: string;
				event: SseEvent;
			};
	  }
	| {
			event: 'experience_suggest';
			data: {
				project_id: string;
				items: Array<{
					content: string;
					source_session_id: string | null;
					source_plan_id: string | null;
				}>;
			};
	  }
	| { event: 'status'; data: { message: string; level: 'info' | 'warn' } }
	| {
			event: 'error';
			data: {
				code: 'rate_limit' | 'auth' | 'internal' | 'cancelled';
				message: string;
			};
	  }
	| {
			event: 'turn_finished';
			data: {
				reason: 'end_turn' | 'tool_use' | 'max_tokens' | 'stop';
				usage: { input: number; output: number; cost_usd: number };
			};
	  }
	| { event: 'message_stop'; data: Record<string, never> };

export type SseEventName = SseEvent['event'];

// 4 种 error code 处理建议 (跟 _shared-contract v2 §3.2 一致)
export const ERROR_CODE_HANDLERS: Record<string, string> = {
	rate_limit: '请求过快, 请稍后重试',
	auth: 'API Key 配置错误, 请检查设置',
	internal: '出错了, 请重试',
	cancelled: '已取消',
};

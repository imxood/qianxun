// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — Chat 流式 SseEvent 状态机 (Stage 4a sub-task #4)
//
// 镜像 qianxun-runtime/src/sse.rs::SseEventBuilder 的前端版本.
// 把后端 12 种 SseEvent 逐步 apply 到 MessageStreamState, 触发 onUpdate 通知 UI.
//
// 业务 (跟后端 builder 1:1, 只看前端需要的):
//   - message_start      → 标记 message 开始 (messageId 由前端预生成)
//   - content_block_start → 切 currentBlock (text / thinking / tool_use)
//   - text_delta         → 累加 content
//   - thinking_delta     → 累加 thinking
//   - tool_use_complete  → 推 toolCalls[] (id / name / arguments)
//   - tool_result        → 找对应 toolCall 加 result (content / is_error / elapsed_ms)
//   - content_block_stop → currentBlock = 'none'
//   - message_stop       → finished = true (收尾)
//   - error              → 追加错误信息到 content, finished = true
//
// 不在本文件:
//   - session_event 全局监听: 拆到 chat.svelte.ts (一个 listener 复用)
//   - 跟 store 集成: 拆到 chat.svelte.ts
//
// 关联:
//   - qianxun-runtime/src/sse.rs::SseEvent (12 variants, 1:1 镜像)
//   - $lib/ipc/runtime.ts::SseEventFromBackend (snake_case 字段名)
//   - qianxun-runtime/src/sse.rs::SseEventBuilder (后端 builder, 状态机 1:1)
// ───────────────────────────────────────────────────────────────────────────

import type { SseEventFromBackend } from '$lib/ipc/runtime';

export type BlockKind = 'none' | 'text' | 'thinking' | 'tool_use';

export interface ToolCallState {
	id: string;
	name: string;
	arguments: Record<string, unknown>;
	result?: {
		content: string;
		isError: boolean;
		elapsedMs: number;
	};
}

/// 单条 streaming message 的状态. 每条 message 独立 state.
/// 跟 entity.Message 字段对齐 (除 tool_calls 因为 tool 还没真跑, 留占位).
export interface MessageStreamState {
	/// 前端预生成的 message id (跟 sessionStore.appendMessage 用的 id 一致).
	messageId: string;
	/// 当前 streaming 的 text content (text_delta 累加).
	content: string;
	/// 当前 streaming 的 thinking content (thinking_delta 累加, 暂 UI 不显示, 留接口).
	thinking: string;
	/// tool calls 数组 (tool_use_complete 推, tool_result 补 result).
	toolCalls: ToolCallState[];
	/// 当前 block 类型 (content_block_start 切, content_block_stop 重置).
	currentBlock: BlockKind;
	/// 收尾标记 (message_stop 或 error 后为 true).
	finished: boolean;
	/// 错误信息 (event.type === 'error' 时填充).
	error?: { code: string; message: string };
	/// 每次 state 变都触发 (UI 重渲染).
	onUpdate: () => void;
}

/// 构造新的 stream state. messageId 由调用方预生成.
/// onUpdate 通常是 () => 触发 Svelte 响应式 (因为 Message 是 $state, 直接修改字段就行).
export function newStreamState(messageId: string, onUpdate: () => void): MessageStreamState {
	return {
		messageId,
		content: '',
		thinking: '',
		toolCalls: [],
		currentBlock: 'none',
		finished: false,
		onUpdate,
	};
}

/// 把 1 个 SseEvent apply 到 stream state. 纯函数 (但通过 onUpdate 触发副作用).
export function applyEvent(state: MessageStreamState, event: SseEventFromBackend): void {
	switch (event.type) {
		case 'message_start':
			// 后端第一件事就是 message_start. 前端不重置 state, content 已是 ''.
			// 实际 messageId 是前端预生成的, 后端的 session_id 仅用来关联.
			break;

		case 'content_block_start':
			if (event.block_type === 'text') state.currentBlock = 'text';
			else if (event.block_type === 'thinking') state.currentBlock = 'thinking';
			else if (event.block_type === 'tool_use') state.currentBlock = 'tool_use';
			break;

		case 'text_delta':
			if (state.currentBlock === 'text') {
				state.content += event.text;
				state.onUpdate();
			}
			break;

		case 'thinking_delta':
			if (state.currentBlock === 'thinking') {
				state.thinking += event.text;
				state.onUpdate();
			}
			break;

		case 'tool_use_complete': {
			state.toolCalls.push({
				id: event.id,
				name: event.name,
				arguments: event.arguments,
			});
			state.onUpdate();
			break;
		}

		case 'tool_result': {
			const tc = state.toolCalls.find((t) => t.id === event.tool_use_id);
			if (tc) {
				tc.result = {
					content: event.content,
					isError: event.is_error,
					elapsedMs: event.elapsed_ms,
				};
				state.onUpdate();
			}
			break;
		}

		case 'content_block_stop':
			state.currentBlock = 'none';
			break;

		case 'usage':
			// token 计数, 后续 sub-task 接 Settings 面板的 budget 跟踪.
			// 当前 sub-task 范围: 忽略.
			break;

		case 'message_delta':
			// stop_reason 提示, 后续 sub-task 接 status 派生.
			// 当前 sub-task 范围: 忽略.
			break;

		case 'message_stop':
			state.finished = true;
			state.onUpdate();
			break;

		case 'error':
			state.error = { code: event.code, message: event.message };
			state.content += `\n\n[错误: ${event.code}] ${event.message}`;
			state.finished = true;
			state.onUpdate();
			break;

		case 'tool_use_delta':
			// 批式 tool call (qianxun-runtime 当前不产生, 跟 _shared-contract §3.2 一致).
			// 增量 delta 暂不处理, 走 tool_use_complete.
			break;
	}
}

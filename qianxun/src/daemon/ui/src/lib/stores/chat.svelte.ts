// ───────────────────────────────────────────────────────────────────────────
// 千寻 Web Console — ChatStore (Stage 9c)
// 跟 docs/30_子项目规划/01b-daemon-web-console.md §10 Chat 视图 一致
// 跟 docs/30_子项目规划/_shared-contract.md §3.2 SSE 事件 schema 一致
//
// 范围:
//   - activeSession: 当前选中的 chat session
//   - sessions: session 列表 (last_active desc)
//   - messages: 当前 session 的 messages
//   - streaming: 是否在流式生成中
//   - currentAbort: AbortController, 用于 cancel
//   - usage: token 累计
//   - lastError: 错误信息
//
// 跟 Tauri 桌面端 session.svelte.ts 类似, 但简化:
//   - 不实现 offlineQueue (Web Console 是远程调试, 假设 daemon 可达)
//   - 不实现 multi-session runtime (一个 store 一个 active session)
//
// 渲染模型: messages 是 Message[] (role 在 Message 层级), 跟 Tauri Stage 5
// 对齐 (Tauri Stage 3 是 ContentBlock[], 现在升级到 Message[]).
// ───────────────────────────────────────────────────────────────────────────

import type { ContentBlock, Message, SseEvent, SseBlockType } from '$lib/types/chat';
import { fetchPromptStream } from '$lib/api/chat';
import { parseSseStream } from '$lib/sse/parser';
import { authStore } from '$lib/stores/auth.svelte';

function uuid(): string {
	if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
		return crypto.randomUUID();
	}
	return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
		const r = (Math.random() * 16) | 0;
		const v = c === 'x' ? r : (r & 0x3) | 0x8;
		return v.toString(16);
	});
}

function nowIso(): string {
	return new Date().toISOString();
}

interface ChatRuntimeState {
	activeSessionId: string | null;
	streaming: boolean;
	currentAbort: AbortController | null;
	usage: { input: number; output: number } | null;
	stopReason: string | null;
	lastError: string | null;
	model: string;
	/// SSE event.index → messages[] array index 的映射 (SSE 协议允许 index 任意顺序)
	indexMap: Map<number, number>;
	/// tool_use 流式参数累积 (event.index → 累计的 arguments_json 字符串)
	argsAcc: Map<number, string>;
}

class ChatStore {
	// ─── 状态 ────────────────────────────────────────────────────────────────

	/// 全部 session (按 last_active desc, 调用方负责排序)
	sessions = $state<import('$lib/types/chat').ChatSession[]>([]);
	/// 当前 active session 的 messages (一维, 完整对话历史)
	messages = $state<Message[]>([]);
	/// 加载状态
	loading = $state(false);
	loadError = $state<string | null>(null);

	// ─── 内部 runtime (非响应式 + 部分响应式) ──────────────────────────────

	#runtime: ChatRuntimeState = {
		activeSessionId: null,
		streaming: false,
		currentAbort: null,
		usage: null,
		stopReason: null,
		lastError: null,
		model: 'MiniMax-M3',
		indexMap: new Map(),
		argsAcc: new Map()
	};

	// ─── 派生 getter ────────────────────────────────────────────────────────

	get isStreaming(): boolean {
		return this.#runtime.streaming;
	}

	get currentAbort(): AbortController | null {
		return this.#runtime.currentAbort;
	}

	get usage$(): { input: number; output: number } | null {
		return this.#runtime.usage;
	}

	get lastError(): string | null {
		return this.#runtime.lastError;
	}

	get stopReason(): string | null {
		return this.#runtime.stopReason;
	}

	get model(): string {
		return this.#runtime.model;
	}

	get activeSessionId(): string | null {
		return this.#runtime.activeSessionId;
	}

	// ─── Session 列表 / 切换 ────────────────────────────────────────────────

	async loadSessions(): Promise<void> {
		this.loading = true;
		this.loadError = null;
		try {
			const { listChatSessionsAll } = await import('$lib/api/chat');
			const list = await listChatSessionsAll();
			// 按 last_active desc 排序
			this.sessions = [...list].sort((a, b) =>
				b.last_active.localeCompare(a.last_active)
			);
		} catch (e) {
			this.loadError = e instanceof Error ? e.message : String(e);
		} finally {
			this.loading = false;
		}
	}

	selectSession(id: string | null): void {
		// 切换时如有正在流式, 取消
		if (this.#runtime.streaming) {
			this.cancel();
		}
		this.#runtime.activeSessionId = id;
		this.messages = [];
		this.#resetRuntime();
		// 注: 消息历史从 daemon 拉 (Stage 9c 简化: 假设 active 后 server-side 续聊, 不预拉历史)
		// 完整消息历史拉取留 Stage 9d
	}

	/// 删除 session
	async deleteSession(id: string): Promise<void> {
		const { deleteChatSessionById } = await import('$lib/api/chat');
		await deleteChatSessionById(id);
		this.sessions = this.sessions.filter((s) => s.id !== id);
		if (this.#runtime.activeSessionId === id) {
			this.selectSession(null);
		}
	}

	/// 创建新 session
	async createNewSession(): Promise<string | null> {
		const { createChatSession } = await import('$lib/api/chat');
		const r = await createChatSession();
		// 插入到 sessions 顶部
		const newSession: import('$lib/types/chat').ChatSession = {
			id: r.session_id,
			model: this.#runtime.model,
			created_at: nowIso(),
			last_active: nowIso(),
			message_count: 0,
			status: 'active',
			token_usage: { input: 0, output: 0, total: 0 }
		};
		this.sessions = [newSession, ...this.sessions];
		this.selectSession(r.session_id);
		return r.session_id;
	}

	// ─── 发送 prompt ────────────────────────────────────────────────────────

	async sendPrompt(text: string): Promise<void> {
		if (!text.trim()) return;
		if (this.#runtime.streaming) return;
		if (!this.#runtime.activeSessionId) return;
		if (!authStore.isAuthenticated) {
			this.#runtime.lastError = '未配置 JWT token (在顶栏设置)';
			return;
		}

		const sessionId = this.#runtime.activeSessionId;
		const userMessage: Message = {
			id: uuid(),
			session_id: sessionId,
			role: 'user',
			content: [{ type: 'text', text }],
			created_at: nowIso()
		};
		// 推 user message + 创建空的 assistant message 占位
		const assistantMessage: Message = {
			id: uuid(),
			session_id: sessionId,
			role: 'assistant',
			content: [],
			created_at: nowIso(),
			streaming: true
		};
		this.messages = [...this.messages, userMessage, assistantMessage];
		this.#resetRuntime();
		this.#runtime.streaming = true;
		this.#runtime.currentAbort = new AbortController();

		try {
			const response = await fetchPromptStream(
				sessionId,
				text,
				this.#runtime.currentAbort.signal
			);
			if (!response.ok) {
				const errText = await response.text().catch(() => '');
				throw new Error(`HTTP ${response.status}: ${errText || response.statusText}`);
			}
			if (!response.body) {
				throw new Error('Empty response body');
			}

			// 流式消费 + 逐事件处理
			for await (const event of parseSseStream(response.body)) {
				this.handleEvent(event);
			}
		} catch (e) {
			if ((e as Error).name === 'AbortError') {
				this.#runtime.lastError = '已取消';
			} else {
				this.#runtime.lastError = (e as Error).message || String(e);
			}
		} finally {
			this.#runtime.streaming = false;
			this.#runtime.currentAbort = null;
			// 标记 assistant message 不再 streaming
			const last = this.messages[this.messages.length - 1];
			if (last && last.role === 'assistant') {
				last.streaming = false;
				// 触发 Svelte 反应: 重新赋值数组
				this.messages = [...this.messages];
			}
		}
	}

	/// 取消当前流
	cancel(): void {
		this.#runtime.currentAbort?.abort();
	}

	// ─── SSE 事件处理 ──────────────────────────────────────────────────────

	handleEvent(event: SseEvent): void {
		const last = this.messages[this.messages.length - 1];
		if (!last || last.role !== 'assistant') return;
		const r = this.#runtime;

		switch (event.type) {
			case 'message_start': {
				r.indexMap.clear();
				r.argsAcc.clear();
				r.stopReason = null;
				r.usage = null;
				r.lastError = null;
				r.model = event.model;
				last.model = event.model;
				break;
			}

			case 'content_block_start': {
				let block: ContentBlock;
				if (event.block_type === 'text') {
					block = { type: 'text', text: '' };
				} else if (event.block_type === 'thinking') {
					block = { type: 'thinking', text: '' };
				} else {
					block = { type: 'tool_use', id: '', name: '', input: {} };
				}
				last.content = [...last.content, block];
				r.indexMap.set(event.index, last.content.length - 1);
				break;
			}

			case 'text_delta': {
				const arrIdx = r.indexMap.get(event.index);
				const block = arrIdx !== undefined ? last.content[arrIdx] : undefined;
				if (block && block.type === 'text') {
					block.text = (block.text ?? '') + event.text;
					// 触发反应
					last.content = [...last.content];
				}
				break;
			}

			case 'thinking_delta': {
				const arrIdx = r.indexMap.get(event.index);
				const block = arrIdx !== undefined ? last.content[arrIdx] : undefined;
				if (block && block.type === 'thinking') {
					block.text = (block.text ?? '') + event.text;
					last.content = [...last.content];
				}
				break;
			}

			case 'tool_use_delta': {
				const arrIdx = r.indexMap.get(event.index);
				const block = arrIdx !== undefined ? last.content[arrIdx] : undefined;
				if (block && block.type === 'tool_use') {
					if (event.id) block.id = event.id;
					if (event.name) block.name = event.name;
					const acc =
						(r.argsAcc.get(event.index) ?? '') + (event.arguments_json ?? '');
					r.argsAcc.set(event.index, acc);
					try {
						block.input = JSON.parse(acc);
					} catch {
						// 还没拼完整, 继续累积
					}
					last.content = [...last.content];
				}
				break;
			}

			case 'tool_use_complete': {
				const arrIdx = r.indexMap.get(event.index);
				const block = arrIdx !== undefined ? last.content[arrIdx] : undefined;
				if (block && block.type === 'tool_use') {
					block.id = event.id;
					block.name = event.name;
					block.input = event.arguments;
					last.content = [...last.content];
				}
				r.argsAcc.delete(event.index);
				break;
			}

			case 'tool_result': {
				last.content = [
					...last.content,
					{
						type: 'tool_result',
						tool_use_id: event.tool_use_id,
						content: event.content,
						is_error: event.is_error,
						elapsed_ms: event.elapsed_ms
					}
				];
				break;
			}

			case 'content_block_stop': {
				r.argsAcc.delete(event.index);
				break;
			}

			case 'usage': {
				r.usage = { input: event.input_tokens, output: event.output_tokens };
				last.usage = { input: event.input_tokens, output: event.output_tokens };
				break;
			}

			case 'message_delta': {
				r.stopReason = event.stop_reason;
				last.stop_reason = event.stop_reason;
				break;
			}

			case 'message_stop': {
				// 流自然结束
				break;
			}

			case 'error': {
				r.lastError = `[${event.code}] ${event.message}`;
				last.error = r.lastError;
				break;
			}
		}
	}

	// ─── 内部 helpers ──────────────────────────────────────────────────────

	#resetRuntime(): void {
		this.#runtime.usage = null;
		this.#runtime.stopReason = null;
		this.#runtime.lastError = null;
		this.#runtime.indexMap.clear();
		this.#runtime.argsAcc.clear();
	}

	/// 显式重置整个 store
	reset(): void {
		this.cancel();
		this.sessions = [];
		this.messages = [];
		this.#resetRuntime();
		this.#runtime.activeSessionId = null;
	}
}

export const chatStore = new ChatStore();
